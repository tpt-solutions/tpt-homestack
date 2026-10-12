// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Sandbox tests for the JS script tier. Each test runs one script through
//! `script::call_script` and checks that hostile or broken scripts produce
//! `Err`, never a panic, a hang, or access outside their grant.

#![cfg(feature = "js")]

use tpt_home_plugins::manifest::{self, Permissions, PluginManifest};
use tpt_home_plugins::runtime::{PluginRequest, PluginResponse, PluginUser};
use tpt_home_plugins::script::{self, ScriptSource};

fn manifest_with(max_fuel: u64, max_millis: u64) -> PluginManifest {
    manifest::parse(&format!(
        r#"
[plugin]
id = "scripts"
name = "Scripts"
version = "0.0.1"
entry = "main.js"
runtime = "js"

[limits]
max_fuel = {max_fuel}
max_memory_mb = 16
max_millis = {max_millis}
"#
    ))
    .expect("manifest")
}

fn request() -> PluginRequest {
    PluginRequest {
        method: "GET".into(),
        path: "/".into(),
        query: String::new(),
        body: None,
        user: None,
    }
}

/// Run `source` with the given grants and budget on a roomy thread: Boa's
/// evaluator is recursive and needs more stack in debug builds.
fn run(
    source: &str,
    perms: Permissions,
    max_fuel: u64,
    max_millis: u64,
) -> Result<PluginResponse, anyhow::Error> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let script = ScriptSource::new(source)?;
            let dir = tempfile::tempdir()?;
            script::call_script(
                &script,
                &manifest_with(max_fuel, max_millis),
                &request(),
                "scripts",
                &perms,
                &dir.path().join("none.db"),
                dir.path(),
            )
        })
        .expect("spawn")
        .join()
        .expect("script run panicked")
}

fn ok(source: &str) -> PluginResponse {
    run(source, Permissions::default(), 50_000_000, 5_000).expect("script should succeed")
}

fn err(source: &str) -> String {
    run(source, Permissions::default(), 50_000_000, 5_000)
        .expect_err("script should fail")
        .to_string()
}

#[test]
fn handle_returns_a_response() {
    let r = ok(r#"function handle(req) {
        return { status: 201, content_type: "text/plain", body: "hi " + req.path };
    }"#);
    assert_eq!(r.status, 201);
    assert_eq!(r.body, "hi /");
}

#[test]
fn missing_handle_is_an_error() {
    let msg = err("var x = 1;");
    assert!(msg.contains("handle"), "{msg}");
}

#[test]
fn infinite_loop_hits_the_loop_limit() {
    // max_fuel 1e6 gives roughly 20k iterations per loop.
    let msg = run(
        "function handle() { while (true) {} }",
        Permissions::default(),
        1_000_000,
        5_000,
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("loop iteration limit"), "{msg}");
}

#[test]
fn deep_recursion_is_stopped() {
    let msg = err("function f(n) { return f(n + 1); } function handle() { return f(0); }");
    assert!(msg.contains("recursive calls"), "{msg}");
}

