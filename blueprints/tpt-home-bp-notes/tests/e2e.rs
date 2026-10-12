// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Notes blueprint end-to-end over the real router: proves the macros
//! registered migration, routes and UI page, and that FTS5 search works
//! through HTTP.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::OnceLock;
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::Blueprint;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

// Reference the blueprint so its objects are linked into the test binary
// (inventory entries live in its rlib).
#[test]
fn blueprint_is_registered() {
    let bp = tpt_home_bp_notes::NotesBlueprint;
    assert_eq!(bp.id(), "notes");
    assert_eq!(bp.name(), "Notes");
}

fn test_config() -> &'static Config {
    static CFG: OnceLock<Config> = OnceLock::new();
    CFG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        std::mem::forget(dir);
        Config {
            data_dir,
            ..Config::default()
        }
    })
}

async fn authed_router() -> axum::Router {
    tpt_home_core::config::set_global_for_tests(test_config().clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let app = build_router(db, test_config());

    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/setup",
            json!({"username": "admin", "password": "hunter2hunter2"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    app
}

fn json_req(method: &str, uri: &str, body: Value, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        b = b.header("cookie", c);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

async fn login(app: &axum::Router) -> String {
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/auth/login",
            json!({"username": "admin", "password": "hunter2hunter2"}),
            None,
        ))
        .await
        .unwrap();
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    cookie.split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn notes_crud_and_search_over_http() {
    let app = authed_router().await;
    let cookie = login(&app).await;

    // Blueprint API routes were injected by #[bp_route].
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/notes",
            json!({"title": "Home network", "body": "router **password** is correct horse", "tags": ["infra", "secret"]}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let note = {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    let id = note["id"].as_i64().unwrap();
    assert_eq!(note["tags"], json!(["infra", "secret"]));

    // FTS5 search (injected triggers keep the index in sync).
    let res = app
        .clone()
        .oneshot(json_req(
            "GET",
            "/api/notes?q=horse",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    let hits: Vec<Value> = {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], json!(id));

    // Tag filter.
    let res = app
        .clone()
        .oneshot(json_req(
            "GET",
            "/api/notes?tag=infra",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    let hits: Vec<Value> = {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(hits.len(), 1);

    // Sanitized markdown render endpoint.
    let _ = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/notes",
            json!({"title": "evil", "body": "<script>alert(1)</script>[x](https://ok.example)"}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/notes/2/render")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(!html.contains("<script>"), "XSS must be escaped: {html}");
    assert!(html.contains("https://ok.example"));

    // Delete + 404 afterwards.
    let res = app
        .clone()
        .oneshot(json_req(
            "DELETE",
            "/api/notes/1",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .oneshot(json_req("GET", "/api/notes/1", Value::Null, Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn notes_ui_page_in_sidebar() {
    let app = authed_router().await;
    let cookie = login(&app).await;

    // #[bp_ui_route] registered the page.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/notes")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("Notes"));
    assert!(html.contains("/notes"));

    // Dashboard lists the blueprint.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.contains("Notes"),
        "dashboard should show the blueprint card"
    );
}
