// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The `plugins/logbook` JavaScript plugin, installed for real: entry and
//! delete rules through the same dispatch path as HTTP, and the summary page
//! with its grouped cards.
//!
//! Like the other plugin tests, these share one database file and take turns.

#![cfg(feature = "js")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use axum::Extension;
use axum::body::to_bytes;
use axum::extract::{Path as AxPath, Query, State};
use tpt_home_core::auth::user::{AuthUser, Role};
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::{DbPool, connect};
use tpt_home_plugins::dispatch::{self, invoke_plugin};
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

fn config() -> Config {
    Config {
        data_dir: data_root().to_path_buf(),
        ..Config::default()
    }
}

/// Copy the shipped plugin under `id`. The manifest id and the table prefix
/// are rewritten, so each test gets its own tables. Install renames the
/// directory, so it is a copy.
fn staged_copy(id: &str) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/logbook");
    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = std::fs::read_to_string(source.join("plugin.toml"))
        .unwrap()
        .replace("id = \"logbook\"", &format!("id = \"{id}\""))
        .replace("logbook_", &format!("{id}_"));
    let script = std::fs::read_to_string(source.join("main.js"))
        .unwrap()
        .replace("logbook_", &format!("{id}_"));
    std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
    std::fs::write(dir.join("main.js"), script).unwrap();
    dir
}

/// One test at a time over the shared database file.
async fn turn() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Migrate the server database file, install a fresh copy, return the pool.
async fn install(id: &str) -> DbPool {
    data_root();
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let db = connect(&config().db_path()).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let grant = Permissions::parse_list("db").unwrap();
    lifecycle::install_from_dir(&db, &staged_copy(id), true, &grant)
        .await
        .unwrap();
    db
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

/// A unique plugin id: lowercase letters and digits, starting with a letter.
fn fresh_id(tag: &str) -> String {
    format!("{tag}{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

fn viewer() -> AuthUser {
    AuthUser {
        id: 1,
        username: "ana".into(),
        role: Role::User,
    }
}

async fn summary_html(db: &DbPool, id: &str) -> String {
    dispatch::page(
        State(db.clone()),
        Extension(viewer()),
        AxPath((id.to_string(), "summary".to_string())),
        Query(HashMap::new()),
    )
    .await
    .unwrap()
    .0
}

const ENTRY: &str = r#"{"logged_on":"2026-10-12","label":"Meter","value":"125.5","notes":"Kitchen"}"#;

#[tokio::test]
async fn add_entry_then_delete_it() {
    let _turn = turn().await;
    let id = fresh_id("lba");
    let db = install(&id).await;

    let (status, text) = call(&id, "POST", "entries", Some(ENTRY)).await;
    assert_eq!(status, 201, "{text}");

    let html = summary_html(&db, &id).await;
    assert!(html.contains(">1</div>"), "one entry counted: {html}");
    assert!(html.contains("2026-10"), "month group shown: {html}");

    let (status, text) = call(&id, "POST", "delete", Some(r#"{"id":"1"}"#)).await;
    assert_eq!(status, 200, "{text}");
    let html = summary_html(&db, &id).await;
    assert!(html.contains(">0</div>"), "entry removed: {html}");
}

#[tokio::test]
async fn bad_input_is_refused_with_a_reason() {
    let _turn = turn().await;
    let id = fresh_id("lbb");
    install(&id).await;

    let cases = [
        (r#"{"logged_on":"2025-02-29","label":"x","value":"1"}"#, "real date"),
        (r#"{"logged_on":"2026-10-12","label":"  ","value":"1"}"#, "label must be"),
        (r#"{"logged_on":"2026-10-12","label":"x","value":""}"#, "value must be a number"),
        (r#"{"logged_on":"2026-10-12","label":"x","value":"abc"}"#, "value must be a number"),
    ];
    for (body, reason) in cases {
        let (status, text) = call(&id, "POST", "entries", Some(body)).await;
        assert_eq!(status, 400, "{body}");
        assert!(text.contains(reason), "{text}");
    }

    let (status, _) = call(&id, "POST", "entries", Some("not json")).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn deleting_a_missing_or_invalid_id_is_refused() {
    let _turn = turn().await;
    let id = fresh_id("lbd");
    install(&id).await;

    let (status, _) = call(&id, "POST", "delete", Some(r#"{"id":"9999"}"#)).await;
    assert_eq!(status, 404);
    let (status, _) = call(&id, "POST", "delete", Some(r#"{"id":"1 OR 1=1"}"#)).await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn quotes_in_labels_are_stored_literally_and_escaped_on_the_page() {
    let _turn = turn().await;
    let id = fresh_id("lbq");
    let db = install(&id).await;
    let body = serde_json::json!({
        "logged_on": "2026-01-03",
        "label": "O'Brien'); DROP TABLE x; --",
        "value": "2"
    })
    .to_string();
    let (status, text) = call(&id, "POST", "entries", Some(&body)).await;
    assert_eq!(status, 201, "{text}");

    let html = summary_html(&db, &id).await;
    assert!(html.contains("O&#39;Brien&#39;); DROP TABLE x; --"), "{html}");
    assert!(!html.contains("O'Brien'); DROP"), "unescaped label on the page");
}

#[tokio::test]
async fn grouped_cards_show_one_row_per_group() {
    let _turn = turn().await;
    let id = fresh_id("lbg");
    let db = install(&id).await;
    for body in [
        r#"{"logged_on":"2026-09-02","label":"Gas","value":"10"}"#,
        r#"{"logged_on":"2026-09-20","label":"Gas","value":"5.25"}"#,
        r#"{"logged_on":"2026-10-01","label":"Power","value":"40"}"#,
    ] {
        let (status, text) = call(&id, "POST", "entries", Some(body)).await;
        assert_eq!(status, 201, "{text}");
    }

    let html = summary_html(&db, &id).await;
    assert!(html.contains("Total by month"), "{html}");
    // Newest month first; September totals 15.25 across two entries.
    assert!(html.contains("<td>2026-10</td><td>40.00</td>"), "{html}");
    assert!(html.contains("<td>2026-09</td><td>15.25</td>"), "{html}");
    assert!(html.contains("<td>Gas</td><td>2</td>"), "{html}");
    assert!(html.contains("<td>Power</td><td>1</td>"), "{html}");
}

#[tokio::test]
async fn empty_grouped_card_says_nothing_yet() {
    let _turn = turn().await;
    let id = fresh_id("lbe");
    let db = install(&id).await;
    let html = summary_html(&db, &id).await;
    assert!(html.contains("Nothing yet."), "{html}");
}
