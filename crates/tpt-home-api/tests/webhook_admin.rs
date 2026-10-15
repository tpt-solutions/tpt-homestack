// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Webhook administration through the router: create, duplicate and invalid
//! names, listing without secrets, deliveries after a signed post, deletion,
//! and admin-only access.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};
use tpt_home_core::webhook;

async fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    (build_router(db, &cfg), dir)
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("content-type", "application/json");
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let res = app
        .clone()
        .oneshot(builder.body(Body::from(payload)).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn webhooks_are_created_listed_received_and_deleted_by_admins_only() {
    let (app, _dir) = app().await;
    let setup = app
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
    let cookie = setup
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // Create: the secret is shown once, in the create reply.
    let (status, created) = send(
        &app,
        "POST",
        "/api/webhooks",
        &cookie,
        Some(json!({"name": "door-sensor"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let secret = created["secret"]
        .as_str()
        .expect("secret shown once")
        .to_string();
    let id = created["id"].as_i64().unwrap();

    // Duplicate names conflict; malformed names are refused.
    let (status, _) = send(
        &app,
        "POST",
        "/api/webhooks",
        &cookie,
        Some(json!({"name": "door-sensor"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = send(
        &app,
        "POST",
        "/api/webhooks",
        &cookie,
        Some(json!({"name": "Bad Name!"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The list never shows the secret.
    let (status, listed) = send(&app, "GET", "/api/webhooks", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !listed.to_string().contains(&secret),
        "the secret is never listed"
    );

    // A signed post is accepted and appears in the deliveries.
    let body = r#"{"state":"open"}"#;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let sig = webhook::sign(secret.as_bytes(), ts, body.as_bytes());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/hooks/door-sensor")
                .header("content-type", "application/json")
                .header("x-tpt-timestamp", ts.to_string())
                .header("x-tpt-signature", sig)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let (status, deliveries) = send(
        &app,
        "GET",
        &format!("/api/webhooks/{id}/deliveries"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(deliveries.to_string().contains("open"), "{deliveries}");

    // Delete removes it; a second delete finds nothing.
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/webhooks/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/webhooks/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Anonymous callers cannot manage webhooks.
    let (status, _) = send(&app, "GET", "/api/webhooks", "", None).await;
    assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
}
