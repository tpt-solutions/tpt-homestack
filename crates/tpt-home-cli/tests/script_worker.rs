// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The JS script worker, run for real: each call starts a child process of the
//! `tpt-home` binary, as the server does. Covers the happy path, script errors,
//! the loop limit, the call deadline, the memory limit, and the worker's own
//! handling of a bad job.
//!
//! This is its own test binary because the worker executable is set once per
//! process.

#![cfg(feature = "js")]

use std::io::Write;
use std::process::{Command, Stdio};

use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::runtime::PluginRequest;
use tpt_home_plugins::script::{self, ScriptSource};
use tpt_home_plugins::script_worker;

fn manifest(max_fuel: u64, max_memory_mb: u64, max_millis: u64) -> manifest::PluginManifest {
    manifest::parse(&format!(
        r#"
[plugin]
id = "worker_probe"
name = "Worker probe"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[limits]
max_fuel = {max_fuel}
max_memory_mb = {max_memory_mb}
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

/// Run `source` through the runtime entry point, which uses the worker because
/// `set_worker_exe` was called first.
fn call(
    source: &str,
    max_fuel: u64,
    max_memory_mb: u64,
    max_millis: u64,
) -> Result<String, anyhow::Error> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let script = ScriptSource::new(source).expect("script parses");
            let dir = tempfile::tempdir().unwrap();
            let manifest = manifest(max_fuel, max_memory_mb, max_millis);
            script::call_script(
                &script,
                &manifest,
                &request(),
                "worker_probe",
                &Permissions::default(),
                &dir.path().join("none.db"),
                dir.path(),
            )
            .map(|reply| reply.body)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn set_worker() {
    script_worker::set_worker_exe(std::path::PathBuf::from(env!("CARGO_BIN_EXE_tpt-home")));
}

#[test]
fn worker_returns_the_script_response() {
    set_worker();
    let body = call(
        r#"function handle(req) { return { status: 200, content_type: "text/plain", body: "hi " + req.method }; }"#,
        50_000_000,
        16,
        5_000,
    )
    .expect("worker call succeeds");
    assert_eq!(body, "hi GET");
}

#[test]
fn a_thrown_error_comes_back_as_a_script_error() {
    set_worker();
    let err = call(
        r#"function handle() { throw new Error("boom from script"); }"#,
        50_000_000,
        16,
        5_000,
    )
    .expect_err("script error expected");
    let text = format!("{err:#}");
    assert!(text.contains("script error"), "got: {text}");
    assert!(text.contains("boom from script"), "got: {text}");
}

#[test]
fn an_infinite_loop_is_stopped_by_the_loop_limit() {
    set_worker();
    // Small fuel means a small loop budget, so the engine stops the loop well
    // before the deadline.
    let err = call(
        r#"function handle() { while (true) {} }"#,
        50_000,
        16,
        5_000,
    )
    .expect_err("loop must be stopped");
    assert!(!format!("{err:#}").is_empty());
}

#[test]
fn a_call_that_keeps_calling_the_host_hits_the_deadline() {
    set_worker();
    // Each host call checks the deadline, so a host-calling loop stops at the
    // call's own limit, and the message names the deadline.
    let err = call(
        r#"function handle() { while (true) { tpt.now(); } }"#,
        u64::MAX / 2,
        16,
        300,
    )
    .expect_err("deadline expected");
    assert!(
        format!("{err:#}").contains("plugin call deadline exceeded"),
        "got: {err:#}"
    );
}

#[test]
fn growing_memory_past_the_limit_fails_only_that_call() {
    set_worker();
    // The string doubles each iteration. The job object (Windows) or RLIMIT_DATA
    // (Linux) stops the worker, or the engine rejects the allocation. Either way
    // the call fails, and the test process, which is the server stand-in, keeps
    // running and can make a second call.
    let grow = r#"function handle() { let s = "x"; while (true) { s = s + s; } }"#;
    let err = call(grow, 500_000_000, 16, 10_000).expect_err("memory growth must fail");
    assert!(!format!("{err:#}").is_empty());

    let body = call(
        r#"function handle() { return { status: 200, content_type: "text/plain", body: "still here" }; }"#,
        50_000_000,
        16,
        5_000,
    )
    .expect("a later call still works");
    assert_eq!(body, "still here");
}

#[test]
fn worker_rejects_a_bad_job_with_exit_code_2() {
    // The worker reads one JSON job from stdin. Garbage must exit 2 without
    // panicking, and without writing a result the parent could mistake for one.
    let mut child = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg(script_worker::WORKER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start worker");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not json\n")
        .unwrap();
    let output = child.wait_with_output().expect("worker exits");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bad job"), "stderr: {stderr}");
}
