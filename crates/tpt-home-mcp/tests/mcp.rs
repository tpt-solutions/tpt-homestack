// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! MCP protocol and authorization tests. A small wasm plugin is registered in
//! the plugin registry, so tool calls run through the real dispatch path.

use std::sync::OnceLock;

use serde_json::{Value, json};
use tpt_home_core::auth::user::{AuthUser, Role};
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_mcp::{Caller, handle};
use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::registry::{self, Backend, LoadedPlugin};

/// Global config for the process: plugin storage and db paths hang off it.
fn init_config() {
    static CFG: OnceLock<()> = OnceLock::new();
    CFG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        std::mem::forget(dir);
        set_global_for_tests(Config {
            data_dir,
            ..Config::default()
        });
    });
}

/// A wasm plugin that answers every request with "hello" and status 200.
fn register_fixture() {
    init_config();
    const JSON: &str = r#"{"status":200,"content_type":"text/plain","body":"hello"}"#;
    let wat = format!(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "{}")
            (func (export "tpt_alloc") (param i32) (result i32) i32.const 1024)
            (func (export "tpt_entry") (param i32 i32) (result i64) i64.const {}))"#,
        JSON.replace('"', "\\\""),
        JSON.len()
    );
    let manifest = manifest::parse(
        r#"
[plugin]
id = "fixture"
name = "Fixture"
version = "0.1.0"
entry = "plugin.wasm"

[[pages]]
path = "/add"
title = "Add an item"
kind = "form"
fields = ["label"]
action = "add"
"#,
    )
    .unwrap();
    let module = wasmi::Module::new(tpt_home_plugins::runtime::engine(), wat.as_bytes()).unwrap();
    registry::put(
        LoadedPlugin {
            manifest,
            backend: Backend::Wasm(module),
            dir: std::env::temp_dir(),
            approved: Permissions::default(),
        },
        true,
    );
}

fn caller(scopes: &[&str]) -> Caller {
    Caller {
        user: AuthUser {
            id: 1,
            username: "alice".into(),
            role: Role::Admin,
        },
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        forward: Vec::new(),
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

async fn reply(caller: &Caller, message: Value) -> Value {
    handle(caller, &message).await.expect("a reply")
}

fn tool_names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn initialize_advertises_tools() {
    let r = reply(&caller(&["api"]), rpc("initialize", json!({}))).await;
    assert_eq!(
        r["result"]["protocolVersion"],
        tpt_home_mcp::PROTOCOL_VERSION
    );
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "tpt-homestack");
}

#[tokio::test]
async fn notifications_get_no_reply_and_unknown_methods_error() {
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(handle(&caller(&["api"]), &note).await.is_none());

    let r = reply(&caller(&["api"]), rpc("nope/nothing", json!({}))).await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn tools_list_derives_form_actions_from_plugins() {
    register_fixture();
    let list = reply(&caller(&["api"]), rpc("tools/list", json!({}))).await;
    let names = tool_names(&list);
    assert!(names.contains(&"plugins_list".to_string()));
    assert!(names.contains(&"plugin_call".to_string()));
    assert!(names.contains(&"fixture_add".to_string()), "{names:?}");

    let add = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "fixture_add")
        .unwrap();
    assert_eq!(add["inputSchema"]["required"], json!(["label"]));
}

#[tokio::test]
async fn plugin_call_runs_the_plugin_and_reports_status() {
    register_fixture();
    let r = reply(
        &caller(&["api"]),
        rpc(
            "tools/call",
            json!({ "name": "plugin_call", "arguments": { "plugin": "fixture", "path": "/" } }),
        ),
    )
    .await;
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(r["result"]["structuredContent"]["status"], 200);
    assert_eq!(r["result"]["structuredContent"]["body"], "hello");
}

#[tokio::test]
async fn form_action_tool_posts_its_arguments() {
    register_fixture();
    let r = reply(
        &caller(&["api"]),
        rpc(
            "tools/call",
            json!({ "name": "fixture_add", "arguments": { "label": "milk" } }),
        ),
    )
    .await;
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(r["result"]["structuredContent"]["body"], "hello");
}

#[tokio::test]
async fn token_scope_hides_and_blocks_other_plugins() {
    register_fixture();
    let other = caller(&["plugin:calendar"]);

    let list = reply(&other, rpc("tools/list", json!({}))).await;
    let names = tool_names(&list);
    assert!(
        !names.contains(&"fixture_add".to_string()),
        "scope must hide fixture tools"
    );
    assert!(names.contains(&"plugins_list".to_string()));

    let call = reply(
        &other,
        rpc(
            "tools/call",
            json!({ "name": "plugin_call", "arguments": { "plugin": "fixture", "path": "/" } }),
        ),
    )
    .await;
    assert_eq!(
        call["result"]["isError"], true,
        "out-of-scope call must fail"
    );

    let plugins = reply(
        &other,
        rpc(
            "tools/call",
            json!({ "name": "plugins_list", "arguments": {} }),
        ),
    )
    .await;
    assert!(
        !plugins["result"]["structuredContent"]["plugins"]
            .to_string()
            .contains("fixture")
    );
}

#[tokio::test]
async fn plugin_call_validates_its_input() {
    register_fixture();
    let c = caller(&["api"]);
    for args in [
        json!({ "plugin": "fixture" }),
        json!({ "plugin": "fixture", "path": "no-slash" }),
        json!({ "plugin": "fixture", "path": "/", "method": "TRACE" }),
    ] {
        let r = reply(
            &c,
            rpc(
                "tools/call",
                json!({ "name": "plugin_call", "arguments": args }),
            ),
        )
        .await;
        assert_eq!(r["result"]["isError"], true, "{r}");
    }
}

#[tokio::test]
async fn unknown_tool_is_a_protocol_error() {
    let r = reply(
        &caller(&["api"]),
        rpc("tools/call", json!({ "name": "rm_rf" })),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn batches_return_one_reply_per_request() {
    let batch = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "ping" },
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "id": 2, "method": "ping" },
    ]);
    let r = handle(&caller(&["api"]), &batch).await.expect("replies");
    assert_eq!(r.as_array().unwrap().len(), 2);
}

