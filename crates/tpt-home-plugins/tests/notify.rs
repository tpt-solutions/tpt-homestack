// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Notification channels: `tpt.notify` from a JavaScript plugin, against a
//! local device that records what it receives, and the admin routes'
//! validation. The channel row lives in a real migrated database file, since
//! the host reads it through its own connection.

#![cfg(feature = "js")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path as AxPath, State};
use tpt_home_core::db::{DbPool, connect};
use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::notify::{self, ChannelBody};
use tpt_home_plugins::runtime::PluginRequest;
use tpt_home_plugins::script::{self, ScriptSource};

/// Serve `count` requests on 127.0.0.1 with the given status, recording each
/// raw request.
fn device(count: usize, status: u16) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for _ in 0..count {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            log.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).into_owned());
            let reply = format!("HTTP/1.0 {status} X\r\nContent-Length: 0\r\n\r\n");
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (port, seen)
}

/// A migrated database file with one channel named `alerts` at `url`.
async fn database(url: &str) -> (tempfile::TempDir, PathBuf, DbPool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tpt.db");
    let db = connect(&path).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    sqlx::query(
        "INSERT INTO notification_channels (name, url, created_at) VALUES ('alerts', ?1, 0)",
    )
    .bind(url)
    .execute(&db)
    .await
    .unwrap();
    (dir, path, db)
}

/// Run a script whose manifest asks for `notify` to `alerts`, with the
/// approval `approved`. Returns the body the script produced.
fn run(source: &str, approved: Permissions, db_path: PathBuf) -> String {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let script = ScriptSource::new(source).expect("script parses");
            let manifest = manifest::parse(
                r#"
[plugin]
id = "alerter"
name = "Alerter"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[permissions]
notify = true
notify_channels = ["alerts"]

[limits]
max_fuel = 50000000
max_memory_mb = 16
max_millis = 5000
"#,
            )
            .expect("manifest");
            let effective = manifest.permissions.intersect(&approved);
            let dir = tempfile::tempdir().unwrap();
            script::call_script(
                &script,
                &manifest,
                &PluginRequest {
                    method: "GET".into(),
                    path: "/".into(),
                    query: String::new(),
                    body: None,
                    user: None,
                },
                "alerter",
                &effective,
                &db_path,
                dir.path(),
            )
            .expect("script should succeed")
            .body
        })
        .unwrap()
        .join()
        .unwrap()
}

fn approved() -> Permissions {
    Permissions::parse_list("notify").unwrap()
}

/// The stored delivery state of the `alerts` channel.
async fn delivery_state(db: &DbPool) -> (Option<i64>, Option<String>) {
    sqlx::query_as(
        "SELECT last_sent_at, last_error FROM notification_channels WHERE name = 'alerts'",
    )
    .fetch_one(db)
    .await
    .unwrap()
}

const SEND: &str = r#"function handle() {
    const sent = tpt.notify("alerts", "door open");
    return { status: 200, body: String(sent) };
}"#;

#[tokio::test]
async fn a_message_is_posted_as_json_and_the_success_is_recorded() {
    let (port, seen) = device(1, 204);
    let (_dir, path, db) = database(&format!("http://127.0.0.1:{port}/hook")).await;

    let body = run(SEND, approved(), path);
    assert_eq!(body, "true");

    let request = seen.lock().unwrap()[0].clone();
    assert!(
        request.starts_with("POST /hook HTTP/1.0\r\n"),
        "{}",
        request
    );
    assert!(
        request.contains("Content-Type: application/json\r\n"),
        "{}",
        request
    );
    assert!(request.ends_with(r#"{"text":"door open"}"#), "{}", request);
    let (sent_at, error) = delivery_state(&db).await;
    assert!(sent_at.is_some());
    assert_eq!(error, None);
}

#[tokio::test]
async fn a_non_2xx_answer_is_false_and_is_recorded() {
    let (port, _seen) = device(1, 500);
    let (_dir, path, db) = database(&format!("http://127.0.0.1:{port}/hook")).await;

    assert_eq!(run(SEND, approved(), path), "false");
    let (_, error) = delivery_state(&db).await;
    assert_eq!(error.as_deref(), Some("status 500"));
}

#[tokio::test]
async fn an_unapproved_plugin_sends_nothing() {
    let (port, seen) = device(1, 200);
    let (_dir, path, _db) = database(&format!("http://127.0.0.1:{port}/hook")).await;

    assert_eq!(run(SEND, Permissions::default(), path), "false");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn undeclared_channels_and_missing_channels_are_refused() {
    let (port, seen) = device(1, 200);
    let (_dir, path, _db) = database(&format!("http://127.0.0.1:{port}/hook")).await;

    let undeclared = run(
        r#"function handle() {
            return { status: 200, body: String(tpt.notify("other", "x")) };
        }"#,
        approved(),
        path.clone(),
    );
    assert_eq!(undeclared, "false", "a channel the manifest does not name");

    // Declared in the manifest, but the admin never created it.
    let (_dir2, missing_path, _db2) = database(&format!("http://127.0.0.1:{port}/hook")).await;
    sqlx::query("DELETE FROM notification_channels")
        .execute(&connect(&missing_path).await.unwrap())
        .await
        .unwrap();
    assert_eq!(run(SEND, approved(), missing_path), "false");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn overlong_messages_are_refused_before_any_send() {
    let (port, seen) = device(1, 200);
    let (_dir, path, _db) = database(&format!("http://127.0.0.1:{port}/hook")).await;
    let long = "x".repeat(2_001);
    let body = run(
        &format!(
            r#"function handle() {{
                return {{ status: 200, body: String(tpt.notify("alerts", "{long}")) }};
            }}"#
        ),
        approved(),
        path,
    );
    assert_eq!(body, "false");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_admin_routes_refuse_bad_names_and_urls() {
    let (_dir, _path, db) = database("http://127.0.0.1:9/hook").await;
    let save = |name: &str, url: &str| {
        notify::save_channel(
            State(db.clone()),
            Json(ChannelBody {
                name: name.into(),
                url: url.into(),
            }),
        )
    };
    assert!(save("Bad Name", "https://example.com/x").await.is_err());
    assert!(save("ok_name", "javascript:alert(1)").await.is_err());
    assert!(save("ok_name", "ftp://example.com/x").await.is_err());
    assert!(save("ok_name", "https://example.com/a b").await.is_err());
    assert!(save("ok_name", "https://").await.is_err());
    assert!(save("ok_name", "https://example.com/hook").await.is_ok());
}

#[tokio::test]
async fn channels_list_without_urls_and_delete_cleanly() {
    let (_dir, _path, db) = database("https://example.com/secret-token").await;
    let listed = notify::list_channels(State(db.clone())).await.unwrap();
    let bytes = axum::body::to_bytes(
        axum::response::IntoResponse::into_response(listed).into_body(),
        64 * 1024,
    )
    .await
    .unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(text.contains("\"alerts\""), "{text}");
    assert!(
        !text.contains("secret-token"),
        "the URL is never listed: {text}"
    );

    assert!(
        notify::delete_channel(State(db.clone()), AxPath("alerts".into()))
            .await
            .is_ok()
    );
    assert!(
        notify::delete_channel(State(db), AxPath("alerts".into()))
            .await
            .is_err()
    );
}
