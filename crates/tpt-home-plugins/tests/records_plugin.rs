// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The `plugins/records` JavaScript plugin, installed for real and called
//! through the same dispatch path as HTTP and MCP. The plugin opens the
//! configured database file, so these tests use a file database too.
//!
//! Each test installs its own copy under a unique id, but the tests take turns
//! (see [`turn`]): they share one database file, and parallel writers would
//! hit SQLite's lock. Covers the add,
//! list and delete rules, and that quotes in user text cannot escape the SQL
//! the plugin builds.

#![cfg(feature = "js")]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use axum::body::to_bytes;
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::connect;
use tpt_home_plugins::dispatch::invoke_plugin;
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::manifest::Permissions;

fn data_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::mem::forget(dir);
        set_global_for_tests(Config {
            data_dir: root.clone(),
            ..Config::default()
        });
        root
    })
}

/// Copy the shipped plugin under `id`: the manifest id and the table names in
/// the script are rewritten. Install renames the directory, so it is a copy.
fn staged_copy(id: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/records");
    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = std::fs::read_to_string(source.join("plugin.toml"))
        .unwrap()
        .replace("id = \"records\"", &format!("id = \"{id}\""));
    let script = std::fs::read_to_string(source.join("main.js"))
        .unwrap()
        .replace("records_records", &format!("{id}_records"));
    std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
    std::fs::write(dir.join("main.js"), script).unwrap();
    dir
}

/// One test at a time over the shared database file.
async fn turn() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Migrate the server database file and install a fresh copy of the plugin.
async fn install(id: &str) {
    data_root();
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let db = connect(
        &Config {
            data_dir: data_root().to_path_buf(),
            ..Config::default()
        }
        .db_path(),
    )
    .await
    .unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let grant = Permissions::parse_list("db").unwrap();
    lifecycle::install_from_dir(&db, &staged_copy(id), true, &grant)
        .await
        .unwrap();
}

/// Returns the status and the body text of one call.
async fn call(id: &str, method: &str, rest: &str, body: Option<&str>) -> (u16, String) {
    let response = invoke_plugin(None, id, method, rest, "", body.map(String::from))
        .await
        .unwrap_or_else(|e| panic!("invoke {method} {rest}: {e:?}"));
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn list(id: &str) -> Vec<serde_json::Value> {
    let (status, text) = call(id, "GET", "records", None).await;
    assert_eq!(status, 200, "{text}");
    serde_json::from_str(&text).unwrap()
}

/// A unique plugin id: lowercase letters and digits, starting with a letter.
fn fresh_id(tag: &str) -> String {
    format!("{tag}{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

#[tokio::test]
async fn add_list_and_delete_round_trip() {
    let _turn = turn().await;
    let id = fresh_id("rta");
    install(&id).await;

    let (status, text) = call(
        &id,
        "POST",
        "records",
        Some(r#"{"title":"Torch","notes":"in the hall"}"#),
    )
    .await;
    assert_eq!(status, 201, "{text}");

    let rows = list(&id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["title"], "Torch");
    assert_eq!(rows[0]["notes"], "in the hall");
    let row = rows[0]["id"].as_i64().unwrap();

    let (status, text) = call(&id, "POST", "delete", Some(&format!(r#"{{"id":"{row}"}}"#))).await;
    assert_eq!(
        status, 200,
        "form values arrive as strings and must still delete: {text}"
    );
    assert!(list(&id).await.is_empty());
}

#[tokio::test]
async fn quotes_in_user_text_are_stored_literally() {
    let _turn = turn().await;
    let id = fresh_id("rtq");
    install(&id).await;
    let title = "O'Brien'); DROP TABLE x; --";
    let body = serde_json::json!({ "title": title, "notes": "it's fine" }).to_string();
    let (status, text) = call(&id, "POST", "records", Some(&body)).await;
    assert_eq!(status, 201, "{text}");

    let rows = list(&id).await;
    assert_eq!(rows.len(), 1, "the row is stored as text");
    assert_eq!(rows[0]["title"], title);
    assert_eq!(rows[0]["notes"], "it's fine");
}

#[tokio::test]
async fn bad_input_is_refused_with_a_reason() {
    let _turn = turn().await;
    let id = fresh_id("rtb");
    install(&id).await;
    let (status, text) = call(&id, "POST", "records", Some(r#"{"title":"   "}"#)).await;
    assert_eq!(status, 400);
    assert!(text.contains("title must not be empty"), "{text}");

    let long = "x".repeat(201);
    let (status, text) = call(
        &id,
        "POST",
        "records",
        Some(&format!(r#"{{"title":"{long}"}}"#)),
    )
    .await;
    assert_eq!(status, 400);
    assert!(text.contains("too long"), "{text}");

    let (status, _) = call(&id, "POST", "records", Some("not json")).await;
    assert_eq!(status, 400);
    assert!(list(&id).await.is_empty());
}

#[tokio::test]
async fn deleting_a_missing_or_invalid_id_is_refused() {
    let _turn = turn().await;
    let id = fresh_id("rtd");
    install(&id).await;
    let (status, _) = call(&id, "POST", "delete", Some(r#"{"id":"9999"}"#)).await;
    assert_eq!(status, 404);

    let (status, _) = call(&id, "POST", "delete", Some(r#"{"id":"1 OR 1=1"}"#)).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn unknown_routes_are_404() {
    let _turn = turn().await;
    let id = fresh_id("rtu");
    install(&id).await;
    let (status, _) = call(&id, "PUT", "records", None).await;
    assert_eq!(status, 404);
}