// Transport tests: the stdio line framing and the HTTP endpoint, driven
// through the same handler the server uses.

fn with_user(scopes: Option<&[&str]>) -> axum::Router {
    use axum::Extension;
    use tpt_home_core::auth::api_token::TokenScopes;

    let user = AuthUser {
        id: 1,
        username: "alice".into(),
        role: Role::Admin,
    };
    let mut router = tpt_home_mcp::http_router().layer(Extension(user));
    if let Some(scopes) = scopes {
        router = router.layer(Extension(TokenScopes(
            scopes.iter().map(|s| s.to_string()).collect(),
        )));
    }
    router
}

async fn post_mcp(router: axum::Router, body: &str) -> (axum::http::StatusCode, Value) {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let req = Request::builder()
        .method("POST")
        .uri("/api/mcp")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = router.oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn stdio_lines_get_one_reply_each_and_skip_notifications() {
    let c = caller(&["api"]);
    let line = rpc("initialize", json!({})).to_string();
    let text = tpt_home_mcp::stdio_reply(&c, &line).await.expect("a reply");
    assert!(text.ends_with('\n'), "replies are newline-terminated");
    assert_eq!(text.matches('\n').count(), 1, "one reply, one line");
    let r: Value = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["serverInfo"]["name"], "tpt-homestack");

    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
    assert!(tpt_home_mcp::stdio_reply(&c, &note).await.is_none());
    assert!(tpt_home_mcp::stdio_reply(&c, "   ").await.is_none());
}

#[tokio::test]
async fn stdio_bad_json_gets_a_parse_error_and_the_loop_survives() {
    let c = caller(&["api"]);
    let text = tpt_home_mcp::stdio_reply(&c, "{not json")
        .await
        .expect("a parse error reply");
    let r: Value = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(r["error"]["code"], -32700);
    assert!(r["id"].is_null());

    // The next line is still served.
    let text = tpt_home_mcp::stdio_reply(&c, &rpc("initialize", json!({})).to_string())
        .await
        .expect("a reply after the bad line");
    assert!(text.contains("serverInfo"));
}

#[tokio::test]
async fn http_endpoint_answers_requests_and_accepts_notifications() {
    register_fixture();
    let (status, r) = post_mcp(with_user(None), &rpc("tools/list", json!({})).to_string()).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(tool_names(&r).contains(&"fixture_add".to_string()));

    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
    let (status, _) = post_mcp(with_user(None), &note).await;
    assert_eq!(status, axum::http::StatusCode::ACCEPTED);
}

#[tokio::test]
async fn http_endpoint_rejects_invalid_json_with_400() {
    let (status, r) = post_mcp(with_user(None), "{oops").await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(r["error"]["code"], -32700);
}

#[tokio::test]
async fn http_token_scope_limits_which_plugin_tools_are_listed() {
    register_fixture();
    // A token scoped to some other plugin must not see the fixture's tools.
    let (status, r) = post_mcp(
        with_user(Some(&["plugin:other"])),
        &rpc("tools/list", json!({})).to_string(),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let names = tool_names(&r);
    assert!(!names.contains(&"fixture_add".to_string()), "{names:?}");
}
