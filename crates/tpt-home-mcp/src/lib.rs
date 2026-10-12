// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! MCP server for AI agents (Model Context Protocol, JSON-RPC 2.0).
//!
//! Two transports share one handler:
//! - **HTTP:** `POST /api/mcp`, behind the normal `/api` guard. The caller's
//!   bearer token (or session) decides what is visible and callable.
//! - **stdio:** `tpt-home mcp --token …`, for local agent clients that spawn
//!   the server as a child process.
//!
//! Tools are derived from enabled plugins:
//! - `plugins_list`: enabled plugins, their tables, pages and permissions.
//! - `plugin_call`: one request to any enabled plugin route.
//! - `<plugin>_<action>`: one per form page that declares an `action`, taking
//!   the form's fields as string arguments.
//!
//! Every call is checked against the caller's token scopes before it runs, and
//! then runs through the same `invoke_plugin` the HTTP route uses, so runtime
//! and permission rules are shared. Blueprint routes are not exposed: only
//! plugins are, which keeps the agent surface to what plugins declare.

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use tpt_home_core::auth::api_token::{ResolvedToken, TokenScopes, scope_allows};
use tpt_home_core::auth::user::AuthUser;
use tpt_home_core::db::DbPool;
use tpt_home_plugins::dispatch::invoke_plugin;
use tpt_home_plugins::registry::{self, LoadedPlugin};

/// MCP protocol revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const PARSE_ERROR: i64 = -32700;

/// Who is calling, and what their credential allows.
#[derive(Debug, Clone)]
pub struct Caller {
    pub user: AuthUser,
    pub scopes: Vec<String>,
}

impl From<ResolvedToken> for Caller {
    fn from(token: ResolvedToken) -> Self {
        Self {
            user: token.user,
            scopes: token.scopes,
        }
    }
}

/// What a tool does when called.
#[derive(Debug, Clone)]
enum Target {
    ListPlugins,
    PluginCall,
    /// A form action: `POST /api/<plugin>/<action>` with the arguments as body.
    FormAction {
        plugin: String,
        action: String,
    },
}

#[derive(Debug, Clone)]
struct Tool {
    name: String,
    description: String,
    input_schema: Value,
    target: Target,
}

fn tool_json(tool: &Tool) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input_schema,
    })
}

/// Enabled plugins this caller may reach at all.
fn visible_plugins(caller: &Caller) -> Vec<Arc<LoadedPlugin>> {
    registry::all()
        .into_iter()
        .filter(|(plugin, enabled)| {
            *enabled
                && scope_allows(
                    &caller.scopes,
                    &format!("/api/{}/", plugin.manifest.plugin.id),
                )
        })
        .map(|(plugin, _)| plugin)
        .collect()
}

/// Tools this caller can see. Plugins outside the token's scope contribute
/// nothing, so an agent is not told about routes it cannot use.
fn tools_for(caller: &Caller) -> Vec<Tool> {
    let mut tools = vec![
        Tool {
            name: "plugins_list".into(),
            description: "List enabled plugins with their tables, pages and permissions.".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            target: Target::ListPlugins,
        },
        Tool {
            name: "plugin_call".into(),
            description: "Call any route of an enabled plugin: GET, POST, PUT, PATCH or DELETE on /api/<plugin>/<path>.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "plugin": { "type": "string", "description": "Plugin id, e.g. calendar" },
                    "method": { "type": "string", "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"], "default": "GET" },
                    "path": { "type": "string", "description": "Path below the plugin, starting with /" },
                    "query": { "type": "string", "description": "Raw query string without '?'" },
                    "body": { "description": "JSON body (optional)" }
                },
                "required": ["plugin", "path"]
            }),
            target: Target::PluginCall,
        },
    ];

    for plugin in visible_plugins(caller) {
        let id = &plugin.manifest.plugin.id;
        for page in &plugin.manifest.pages {
            if page.kind != "form" {
                continue;
            }
            let Some(action) = page.action.as_deref() else {
                continue;
            };
            let action = action.trim_start_matches('/');
            // Tool names carry the plugin id and action, so keep them to the
            // identifier characters MCP clients accept.
            let name = format!("{id}_{}", action.replace('/', "_"));
            if name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let properties: serde_json::Map<String, Value> = page
                .fields
                .iter()
                .map(|f| (f.clone(), json!({ "type": "string" })))
                .collect();
            tools.push(Tool {
                name,
                description: format!("{} (plugin {id})", page.title),
                input_schema: json!({
                    "type": "object",
                    "properties": properties,
                    "required": page.fields,
                }),
                target: Target::FormAction {
                    plugin: id.clone(),
                    action: action.to_string(),
                },
            });
        }
    }
    tools
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Handle one JSON-RPC message or batch. Returns `None` when nothing should
/// be sent back (notifications, or an empty batch of notifications).
pub async fn handle(caller: &Caller, message: &Value) -> Option<Value> {
    if let Some(items) = message.as_array() {
        let mut replies = Vec::new();
        for item in items {
            if let Some(reply) = handle_one(caller, item).await {
                replies.push(reply);
            }
        }
        if replies.is_empty() {
            return None;
        }
        return Some(Value::Array(replies));
    }
    handle_one(caller, message).await
}

