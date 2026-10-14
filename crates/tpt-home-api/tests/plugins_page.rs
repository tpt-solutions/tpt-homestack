// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The admin plugins page through the real router: who may open it, what it
//! shows, and that the approval choices it offers match what the install
//! endpoint accepts.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
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

async fn get_page(app: &axum::Router, uri: &str, cookie: &str) -> (StatusCode, String) {
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
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn admin_sees_the_plugins_page_with_plain_approvals() {
    let (app, _dir) = app().await;
    let cookie = admin_cookie(&app).await;
    let (status, body) = get_page(&app, "/admin/plugins", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Install a plugin"));
    assert!(body.contains("Admin · Plugins"));
    assert!(body.contains("/admin/plugins"), "sidebar links the page");
    for key in tpt_home_ui::CAPABILITY_KEYS {
        let label = tpt_home_ui::capability_label(key);
        assert!(
            body.contains(&format!("value=\"{key}\" checked")),
            "box for {key}"
        );
        assert!(body.contains(label), "plain label for {key}");
    }
    // The approval field is comma-separated, so the script must join the
    // boxes into one value; check that the install endpoint is the target.
    assert!(body.contains("/api/plugins/install"));
}

#[tokio::test]
async fn plugins_page_is_admin_only() {
    let (app, _dir) = app().await;
    let (status, _) = get_page(&app, "/admin/plugins", "").await;
    assert!(status.is_redirection(), "anonymous visitors are sent away");
    let admin = admin_cookie(&app).await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/users")
                .header("content-type", "application/json")
                .header("cookie", &admin)
                .body(Body::from(
                    json!({"username": "bob", "password": "bob-password-1", "role": "user"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username": "bob", "password": "bob-password-1"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let user_cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    // An admin set bob's password, so he must choose his own before the
    // session is usable at all.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/change-password")
                .header("content-type", "application/json")
                .header("cookie", &user_cookie)
                .body(Body::from(
                    json!({"current": "bob-password-1", "new_password": "bob-chosen-pass-2"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let (status, _) = get_page(&app, "/admin/plugins", &user_cookie).await;
    assert!(
        status.is_redirection(),
        "a user is sent to the dashboard, got {status}"
    );
}
