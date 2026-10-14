// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `#[bp_webhook]`: a signed delivery through the real router reaches the
//! handler registered for its webhook name, and a handler that fails does not
//! change what the sender gets back.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};
use tpt_home_core::webhook;
use tpt_home_core::{ApiError, WebhookDelivery};

static SEEN: Mutex<Vec<(i64, String)>> = Mutex::new(Vec::new());
static FAILED: Mutex<Vec<i64>> = Mutex::new(Vec::new());

#[tpt_home_macros::bp_webhook("push-test")]
async fn on_push(_db: DbPool, delivery: WebhookDelivery) -> Result<(), ApiError> {
    SEEN.lock().unwrap().push((delivery.id, delivery.body));
    Ok(())
}

#[tpt_home_macros::bp_webhook("push-fail")]
async fn on_fail(_db: DbPool, delivery: WebhookDelivery) -> Result<(), ApiError> {
    FAILED.lock().unwrap().push(delivery.id);
    Err(ApiError::internal("this handler always fails"))
}

const SECRET: &str = "push-test-secret";

async fn create_webhook(db: &DbPool, name: &str) {
    sqlx::query("INSERT INTO webhooks (name, secret, created_at) VALUES (?1, ?2, ?3)")
        .bind(name)
        .bind(SECRET)
        .bind(now())
        .execute(db)
        .await
        .unwrap();
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn post(app: &axum::Router, name: &str, body: &str) -> StatusCode {
    let ts = now();
    let signature = webhook::sign(SECRET.as_bytes(), ts, body.as_bytes());
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/hooks/{name}"))
                .header("content-type", "application/json")
                .header("x-tpt-timestamp", ts.to_string())
                .header("x-tpt-signature", signature)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn an_accepted_delivery_reaches_its_handler() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    create_webhook(&db, "push-test").await;
    create_webhook(&db, "push-fail").await;
    let app = tpt_home_api::build_router(db.clone(), &cfg);

    assert_eq!(
        post(&app, "push-test", "door=open").await,
        StatusCode::ACCEPTED
    );
    let arrived = wait_for(|| {
        SEEN.lock()
            .unwrap()
            .iter()
            .any(|(_, body)| body == "door=open")
    })
    .await;
    assert!(arrived, "the handler saw the delivery body");

    // A failing handler is logged and contained; the sender still gets a 202.
    assert_eq!(post(&app, "push-fail", "x").await, StatusCode::ACCEPTED);
    assert!(wait_for(|| !FAILED.lock().unwrap().is_empty()).await);

    // A signature from the wrong secret is refused before any handler runs.
    let before = SEEN.lock().unwrap().len();
    let ts = now();
    let bad = webhook::sign(b"wrong", ts, b"forged");
    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/hooks/push-test")
                .header("content-type", "application/json")
                .header("x-tpt-timestamp", ts.to_string())
                .header("x-tpt-signature", bad)
                .body(Body::from("forged"))
                .unwrap(),
        )
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(SEEN.lock().unwrap().len(), before);
}
