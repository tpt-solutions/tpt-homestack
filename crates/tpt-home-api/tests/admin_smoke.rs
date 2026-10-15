// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Every admin page and read-only admin API, through the router: an admin gets
//! the page or data, and an ordinary user is refused. The plugin and notification
//! routes are covered in tpt-home-plugins (they are linked there). A regression in any
//! route's wiring shows up here.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

const ADMIN_PW: &str = "hunter2hunter2";
const USER_PW: &str = "correct horse battery";

const ADMIN_PAGES: &[&str] = &[
    "/welcome",
    "/dashboard",
    "/settings",
    "/admin/users",
    "/admin/jobs",
    "/admin/status",
    "/admin/plugins",
    "/admin/remote",
];

const ADMIN_APIS: &[&str] = &[
    "/api/admin/users",
    "/api/admin/jobs",
    "/api/admin/health",
    "/api/admin/backups",
    "/api/admin/remote",
    "/api/admin/update",
    "/api/webhooks",
    "/api/settings/server",
    "/api/auth/me",
];

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

async fn get(app: &axum::Router, uri: &str, cookie: &str) -> StatusCode {
    let mut builder = Request::builder().uri(uri);
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie);
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn admin_pages_and_apis_answer_admins_and_refuse_users() {
    let (app, _db, _dir) = app().await;

    let setup = app
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
    let admin = cookie_of(&setup);

    // An ordinary user: created by the admin, then changes the admin-set password.
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/users")
                .header("content-type", "application/json")
                .header("cookie", &admin)
                .body(Body::from(
                    json!({"username": "alice", "password": USER_PW, "role": "user"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username": "alice", "password": USER_PW}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let user = cookie_of(&login);
    let changed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/change-password")
                .header("content-type", "application/json")
                .header("cookie", &user)
                .body(Body::from(
                    json!({"current": USER_PW, "new_password": "alice-chosen-pass"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    // Changing a password ends the sessions that existed before it: sign in again.
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username": "alice", "password": "alice-chosen-pass"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let user = cookie_of(&login);

    for page in ADMIN_PAGES {
        let status = get(&app, page, &admin).await;
        // Until the first-run wizard is finished, an admin's dashboard goes to it.
        let wizard_first = *page == "/dashboard" && status.is_redirection();
        assert!(
            status == StatusCode::OK || wizard_first,
            "admin gets {page}: {status}"
        );
        // The dashboard and settings are for every signed-in user.
        if *page == "/dashboard" || *page == "/settings" {
            continue;
        }
        let as_user = get(&app, page, &user).await;
        assert!(
            as_user.is_redirection() || as_user == StatusCode::FORBIDDEN,
            "a user is refused {page}: {as_user}"
        );
    }
    for api in ADMIN_APIS {
        assert_eq!(
            get(&app, api, &admin).await,
            StatusCode::OK,
            "admin gets {api}"
        );
    }
    // The user-facing API still answers the user, but the admin APIs do not.
    assert_eq!(get(&app, "/api/auth/me", &user).await, StatusCode::OK);
    for api in ADMIN_APIS.iter().filter(|a| a.starts_with("/api/admin/")) {
        let status = get(&app, api, &user).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
            "a user is refused {api}: {status}"
        );
    }
}
