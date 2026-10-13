// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Safe defaults through the real router: an admin-set password must be
//! replaced before the app can be used, and the exposed-server warning shows
//! on every page when the server listens beyond this machine.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

const ADMIN_PW: &str = "hunter2hunter2";
const SET_BY_ADMIN: &str = "temporary-pass-1";
const CHOSEN: &str = "a-password-of-my-own";

async fn app(host: &str) -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        host: host.to_string(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    (build_router(db, &cfg), dir)
}

async fn send(app: &axum::Router, req: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(req).await.unwrap()
}

async fn body_text(res: axum::response::Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn json_req(method: &str, uri: &str, body: Value, cookie: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

fn get(uri: &str, cookie: &str, accept_html: bool) -> Request<Body> {
    let mut builder = Request::builder().uri(uri).header("cookie", cookie);
    if accept_html {
        builder = builder.header("accept", "text/html,application/xhtml+xml");
    }
    builder.body(Body::empty()).unwrap()
}

fn cookie_from(res: &axum::response::Response) -> String {
    res.headers()
        .get("set-cookie")
        .expect("set-cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn admin_cookie(app: &axum::Router) -> String {
    let res = send(
        app,
        json_req(
            "POST",
            "/api/setup",
            json!({"username": "admin", "password": ADMIN_PW}),
            "",
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    cookie_from(&res)
}

async fn login(app: &axum::Router, username: &str, password: &str) -> String {
    let res = send(
        app,
        json_req(
            "POST",
            "/api/auth/login",
            json!({"username": username, "password": password}),
            "",
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK, "login as {username}");
    cookie_from(&res)
}

/// An admin creates a second admin with a password of their choosing. The
/// new account is flagged: it must choose its own password.
async fn flagged_admin(app: &axum::Router, admin: &str) -> String {
    let res = send(
        app,
        json_req(
            "POST",
            "/api/admin/users",
            json!({"username": "carol", "password": SET_BY_ADMIN, "role": "admin"}),
            admin,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    login(app, "carol", SET_BY_ADMIN).await
}

#[tokio::test]
async fn a_password_set_by_an_admin_must_be_replaced_before_use() {
    let (app, _dir) = app("127.0.0.1").await;
    let admin = admin_cookie(&app).await;
    let carol = flagged_admin(&app, &admin).await;

    // Pages go to Settings, and API calls are refused, except the calls that
    // change the password and sign out.
    let res = send(&app, get("/dashboard", &carol, true)).await;
    assert!(res.status().is_redirection());
    assert_eq!(res.headers()["location"], "/settings");

    let res = send(&app, get("/api/admin/health", &carol, false)).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(body_text(res).await.contains("password change required"));

    let res = send(&app, get("/api/auth/me", &carol, false)).await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = send(&app, get("/settings", &carol, true)).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn changing_the_password_lifts_the_requirement() {
    let (app, _dir) = app("127.0.0.1").await;
    let admin = admin_cookie(&app).await;
    let carol = flagged_admin(&app, &admin).await;

    let res = send(
        &app,
        json_req(
            "POST",
            "/api/auth/change-password",
            json!({"current": SET_BY_ADMIN, "new_password": CHOSEN}),
            &carol,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    // The change revokes sessions, so sign in again with the new password.
    let carol = login(&app, "carol", CHOSEN).await;
    let res = send(&app, get("/api/admin/health", &carol, false)).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_pending_change_shows_a_notice_on_settings() {
    let (app, _dir) = app("127.0.0.1").await;
    let admin = admin_cookie(&app).await;
    let carol = flagged_admin(&app, &admin).await;
    let html = body_text(send(&app, get("/settings", &carol, true)).await).await;
    assert!(
        html.contains("An administrator set your password"),
        "no notice"
    );
    // The notice goes inside the page, after the main tag.
    let main = html.find("<main class=\"main\">").unwrap();
    let notice = html.find("An administrator set your password").unwrap();
    assert!(notice > main);
}

#[tokio::test]
async fn the_first_admin_chooses_their_own_password_and_is_not_flagged() {
    let (app, _dir) = app("127.0.0.1").await;
    let admin = admin_cookie(&app).await;
    let res = send(&app, get("/api/admin/health", &admin, false)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let html = body_text(send(&app, get("/settings", &admin, true)).await).await;
    assert!(!html.contains("An administrator set your password"));
}

#[tokio::test]
async fn exposed_server_shows_a_warning_on_every_page() {
    let (app, _dir) = app("0.0.0.0").await;
    let admin = admin_cookie(&app).await;
    let html = body_text(send(&app, get("/settings", &admin, true)).await).await;
    assert!(html.contains("Exposed to the network"), "warning missing");
}

#[tokio::test]
async fn loopback_server_shows_no_exposure_warning() {
    let (app, _dir) = app("127.0.0.1").await;
    let admin = admin_cookie(&app).await;
    let html = body_text(send(&app, get("/settings", &admin, true)).await).await;
    assert!(!html.contains("Exposed to the network"));
}
