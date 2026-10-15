// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Email alerts against a fake SMTP server on loopback: saving the account,
//! the test send, the reserved `email` channel name, and that the password is
//! never returned by the API.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

// Referencing the plugin crate keeps its routes linked into this test binary.
const _: &str = tpt_home_plugins::email::EMAIL_CHANNEL;

/// A minimal SMTP server: accepts one message per connection and keeps the
/// data it received. Returns its port.
fn fake_smtp(received: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let _ = writer.write_all(b"220 fake ready\r\n");
            let mut in_data = false;
            let mut data = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        received.lock().unwrap().push(std::mem::take(&mut data));
                        let _ = writer.write_all(b"250 queued\r\n");
                    } else {
                        data.push_str(&line);
                    }
                    continue;
                }
                let upper = line.to_ascii_uppercase();
                let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    b"250 fake\r\n"
                } else if upper.starts_with("DATA") {
                    in_data = true;
                    b"354 send it\r\n"
                } else if upper.starts_with("QUIT") {
                    let _ = writer.write_all(b"221 bye\r\n");
                    break;
                } else {
                    b"250 ok\r\n"
                };
                let _ = writer.write_all(reply);
            }
        }
    });
    port
}

async fn app() -> (axum::Router, DbPool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    (build_router(db.clone(), &cfg), db, dir)
}

async fn admin_cookie(app: &axum::Router) -> String {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username": "admin", "password": "hunter2hunter2"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    res.headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("content-type", "application/json");
    let payload = match body {
        Some(v) => Body::from(v.to_string()),
        None => Body::empty(),
    };
    let res = app
        .clone()
        .oneshot(builder.body(payload).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn email_alerts_send_through_the_saved_account_and_hide_the_password() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let port = fake_smtp(received.clone());
    let (app, _db, _dir) = app().await;
    let cookie = admin_cookie(&app).await;

    let (status, body) = call(&app, "GET", "/api/notify/email", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["configured"],
        false
    );

    // Saving with a password the API must not echo back.
    let saved = json!({
        "host": "127.0.0.1",
        "port": port,
        "security": "none",
        "username": "",
        "password": "super-secret-pw",
        "from": "homestack@example.org",
        "to": "family@example.org",
    });
    let (status, _) = call(&app, "PUT", "/api/notify/email", &cookie, Some(saved)).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = call(&app, "GET", "/api/notify/email", &cookie, None).await;
    assert!(
        !body.contains("super-secret-pw"),
        "the password is never returned"
    );
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["password_set"], true);
    assert_eq!(v["to"], "family@example.org");

    // The reserved channel name cannot be set or deleted through the channel API.
    let (status, _) = call(
        &app,
        "POST",
        "/api/notify/channels",
        &cookie,
        Some(json!({"name": "email", "url": "https://example.org/hook"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, channels) = call(&app, "GET", "/api/notify/channels", &cookie, None).await;
    assert!(channels.contains("\"email\""), "the channel exists");
    assert!(
        !channels.contains("mailto:configured"),
        "its URL is never listed"
    );

    // A test send reaches the fake server with the message text.
    let (status, body) = call(&app, "POST", "/api/notify/email/test", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let messages = received.lock().unwrap().clone();
    assert_eq!(messages.len(), 1, "one message delivered");
    assert!(messages[0].contains("Subject: tpt-homestack test"));
    assert!(messages[0].contains("If you can read it"));
    assert!(messages[0].contains("family@example.org"));

    // Invalid settings are refused and nothing is saved.
    let (status, _) = call(
        &app,
        "PUT",
        "/api/notify/email",
        &cookie,
        Some(json!({"host": "127.0.0.1", "port": port, "security": "none", "from": "nope", "to": "family@example.org"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Removing the account removes its channel.
    let (status, _) = call(&app, "DELETE", "/api/notify/email", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, channels) = call(&app, "GET", "/api/notify/channels", &cookie, None).await;
    assert!(!channels.contains("\"email\""));
}
