// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! End-to-end check of the example calendar plugin through the real wasmi
//! sandbox: the plugin is compiled for `wasm32-unknown-unknown`, instantiated
//! with the host's `call_plugin`, and driven over its HTTP-shaped routes.
//!
//! Ignored by default because it runs a nested wasm build. Run with:
//! `cargo test -p tpt-home-plugins --test calendar_plugin -- --ignored`

use std::path::{Path, PathBuf};
use std::process::Command;

use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::runtime::{self, PluginRequest, PluginResponse};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn build_calendar(target_dir: &Path) -> Vec<u8> {
    let status = Command::new(env!("CARGO"))
        .current_dir(repo_root())
        .args([
            "build",
            "-p",
            "tpt-home-bp-calendar",
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--target-dir",
        ])
        .arg(target_dir)
        .status()
        .expect("run cargo");
    assert!(status.success(), "building the calendar plugin for wasm32 failed");
    std::fs::read(target_dir.join("wasm32-unknown-unknown/release/tpt_home_bp_calendar.wasm"))
        .expect("wasm artifact")
}

#[test]
#[ignore = "builds the example plugin for wasm32-unknown-unknown; run with --ignored"]
fn calendar_plugin_end_to_end() {
    // Debug builds of wasmi recurse deeply; give the body a roomy stack. Release
    // builds fit in the default 2 MiB.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .expect("spawn test thread")
        .join()
        .unwrap();
}

fn run() {
    let tmp = tempfile::tempdir().unwrap();
    let wasm = build_calendar(&tmp.path().join("target"));
    let manifest =
        manifest::from_dir(&repo_root().join("plugins/tpt-home-bp-calendar")).expect("manifest");
    let module = wasmi::Module::new(runtime::engine(), wasm.as_slice()).expect("compile wasm");

    // The host creates `<id>_<name>` tables at install; mirror that here.
    let db = tmp.path().join("app.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE calendar_events (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                starts_at INTEGER NOT NULL
            );",
        )
        .unwrap();
    let storage = tmp.path().join("storage");

    let call = |perms: &Permissions, method: &str, path: &str, query: &str, body: Option<&str>| {
        runtime::call_plugin(
            &module,
            &manifest,
            &PluginRequest {
                method: method.into(),
                path: path.into(),
                query: query.into(),
                body: body.map(String::from),
            },
            "calendar",
            perms,
            &db,
            &storage,
        )
        .expect("plugin invocation")
    };
    let granted = manifest.permissions.clone();
    let json_rows = |r: &PluginResponse| -> Vec<serde_json::Value> {
        serde_json::from_str(&r.body).expect("JSON array")
    };

    // Create two events. The second title is hostile SQL and must be stored
    // verbatim, not executed.
    let r = call(&granted, "POST", "/events", "", Some(r#"{"title":"Dentist","starts_at":1700000000}"#));
    assert_eq!(r.status, 201, "{}", r.body);
    let hostile = "x'); DROP TABLE calendar_events; --";
    let body = serde_json::json!({ "title": hostile, "starts_at": 1700000100 }).to_string();
    let r = call(&granted, "POST", "/events", "", Some(&body));
    assert_eq!(r.status, 201, "{}", r.body);

    // List: ordered by start time, hostile title intact, table still there.
    let r = call(&granted, "GET", "/events", "", None);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.content_type, "application/json");
    let rows = json_rows(&r);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["title"], "Dentist");
    assert_eq!(rows[1]["title"], hostile);

    // Validation: empty titles and bad bodies are client errors.
    let r = call(&granted, "POST", "/events", "", Some(r#"{"title":"   ","starts_at":1}"#));
    assert_eq!(r.status, 400, "{}", r.body);
    let r = call(&granted, "POST", "/events", "", Some("not json"));
    assert_eq!(r.status, 400, "{}", r.body);

    // Delete by id; a second delete of the same id is a 404.
    let id = rows[0]["id"].as_i64().unwrap();
    let r = call(&granted, "DELETE", "/events", &format!("id={id}"), None);
    assert_eq!(r.status, 200, "{}", r.body);
    let r = call(&granted, "DELETE", "/events", &format!("id={id}"), None);
    assert_eq!(r.status, 404, "{}", r.body);
    let r = call(&granted, "DELETE", "/events", "id=abc", None);
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(json_rows(&call(&granted, "GET", "/events", "", None)).len(), 1);

    // Without the `db` permission the host denies the query and the plugin
    // reports it as an error response, not a trap.
    let r = call(&Permissions::default(), "GET", "/events", "", None);
    assert_eq!(r.status, 500, "{}", r.body);
    assert!(r.body.contains("denied"), "{}", r.body);

    // Unknown routes are the plugin's own 404.
    let r = call(&granted, "GET", "/nope", "", None);
    assert_eq!(r.status, 404, "{}", r.body);
}