#[test]
fn eval_is_removed() {
    let r = ok(r#"function handle() {
        return { status: 200, body: typeof eval };
    }"#);
    assert_eq!(r.body, "undefined");

    let msg = err(r#"function handle() { return { status: 200, body: String(eval("1+1")) }; }"#);
    assert!(msg.contains("eval is not defined"), "{msg}");
}

#[test]
fn function_constructor_escapes_are_closed() {
    let r = ok(r#"function handle() {
        const viaFn = (function () {}).constructor;
        const viaArrow = (() => {}).constructor;
        const viaAsync = (async function () {}).constructor;
        const viaGen = (function* () {}).constructor;
        return { status: 200, body: [viaFn, viaArrow, viaAsync, viaGen].map((c) => typeof c).join(",") };
    }"#);
    assert_eq!(r.body, "undefined,undefined,undefined,undefined");

    let msg = err(r#"function handle() {
        return { status: 200, body: (() => {}).constructor("return 1")() };
    }"#);
    assert!(msg.contains("not a callable"), "{msg}");
}

#[test]
fn module_syntax_is_rejected_at_load() {
    assert!(ScriptSource::new("import x from 'y'; function handle() {}".into()).is_err());
    assert!(ScriptSource::new("export const a = 1;".into()).is_err());
}

#[test]
fn oversize_source_is_rejected_at_load() {
    let big = format!(
        "function handle() {{}} // {}",
        "x".repeat(script::MAX_SOURCE_BYTES)
    );
    assert!(ScriptSource::new(big).is_err());
}

#[test]
fn no_timers_modules_or_host_leaks() {
    let r = ok(r#"function handle() {
        const names = ["setTimeout", "setInterval", "require", "fetch", "process",
                       "__tpt_log", "__tpt_db_query", "__tpt_kv_get"];
        return { status: 200, body: names.map((n) => typeof globalThis[n]).join(",") };
    }"#);
    assert_eq!(
        r.body,
        "undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined"
    );
    let tpt = ok("function handle() { return { status: 200, body: typeof tpt }; }");
    assert_eq!(tpt.body, "object");
}

#[test]
fn ungranted_kv_read_is_null() {
    let r = ok(r#"function handle() {
        return { status: 200, body: String(tpt.kv.get("secret")) };
    }"#);
    assert_eq!(r.body, "null");
}

#[test]
fn ungranted_write_reports_false() {
    let r = ok(r#"function handle() {
        return { status: 200, body: String(tpt.kv.set("a", "b")) };
    }"#);
    assert_eq!(r.body, "false");
}

#[test]
fn out_of_scope_sql_is_null_even_when_granted() {
    let perms = Permissions {
        db: true,
        ..Default::default()
    };
    let r = run(
        r#"function handle() {
            return { status: 200, body: String(tpt.db.query("SELECT * FROM plugin_kv")) };
        }"#,
        perms,
        50_000_000,
        5_000,
    )
    .expect("out-of-scope SQL must not trap");
    assert_eq!(r.body, "null");
}

#[test]
fn storage_round_trip_when_granted() {
    let perms = Permissions {
        storage: true,
        ..Default::default()
    };
    let r = run(
        r#"function handle() {
            tpt.storage.write("note.txt", "hello");
            return { status: 200, body: String(tpt.storage.read("note.txt")) };
        }"#,
        perms,
        50_000_000,
        5_000,
    )
    .expect("storage script");
    assert_eq!(r.body, "hello");
}

#[test]
fn host_call_loop_hits_the_deadline() {
    // Five million host calls far exceed a 1 ms budget, while fuel is ample.
    let msg = run(
        r#"function handle() {
            for (let i = 0; i < 5000000; i++) { tpt.log("x"); }
            return { status: 200, body: "done" };
        }"#,
        Permissions::default(),
        1_000_000_000,
        1,
    )
    .unwrap_err()
    .to_string();
    assert!(msg.contains("deadline"), "{msg}");
}

#[test]
fn now_gives_the_clock_without_any_grant() {
    let r = ok(r#"function handle() {
        const t = tpt.now();
        const ok = t.unix > 1700000000
            && /^\d{4}-\d{2}-\d{2}$/.test(t.local_date)
            && t.local_hour >= 0 && t.local_hour < 24
            && t.local_minute >= 0 && t.local_minute < 60
            && Math.abs(t.utc_offset_minutes) <= 14 * 60;
        return { status: 200, body: ok ? "ok" : JSON.stringify(t) };
    }"#);
    assert_eq!(r.body, "ok");
}

#[test]
fn now_is_frozen_out_of_the_global_surface() {
    let r = ok(r#"function handle() {
        return { status: 200, body: typeof __tpt_now + " " + typeof tpt.now };
    }"#);
    assert_eq!(r.body, "undefined function");
}

/// Like [`run`], with a caller on the request.
fn run_as_user(source: &str) -> Result<PluginResponse, anyhow::Error> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let script = ScriptSource::new(source)?;
            let dir = tempfile::tempdir()?;
            let mut req = request();
            req.user = Some(PluginUser {
                id: 7,
                username: "ana".into(),
                role: "user".into(),
            });
            script::call_script(
                &script,
                &manifest_with(50_000_000, 5_000),
                &req,
                "scripts",
                &Permissions::default(),
                &dir.path().join("none.db"),
                dir.path(),
            )
        })
        .expect("spawn")
        .join()
        .expect("script run panicked")
}

#[test]
fn the_caller_reaches_the_script_and_is_absent_when_anonymous() {
    let r = run_as_user(
        r#"function handle(req) {
        return { status: 200, body: req.user.username + " " + req.user.role + " " + req.user.id };
    }"#,
    )
    .expect("script should succeed");
    assert_eq!(r.body, "ana user 7");

    let anonymous = ok(r#"function handle(req) {
        return { status: 200, body: String("user" in req) };
    }"#);
    assert_eq!(anonymous.body, "false");
}
