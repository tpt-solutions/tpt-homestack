// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! A worker executable that does not exist must fail the one call with a clear
//! error, not panic or run the script in the server process.
//!
//! Its own test binary because the worker executable is set once per process.

#![cfg(feature = "js")]

use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::runtime::PluginRequest;
use tpt_home_plugins::script::{self, ScriptSource};
use tpt_home_plugins::script_worker;

#[test]
fn missing_worker_executable_fails_the_call() {
    script_worker::set_worker_exe(std::path::PathBuf::from(
        "does-not-exist/tpt-home-worker-missing",
    ));
    let manifest = manifest::parse(
        r#"
[plugin]
id = "worker_probe"
name = "Worker probe"
version = "0.1.0"
entry = "main.js"
runtime = "js"
"#,
    )
    .expect("manifest");
    let script = ScriptSource::new(
        r#"function handle() { return { status: 200, content_type: "text/plain", body: "ran" }; }"#
            .to_string(),
    )
    .expect("script parses");
    let dir = tempfile::tempdir().unwrap();
    let err = script::call_script(
        &script,
        &manifest,
        &PluginRequest {
            method: "GET".into(),
            path: "/".into(),
            query: String::new(),
            body: None,
            user: None,
        },
        "worker_probe",
        &Permissions::default(),
        &dir.path().join("none.db"),
        dir.path(),
    )
    .expect_err("call must fail when the worker cannot start");
    assert!(
        format!("{err:#}").contains("start script worker"),
        "got: {err:#}"
    );
}