async fn handle_one(caller: &Caller, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(rpc_error(
            &Value::Null,
            PARSE_ERROR,
            "message must be a JSON object",
        ));
    };
    let method = object.get("method").and_then(Value::as_str).unwrap_or("");
    let params = object.get("params").cloned().unwrap_or(Value::Null);

    // A message without an id is a notification: act on it, never reply.
    let id = object.get("id").cloned()?;

    let reply = match method {
        "initialize" => rpc_result(
            &id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "tpt-homestack", "version": tpt_home_core::VERSION },
            }),
        ),
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(
            &id,
            json!({ "tools": tools_for(caller).iter().map(tool_json).collect::<Vec<_>>() }),
        ),
        "tools/call" => match call_tool(caller, &params).await {
            Ok(result) => rpc_result(&id, result),
            Err((code, message)) => rpc_error(&id, code, &message),
        },
        other => rpc_error(&id, METHOD_NOT_FOUND, &format!("unknown method '{other}'")),
    };
    Some(reply)
}

/// Run `tools/call`. Unknown tools are protocol errors; failures inside a
/// known tool come back as a result with `isError: true`, as MCP specifies.
async fn call_tool(caller: &Caller, params: &Value) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tool = tools_for(caller)
        .into_iter()
        .find(|t| t.name == name)
        .ok_or((INVALID_PARAMS, format!("unknown tool '{name}'")))?;

    let outcome = match tool.target {
        Target::ListPlugins => Ok(list_plugins(caller)),
        Target::PluginCall => plugin_call(caller, &arguments).await,
        Target::FormAction { plugin, action } => {
            let path = format!("/api/{plugin}/{action}");
            if !scope_allows(&caller.scopes, &path) {
                Err("token scope does not allow this plugin".to_string())
            } else {
                let response =
                    invoke_plugin(&plugin, "POST", &action, "", Some(arguments.to_string())).await;
                Ok(render(response).await)
            }
        }
    };

    Ok(match outcome {
        Ok(value) => value,
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    })
}

