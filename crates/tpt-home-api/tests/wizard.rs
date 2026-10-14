// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The first-run wizard through the real router: the redirect for admins who
//! have not finished it, server settings (admin-only writes, validation), the
//! wizard page, and finishing.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

const ADMIN_PW: &str = "hunter2hunter2";
const USER_PW: &str = "correct horse battery";

async fn app() -> (axum::Router, Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    (build_router(db, &cfg), cfg, dir)
}

async fn send(app: &axum::Router, req: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(req).await.unwrap()
}

async fn body_bytes(res: axum::response::Response) -> Vec<u8> {
    res.into_body().collect().await.unwrap().to_bytes().to_vec()
}

async fn body_json(res: axum::response::Response) -> Value {
    serde_json::from_slice(&body_bytes(res).await).unwrap_or(Value::Null)
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

fn get(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
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

async fn user_cookie(app: &axum::Router, admin: &str) -> String {
    let res = send(
        app,
        json_req(
            "POST",
            "/api/admin/users",
            json!({"username": "alice", "password": USER_PW, "role": "user"}),
            admin,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let res = send(
        app,
        json_req(
            "POST",
            "/api/auth/login",
            json!({"username": "alice", "password": USER_PW}),
            "",
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    // An admin set this password, so alice chooses her own before use.
    let cookie = cookie_from(&res);
    let res = send(
        app,
        json_req(
            "POST",
            "/api/auth/change-password",
            json!({"current": USER_PW, "new_password": "alice-chosen-pass"}),
            &cookie,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let res = send(
        app,
        json_req(
            "POST",
            "/api/auth/login",
            json!({"username": "alice", "password": "alice-chosen-pass"}),
            "",
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    cookie_from(&res)
}

#[tokio::test]
async fn admins_are_sent_to_the_wizard_until_it_is_finished() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;

    let res = send(&app, get("/dashboard", &admin)).await;
    assert!(res.status().is_redirection());
    assert_eq!(res.headers()["location"], "/welcome");

    let res = send(
        &app,
        json_req(
            "PUT",
            "/api/admin/server",
            json!({"wizard_done": true}),
            &admin,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = send(&app, get("/dashboard", &admin)).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn ordinary_users_never_see_the_wizard_redirect() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;
    let user = user_cookie(&app, &admin).await;
    let res = send(&app, get("/dashboard", &user)).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn settings_default_then_follow_changes() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;

    let s = body_json(send(&app, get("/api/settings/server", &admin)).await).await;
    assert_eq!(s["name"], "homestack");
    assert_eq!(s["wizard_done"], false);

    let res = send(
        &app,
        json_req(
            "PUT",
            "/api/admin/server",
            json!({"name": "  Hall House "}),
            &admin,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["name"], "Hall House");

    let s = body_json(send(&app, get("/api/settings/server", &admin)).await).await;
    assert_eq!(s["name"], "Hall House");
}

#[tokio::test]
async fn invalid_names_are_refused_and_nothing_changes() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;
    for bad in ["", "   ", "tab\tname", &"x".repeat(65)] {
        let res = send(
            &app,
            json_req("PUT", "/api/admin/server", json!({"name": bad}), &admin),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "name {bad:?}");
    }
    let s = body_json(send(&app, get("/api/settings/server", &admin)).await).await;
    assert_eq!(s["name"], "homestack");
}

#[tokio::test]
async fn only_admins_can_change_settings_but_anyone_signed_in_can_read_them() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;
    let user = user_cookie(&app, &admin).await;

    let res = send(
        &app,
        json_req("PUT", "/api/admin/server", json!({"name": "Mine"}), &user),
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let res = send(&app, get("/api/settings/server", &user)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["name"], "homestack");
}

#[tokio::test]
async fn the_wizard_page_shows_steps_and_escapes_the_name() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;
    send(
        &app,
        json_req(
            "PUT",
            "/api/admin/server",
            json!({"name": "<b>Hall</b>"}),
            &admin,
        ),
    )
    .await;

    let res = send(&app, get("/welcome", &admin)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let html = String::from_utf8(body_bytes(res).await).unwrap();
    assert!(html.contains("Name this server"));
    assert!(html.contains("Family accounts"));
    assert!(html.contains("Connect from anywhere"));
    assert!(html.contains("catalog_url"));
    assert!(
        html.contains("&lt;b&gt;Hall&lt;/b&gt;"),
        "name must be escaped"
    );
    assert!(!html.contains("<b>Hall</b>"));
    assert!(html.contains("admin")); // the signed-in admin is listed
    // The CSP blocks inline event attributes, so the page must not use them.
    assert!(!html.contains(" onsubmit=") && !html.contains(" onclick="));
}

#[tokio::test]
async fn the_wizard_page_is_admin_only() {
    let (app, _cfg, _dir) = app().await;
    let admin = admin_cookie(&app).await;
    let user = user_cookie(&app, &admin).await;
    let res = send(&app, get("/welcome", &user)).await;
    assert!(res.status().is_redirection());
    assert_eq!(res.headers()["location"], "/dashboard");
}
