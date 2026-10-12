// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Conformance: the same fixture plugin, written once as wasm and once as a
//! JS script, must produce identical responses through the host. This pins
//! the shared response contract (`{status, content_type, body}`) across
//! runtimes.

use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::runtime::{self, PluginRequest, PluginResponse};

const FIXTURE_JSON: &str = r#"{"status":200,"content_type":"text/plain","body":"hello"}"#;

/// Wasm fixture: returns the fixture JSON placed at offset 0 of memory.
fn wasm_fixture() -> String {
    format!(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "{}")
            (func (export "tpt_alloc") (param i32) (result i32) i32.const 1024)
            (func (export "tpt_entry") (param i32 i32) (result i64)
                i64.const {}))"#,
        FIXTURE_JSON.replace('"', "\\\""),
        FIXTURE_JSON.len()
    )
}

/// JS fixture: returns the same response from `handle`.
#[cfg(feature = "js")]
const JS_FIXTURE: &str = r#"
function handle(req) {
    return { status: 200, content_type: "text/plain", body: "hello" };
}
"#;

fn request() -> PluginRequest {
    PluginRequest {
        method: "GET".into(),
        path: "/".into(),
        query: String::new(),
        body: None,
        user: None,
    }
}

fn manifest(runtime: &str, entry: &str) -> tpt_home_plugins::manifest::PluginManifest {
    manifest::parse(&format!(
        r#"
[plugin]
id = "fixture"
name = "Fixture"
version = "0.0.1"
entry = "{entry}"
runtime = "{runtime}"

[limits]
max_fuel = 50000000
max_memory_mb = 16
max_millis = 5000
"#
    ))
    .expect("manifest")
}

fn run_wasm() -> PluginResponse {
    let wat = wasm_fixture();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let module = wasmi::Module::new(runtime::engine(), wat.as_bytes()).expect("compile");
            let dir = tempfile::tempdir().unwrap();
            runtime::call_plugin(
                &module,
                &manifest("wasm", "plugin.wasm"),
                &request(),
                "fixture",
                &Permissions::default(),
                &dir.path().join("none.db"),
                dir.path(),
            )
            .expect("wasm call")
        })
        .unwrap()
        .join()
        .unwrap()
}

#[cfg(feature = "js")]
fn run_js() -> PluginResponse {
    use tpt_home_plugins::script::{self, ScriptSource};
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let source = ScriptSource::new(JS_FIXTURE.to_string()).expect("parse");
            let dir = tempfile::tempdir().unwrap();
            script::call_script(
                &source,
                &manifest("js", "main.js"),
                &request(),
                "fixture",
                &Permissions::default(),
                &dir.path().join("none.db"),
                dir.path(),
            )
            .expect("js call")
        })
        .unwrap()
        .join()
        .unwrap()
}

#[cfg(feature = "js")]
#[test]
fn wasm_and_js_fixtures_respond_identically() {
    let wasm = run_wasm();
    let js = run_js();
    assert_eq!(wasm.status, js.status);
    assert_eq!(wasm.content_type, js.content_type);
    assert_eq!(wasm.body, js.body);
    assert_eq!(wasm.body, "hello");
}

#[cfg(not(feature = "js"))]
#[test]
fn wasm_fixture_responds_as_documented() {
    let wasm = run_wasm();
    assert_eq!(wasm.status, 200);
    assert_eq!(wasm.body, "hello");
}