fn list_plugins(caller: &Caller) -> Value {
    let plugins: Vec<Value> = visible_plugins(caller)
        .iter()
        .map(|plugin| {
            let m = &plugin.manifest;
            json!({
                "id": m.plugin.id,
                "name": m.plugin.name,
                "version": m.plugin.version,
                "runtime": format!("{:?}", m.plugin.runtime).to_lowercase(),
                "tables": m.tables.iter().map(|t| json!({
                    "name": t.name,
                    "columns": t.columns.iter().map(|c| json!({ "name": c.name, "kind": c.kind })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "pages": m.pages.iter().map(|p| json!({ "path": p.path, "title": p.title, "kind": p.kind })).collect::<Vec<_>>(),
                "permissions": {
                    "db": m.permissions.db,
                    "kv": m.permissions.kv,
                    "storage": m.permissions.storage,
                },
            })
        })
        .collect();
    let text = serde_json::to_string_pretty(&plugins).unwrap_or_else(|_| "[]".into());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": { "plugins": plugins },
        "isError": false,
    })
}

async fn plugin_call(caller: &Caller, args: &Value) -> Result<Value, String> {
    let plugin = args
        .get("plugin")
        .and_then(Value::as_str)
        .ok_or("plugin is required")?;
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("path is required")?;
    if !path.starts_with('/') {
        return Err("path must start with '/'".into());
    }
    let method = args
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return Err(format!("unsupported method '{method}'"));
    }
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let body = args.get("body").filter(|b| !b.is_null()).map(|b| match b {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    });

    if !scope_allows(&caller.scopes, &format!("/api/{plugin}{path}")) {
        return Err("token scope does not allow this plugin".into());
    }
    let response = invoke_plugin(plugin, &method, path.trim_start_matches('/'), query, body).await;
    Ok(render(response).await)
}

/// Turn a plugin response into a tool result: the status and body as text,
/// plus structured fields. Status 400 and above is an error result.
async fn render(response: Result<Response, tpt_home_core::error::ApiError>) -> Value {
    let response = match response {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    let status = response.status();
    let bytes = match response.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => Bytes::new(),
    };
    let body = String::from_utf8_lossy(&bytes).into_owned();
    json!({
        "content": [{ "type": "text", "text": format!("HTTP {}\n{body}", status.as_u16()) }],
        "structuredContent": { "status": status.as_u16(), "body": body },
        "isError": status.is_client_error() || status.is_server_error(),
    })
}

/// Link anchor for the binary: see `tpt-home-cli`'s `BLUEPRINT_LINK_ANCHORS`.
#[inline(never)]
pub fn __link_anchor() -> &'static str {
    env!("CARGO_PKG_NAME")
}

// `POST /api/mcp` for HTTP agents. The `/api` guard has already authenticated
// the caller and attached its token scopes.
inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-mcp",
        method: "POST",
        path: "/api/mcp",
        role: None,
        register: |router| {
            router.route("/api/mcp", axum::routing::post(http_endpoint))
        },
    }
}

async fn http_endpoint(
    Extension(user): Extension<AuthUser>,
    scopes: Option<Extension<TokenScopes>>,
    body: Bytes,
) -> Response {
    let caller = Caller {
        user,
        scopes: scopes
            .map(|s| s.0.0)
            .unwrap_or_else(|| vec!["api".to_string()]),
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(rpc_error(
                    &Value::Null,
                    PARSE_ERROR,
                    &format!("invalid JSON: {e}"),
                )),
            )
                .into_response();
        }
    };
    match handle(&caller, &message).await {
        Some(reply) => axum::Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// The HTTP transport as a router: `POST /api/mcp`. The caller's `AuthUser`
/// (and optional `TokenScopes`) must already be attached by the auth guard, as
/// it is in the server. Exposed so tests can drive the endpoint directly.
pub fn http_router() -> axum::Router {
    axum::Router::new().route("/api/mcp", axum::routing::post(http_endpoint))
}

/// One stdio line in, one reply line out (newline included), or `None` for a
/// blank line or a notification. Parse errors get a JSON-RPC error reply.
pub async fn stdio_reply(caller: &Caller, line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let reply = match serde_json::from_str::<Value>(line) {
        Ok(message) => handle(caller, &message).await,
        Err(e) => Some(rpc_error(
            &Value::Null,
            PARSE_ERROR,
            &format!("invalid JSON: {e}"),
        )),
    }?;
    let mut text = reply.to_string();
    text.push('\n');
    Some(text)
}

/// Serve MCP over stdio: one JSON-RPC message per line on stdin, one reply per
/// line on stdout. Returns when stdin closes.
pub async fn run_stdio(caller: Caller) -> std::io::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if let Some(text) = stdio_reply(&caller, &line).await {
            stdout.write_all(text.as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

/// Resolve a bearer token into a stdio caller. Used by `tpt-home mcp`.
pub async fn caller_for_token(db: &DbPool, token: &str) -> Result<Caller, anyhow::Error> {
    tpt_home_core::auth::api_token::resolve_api_token(db, token)
        .await?
        .map(Caller::from)
        .ok_or_else(|| anyhow::anyhow!("token is unknown, expired or revoked"))
}
