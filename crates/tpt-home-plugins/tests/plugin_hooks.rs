// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Signed public endpoints for plugins (`[[hooks]]`), driven through the real
//! application router. A JavaScript plugin stores each verified body in its
//! own table, so a test can see what the handler received.
//!
//! Like the other plugin tests, these share one database file and take turns.

#![cfg(feature = "js")]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::{DbPool, connect};
use tpt_home_core::webhook;
use tpt_home_plugins::hooks;
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::manifest::Permissions;

const MANIFEST: &str = r#"
[plugin]
id = "ID"
name = "Hook test"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[permissions]
db = true

[[tables]]
name = "events"
columns = [{ name = "note", kind = "text" }]

[[hooks]]
name = "feed"
"#;

/// The handler stores the verified body. The body is quoted by hand, since the
/// sandbox has no bound parameters.
const SCRIPT: &str = r#"
function handle(req) {
  if (req.method === "POST" && req.path === "/hooks/feed") {
    const quoted = "'" + String(req.body).replace(/'/g, "''") + "'";
    const ok = tpt.db.execute("INSERT INTO ID_events (note) VALUES (" + quoted + ")");
    return { status: ok ? 200 : 500, content_type: "application/json", body: "{}" };
  }
  return { status: 404, content_type: "application/json", body: "{}" };
}
"#;

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

/// One test at a time over the shared database file.
async fn turn() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Migrate the server database file, install a fresh copy under `id`, and
/// return the pool and the router built over it.
async fn install(id: &str) -> (DbPool, axum::Router) {
    data_root();
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let cfg = config();
    let db = connect(&cfg.db_path()).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), MANIFEST.replace("ID", id)).unwrap();
    std::fs::write(dir.join("main.js"), SCRIPT.replace("ID", id)).unwrap();
    let grant = Permissions::parse_list("db").unwrap();
    lifecycle::install_from_dir(&db, &dir, true, &grant)
        .await
        .unwrap();

    let router = tpt_home_api::build_router(db.clone(), &cfg);
    (db, router)
}

/// A unique plugin id: lowercase letters and digits, starting with a letter.
fn fresh_id() -> String {
    format!("hk{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Post `body` to a hook with the given signature inputs.
async fn post(
    router: &axum::Router,
    url: &str,
    timestamp: &str,
    signature: &str,
    body: &str,
) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri(url)
        .header("content-type", "application/json")
        .header("x-tpt-timestamp", timestamp)
        .header("x-tpt-signature", signature)
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Create (or rotate) the hook secret the way the admin route does.
async fn secret_for(id: &str, name: &str) -> String {
    let response = hooks::rotate_secret(
        axum::extract::State(connect(&config().db_path()).await.unwrap()),
        axum::extract::Path((id.to_string(), name.to_string())),
    )
    .await
    .unwrap();
    let bytes = to_bytes(
        axum::response::IntoResponse::into_response(response).into_body(),
        1024 * 1024,
    )
    .await
    .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["secret"].as_str().unwrap().to_string()
}

fn url(id: &str, name: &str) -> String {
    format!("/api/plugin-hooks/{id}/{name}")
}

async fn stored_notes(db: &DbPool, id: &str) -> Vec<String> {
    let rows: Vec<(String,)> =
        sqlx::query_as(&format!("SELECT note FROM \"{id}_events\" ORDER BY id"))
            .fetch_all(db)
            .await
            .unwrap();
    rows.into_iter().map(|(note,)| note).collect()
}

#[tokio::test]
async fn a_signed_delivery_reaches_the_plugin_handler() {
    let _turn = turn().await;
    let id = fresh_id();
    let (db, router) = install(&id).await;
    let secret = secret_for(&id, "feed").await;

    let body = "door: front, state: open";
    let ts = now().to_string();
    let sig = webhook::sign(secret.as_bytes(), now(), body.as_bytes());
    let (status, text) = post(&router, &url(&id, "feed"), &ts, &sig, body).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{text}");
    assert_eq!(stored_notes(&db, &id).await, vec![body.to_string()]);

    let (last,): (Option<i64>,) =
        sqlx::query_as("SELECT last_received_at FROM plugin_hooks WHERE plugin_id = ?1")
            .bind(&id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(last.is_some(), "the receipt is recorded");
}

#[tokio::test]
async fn bad_signatures_and_stale_timestamps_are_refused_without_a_call() {
    let _turn = turn().await;
    let id = fresh_id();
    let (db, router) = install(&id).await;
    let secret = secret_for(&id, "feed").await;
    let body = "x";

    let wrong = webhook::sign(b"not the secret", now(), body.as_bytes());
    let (status, _) = post(&router, &url(&id, "feed"), &now().to_string(), &wrong, body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let stale_at = now() - 3_600;
    let stale = webhook::sign(secret.as_bytes(), stale_at, body.as_bytes());
    let (status, _) = post(
        &router,
        &url(&id, "feed"),
        &stale_at.to_string(),
        &stale,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = post(&router, &url(&id, "feed"), "", "", body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "missing headers");

    assert!(
        stored_notes(&db, &id).await.is_empty(),
        "nothing reached the plugin"
    );
}

#[tokio::test]
async fn unknown_hooks_and_plugins_look_the_same_as_bad_signatures() {
    let _turn = turn().await;
    let id = fresh_id();
    let (_db, router) = install(&id).await;
    let secret = secret_for(&id, "feed").await;
    let body = "x";
    let sig = webhook::sign(secret.as_bytes(), now(), body.as_bytes());
    let ts = now().to_string();

    for target in [url(&id, "nope"), url("missing_plugin", "feed")] {
        let (status, _) = post(&router, &target, &ts, &sig, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{target}");
    }
}

#[tokio::test]
async fn a_disabled_plugin_stops_accepting_deliveries() {
    let _turn = turn().await;
    let id = fresh_id();
    let (db, router) = install(&id).await;
    let secret = secret_for(&id, "feed").await;
    lifecycle::disable(&db, &id).await.unwrap();

    let body = "x";
    let sig = webhook::sign(secret.as_bytes(), now(), body.as_bytes());
    let (status, _) = post(&router, &url(&id, "feed"), &now().to_string(), &sig, body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rotating_the_secret_retires_the_old_one() {
    let _turn = turn().await;
    let id = fresh_id();
    let (_db, router) = install(&id).await;
    let old = secret_for(&id, "feed").await;
    let new = secret_for(&id, "feed").await;
    assert_ne!(old, new);

    let body = "x";
    let ts = now().to_string();
    let with_old = webhook::sign(old.as_bytes(), now(), body.as_bytes());
    let (status, _) = post(&router, &url(&id, "feed"), &ts, &with_old, body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "old secret is refused");

    let with_new = webhook::sign(new.as_bytes(), now(), body.as_bytes());
    let (status, _) = post(&router, &url(&id, "feed"), &ts, &with_new, body).await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn uninstalling_removes_the_secret_even_when_data_is_kept() {
    let _turn = turn().await;
    let id = fresh_id();
    let (db, _router) = install(&id).await;
    secret_for(&id, "feed").await;

    lifecycle::uninstall(&db, &id, true).await.unwrap();
    let (remaining,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM plugin_hooks WHERE plugin_id = ?1")
            .bind(&id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn the_secret_route_is_admin_only() {
    let _turn = turn().await;
    let id = fresh_id();
    let (_db, router) = install(&id).await;
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/plugins/{id}/hooks/feed/secret"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "anonymous callers get no secret"
    );
}
