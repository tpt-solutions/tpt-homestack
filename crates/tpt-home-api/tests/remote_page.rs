// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The admin Remote access page and its status API through the real router:
//! who may read them, and the shape of what they report.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

const ADMIN_PW: &str = "hunter2hunter2";

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

fn cookie_of(res: &axum::response::Response) -> String {
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
    cookie_of(&res)
}

async fn get(app: &axum::Router, uri: &str, cookie: &str) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().uri(uri);
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    (
        status,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

#[tokio::test]
async fn the_remote_page_is_admin_only_and_linked_from_the_sidebar() {
    let (app, _dir) = app().await;
    let (status, _) = get(&app, "/admin/remote", "").await;
    assert!(status.is_redirection());

    let admin = admin_cookie(&app).await;
    let (status, body) = get(&app, "/admin/remote", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Connect from anywhere"));
    assert!(html.contains("/api/admin/remote"));
    assert!(html.contains("Admin · Remote access"));
}

#[tokio::test]
async fn the_status_api_reports_config_detection_and_steps() {
    let (app, _dir) = app().await;
    let admin = admin_cookie(&app).await;

    let (status, body) = get(&app, "/api/admin/remote", &admin).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["config"]["host"], "127.0.0.1");
    assert_eq!(v["config"]["exposed"], false);
    assert!(v["detected"]["cloudflared"].is_boolean());
    assert!(v["detected"]["netbird"].is_boolean());
    let steps = v["steps"].as_array().unwrap();
    let providers: Vec<&str> = steps
        .iter()
        .map(|s| s["provider"].as_str().unwrap())
        .collect();
    assert_eq!(providers, ["tailscale", "cloudflare", "netbird"]);
    assert!(
        steps
            .iter()
            .all(|s| matches!(s["stage"].as_str(), Some("done" | "todo" | "blocked")))
    );
}

#[tokio::test]
async fn anonymous_callers_get_no_remote_status() {
    let (app, _dir) = app().await;
    let _ = admin_cookie(&app).await;
    let (status, _) = get(&app, "/api/admin/remote", "").await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "got {status}"
    );
}
