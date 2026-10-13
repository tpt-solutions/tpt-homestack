// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Track CSV import and export through the real router: a round trip is
//! byte-identical, headers are matched case-insensitively, and one bad row
//! saves nothing.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::OnceLock;
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

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

fn json_req(method: &str, uri: &str, body: Value, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn csv_req(method: &str, uri: &str, text: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "text/csv")
        .header("cookie", cookie)
        .body(Body::from(text.to_string()))
        .unwrap()
}

fn get_req(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

async fn authed_router() -> (axum::Router, String) {
    // Routes are collected from linked blueprints, so the crate must be linked.
    tpt_home_bp_track::__link_anchor();
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
            "",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    (app, cookie)
}

async fn create_table(app: &axum::Router, cookie: &str, name: &str) -> i64 {
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/track/tables",
            json!({
                "name": name,
                "fields": [
                    {"name": "label", "kind": "text"},
                    {"name": "amount", "kind": "real"},
                    {"name": "paid", "kind": "bool"}
                ]
            }),
            cookie,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let table: Value = serde_json::from_slice(&bytes).unwrap();
    table["id"].as_i64().unwrap()
}

async fn import(app: &axum::Router, cookie: &str, table: i64, text: &str) -> StatusCode {
    app.clone()
        .oneshot(csv_req(
            "POST",
            &format!("/api/track/tables/{table}/import"),
            text,
            cookie,
        ))
        .await
        .unwrap()
        .status()
}

async fn export(app: &axum::Router, cookie: &str, table: i64) -> String {
    let res = app
        .clone()
        .oneshot(get_req(
            &format!("/api/track/tables/{table}/export.csv"),
            cookie,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn export_then_import_is_byte_identical() {
    let (app, cookie) = authed_router().await;
    let first = create_table(&app, &cookie, "Expenses").await;
    let source = "label,amount,paid\r\ncoffee,3.5,true\r\nbook,12,false\r\n";
    assert_eq!(import(&app, &cookie, first, source).await, StatusCode::CREATED);

    let exported = export(&app, &cookie, first).await;
    assert!(exported.starts_with("label,amount,paid\r\n"), "{exported}");
    assert!(exported.contains("coffee"), "{exported}");

    let second = create_table(&app, &cookie, "Copy").await;
    assert_eq!(
        import(&app, &cookie, second, &exported).await,
        StatusCode::CREATED
    );
    assert_eq!(export(&app, &cookie, second).await, exported);
}

#[tokio::test]
async fn headers_are_case_insensitive_and_a_bom_is_ignored() {
    let (app, cookie) = authed_router().await;
    let table = create_table(&app, &cookie, "Shouting").await;
    let text = "\u{feff}LABEL,Amount,PAID\r\nrent,900,true\r\n";
    assert_eq!(import(&app, &cookie, table, text).await, StatusCode::CREATED);
    assert!(export(&app, &cookie, table).await.contains("rent"));
}

#[tokio::test]
async fn one_bad_row_saves_nothing() {
    let (app, cookie) = authed_router().await;
    let table = create_table(&app, &cookie, "Strict").await;
    let text = "label,amount\r\ngood,1\r\nbad,not-a-number\r\n";
    assert_eq!(
        import(&app, &cookie, table, text).await,
        StatusCode::BAD_REQUEST
    );

    let exported = export(&app, &cookie, table).await;
    assert_eq!(exported, "label,amount,paid\r\n", "no rows were saved");
}

#[tokio::test]
async fn unknown_columns_and_non_utf8_are_refused() {
    let (app, cookie) = authed_router().await;
    let table = create_table(&app, &cookie, "Columns").await;
    assert_eq!(
        import(&app, &cookie, table, "label,colour\r\nx,red\r\n").await,
        StatusCode::BAD_REQUEST
    );

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/track/tables/{table}/import"))
                .header("content-type", "text/csv")
                .header("cookie", cookie.as_str())
                .body(Body::from(vec![0x6c, 0x61, 0xff, 0x0a]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
