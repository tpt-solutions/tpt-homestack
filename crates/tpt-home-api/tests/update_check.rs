// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The admin update check against a local release manifest server: a newer
//! version, the same version, an unreachable server, and checks being off.
//! Nothing is downloaded or installed by these routes.

use std::io::{Read, Write};
use std::net::TcpListener;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

const ADMIN_PW: &str = "hunter2hunter2";

/// Serve `body` to every request on a local port; returns the manifest URL.
fn manifest_server(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/latest.json")
}

fn manifest(version: &str) -> String {
    json!({
        "version": version,
        "url": "https://example.org/tpt-home.zip",
        "sha256": "a".repeat(64),
        "notes": "Bug fixes",
    })
    .to_string()
}

async fn app(update_url: &str) -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        update_url: update_url.to_string(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    (build_router(db, &cfg), dir)
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
                    json!({"username": "admin", "password": ADMIN_PW}).to_string(),
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

async fn update_status(app: &axum::Router, cookie: &str) -> Value {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/update")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn a_newer_release_is_reported_with_its_checksum() {
    let url = manifest_server(manifest("99.0.0"));
    let (app, _dir) = app(&url).await;
    let cookie = admin_cookie(&app).await;
    let v = update_status(&app, &cookie).await;
    assert_eq!(v["configured"], true);
    assert_eq!(v["update_available"], true);
    assert_eq!(v["latest"], "99.0.0");
    assert_eq!(v["download_url"], "https://example.org/tpt-home.zip");
    assert_eq!(v["notes"], "Bug fixes");
}

#[tokio::test]
async fn the_running_version_is_not_offered_again() {
    let url = manifest_server(manifest(tpt_home_core::VERSION));
    let (app, _dir) = app(&url).await;
    let cookie = admin_cookie(&app).await;
    let v = update_status(&app, &cookie).await;
    assert_eq!(v["update_available"], false);
    assert!(v.get("error").is_none());
}

#[tokio::test]
async fn an_unreachable_server_is_reported_not_raised() {
    // Port 1 on loopback is closed; the check fails and says so.
    let (app, _dir) = app("http://127.0.0.1:1/latest.json").await;
    let cookie = admin_cookie(&app).await;
    let v = update_status(&app, &cookie).await;
    assert_eq!(v["configured"], true);
    assert!(
        v["error"].as_str().unwrap().contains("could not reach"),
        "{v}"
    );
}

#[tokio::test]
async fn checks_are_off_until_an_address_is_set() {
    let (app, _dir) = app("").await;
    let cookie = admin_cookie(&app).await;
    let v = update_status(&app, &cookie).await;
    assert_eq!(v["configured"], false);
    assert_eq!(v["current"], tpt_home_core::VERSION);
}

#[tokio::test]
async fn a_plain_http_address_is_refused_before_any_request() {
    let (app, _dir) = app("http://example.org/latest.json").await;
    let cookie = admin_cookie(&app).await;
    let v = update_status(&app, &cookie).await;
    assert!(v["error"].as_str().unwrap().contains("https"), "{v}");
}
