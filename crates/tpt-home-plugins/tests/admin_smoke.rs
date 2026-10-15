// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The plugin and notification admin APIs through the router: an admin gets
//! data, an ordinary user is refused. The plugin crate links these routes, so
//! this test lives here rather than in tpt-home-api.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::connect_in_memory;

// Referencing the plugin crate keeps its routes linked into this test binary.
const _: &str = tpt_home_plugins::email::EMAIL_CHANNEL;

const ADMIN_APIS: &[&str] = &[
    "/api/plugins",
    "/api/plugins/catalog",
    "/api/notify/channels",
    "/api/notify/email",
];

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

async fn post(
    app: &axum::Router,
    uri: &str,
    cookie: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if !cookie.is_empty() {
        builder = builder.header("cookie", cookie);
    }
    app.clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn get(app: &axum::Router, uri: &str, cookie: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn plugin_and_notification_admin_apis_are_for_admins() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let app = build_router(db, &cfg);

    let setup = post(
        &app,
        "/api/setup",
        "",
        json!({"username": "admin", "password": "hunter2hunter2"}),
    )
    .await;
    let admin = cookie_of(&setup);
    let created = post(
        &app,
        "/api/admin/users",
        &admin,
        json!({"username": "bob", "password": "bob-first-pass", "role": "user"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let login = post(
        &app,
        "/api/auth/login",
        "",
        json!({"username": "bob", "password": "bob-first-pass"}),
    )
    .await;
    let bob_first = cookie_of(&login);
    // The admin-set password must be changed before the session is usable.
    let changed = post(
        &app,
        "/api/auth/change-password",
        &bob_first,
        json!({"current": "bob-first-pass", "new_password": "bob-chosen-pass"}),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::OK);
    let login = post(
        &app,
        "/api/auth/login",
        "",
        json!({"username": "bob", "password": "bob-chosen-pass"}),
    )
    .await;
    let user = cookie_of(&login);

    // Writes are guarded too: a user may not rotate a hook secret or change email settings.
    for (uri, body) in [
        ("/api/plugins/ticker/hooks/door/secret", json!({})),
        ("/api/notify/email/test", json!({})),
    ] {
        let status = post(&app, uri, &user, body).await.status();
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a user is refused a write to {uri}"
        );
    }

    for api in ADMIN_APIS {
        assert_eq!(
            get(&app, api, &admin).await,
            StatusCode::OK,
            "admin gets {api}"
        );
        let refused = get(&app, api, &user).await;
        assert!(
            refused == StatusCode::FORBIDDEN || refused == StatusCode::UNAUTHORIZED,
            "a user is refused {api}: {refused}"
        );
    }
}
