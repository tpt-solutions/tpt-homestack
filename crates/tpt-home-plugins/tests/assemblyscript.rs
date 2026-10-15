// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The AssemblyScript example (`docs/examples/assemblyscript`) is a real plugin
//! written in another language. This runs its committed `plugin.wasm` through
//! the host and checks the responses the example documents, so the example and
//! the ABI stay in step.

use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::runtime::{self, PluginRequest, PluginResponse};

const WASM_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/examples/assemblyscript/plugin.wasm"
);

fn manifest() -> tpt_home_plugins::manifest::PluginManifest {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/examples/assemblyscript/plugin.toml"
    ))
    .expect("read the example manifest");
    manifest::parse(&text).expect("parse the example manifest")
}

fn request(method: &str, path: &str, query: &str, body: Option<&str>) -> PluginRequest {
    PluginRequest {
        method: method.into(),
        path: path.into(),
        query: query.into(),
        body: body.map(str::to_string),
        user: None,
    }
}

/// Run the example on a thread with a large stack (wasmi recursion in debug
/// builds needs it, as the conformance test does).
fn call(req: PluginRequest) -> PluginResponse {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let bytes = std::fs::read(WASM_PATH).expect("read plugin.wasm");
            let module = wasmi::Module::new(runtime::engine(), &bytes[..]).expect("compile");
            let dir = tempfile::tempdir().unwrap();
            runtime::call_plugin(
                &module,
                &manifest(),
                &req,
                "hello_as",
                &Permissions::default(),
                &dir.path().join("none.db"),
                dir.path(),
            )
            .expect("the example runs")
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn the_assemblyscript_greeting_matches_its_documentation() {
    let res = call(request("GET", "/hello", "name=Ada", None));
    assert_eq!(res.status, 200);
    assert_eq!(res.content_type, "application/json");
    assert_eq!(res.body, r#"{"greeting":"Hello, Ada"}"#);

    let anonymous = call(request("GET", "/hello", "", None));
    assert_eq!(anonymous.body, r#"{"greeting":"Hello, stranger"}"#);

    // Characters that would break the JSON are dropped, not echoed.
    let tricky = call(request("GET", "/hello", r#"name=Ad"a<b>"#, None));
    assert_eq!(tricky.body, r#"{"greeting":"Hello, Adab"}"#);
}

#[test]
fn the_assemblyscript_post_reports_the_body_length() {
    let res = call(request("POST", "/hello", "", Some("abcdef")));
    assert_eq!(res.status, 200);
    assert_eq!(res.body, r#"{"received_bytes":6}"#);
}

#[test]
fn unknown_routes_and_methods_get_their_status() {
    assert_eq!(call(request("GET", "/nope", "", None)).status, 404);
    assert_eq!(call(request("DELETE", "/hello", "", None)).status, 405);
}
