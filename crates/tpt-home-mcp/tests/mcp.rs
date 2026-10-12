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
