// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Track blueprint end-to-end: user-defined tables + rows through the real
//! router, proving the dynamic-SQL safety model.

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

#[test]
fn blueprint_is_registered() {
    let bp = tpt_home_bp_track::TrackBlueprint;
    assert_eq!(bp.id(), "track");
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

async fn authed_router() -> (axum::Router, String) {
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
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    (app, cookie.split(';').next().unwrap().to_string())
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn dynamic_tables_crud_and_filters() {
    let (app, cookie) = authed_router().await;

    // Create a table with typed fields.
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/track/tables",
            json!({
                "name": "Expenses",
                "fields": [
                    {"name": "label", "kind": "text"},
                    {"name": "amount", "kind": "real"},
                    {"name": "paid", "kind": "bool"}
                ]
            }),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let table = body_json(res).await;
    let table_id = table["id"].as_i64().unwrap();
    assert_eq!(table["fields"].as_array().unwrap().len(), 3);

    // Kind validation: wrong type rejected.
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            &format!("/api/track/tables/{table_id}/rows"),
            json!({"label": "coffee", "amount": "not-a-number"}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Insert rows.
    for (label, amount, paid) in [("coffee", 3.5, true), ("book", 12.0, false)] {
        let res = app
            .clone()
            .oneshot(json_req(
                "POST",
                &format!("/api/track/tables/{table_id}/rows"),
                json!({"label": label, "amount": amount, "paid": paid}),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    // Filter by text field.
    let res = app
        .clone()
        .oneshot(json_req(
            "GET",
            &format!("/api/track/tables/{table_id}/rows?label=coffee"),
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    let rows = body_json(res).await;
    assert_eq!(rows.as_array().unwrap().len(), 1);
    // Rows are keyed by the internal f_<field-id> columns.
    let label_col = format!("f_{}", table["fields"][0]["id"].as_i64().unwrap());
    assert_eq!(rows[0][&label_col], json!("coffee"));

    // Update a row.
    let res = app
        .clone()
        .oneshot(json_req(
            "PATCH",
            &format!("/api/track/tables/{table_id}/rows/2"),
            json!({"paid": true}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Delete a row, then the table.
    let res = app
        .clone()
        .oneshot(json_req(
            "DELETE",
            &format!("/api/track/tables/{table_id}/rows/1"),
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .clone()
        .oneshot(json_req(
            "DELETE",
            &format!("/api/track/tables/{table_id}"),
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .oneshot(json_req(
            "GET",
            &format!("/api/track/tables/{table_id}/rows"),
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn table_creation_validation() {
    let (app, cookie) = authed_router().await;

    // Unknown kind rejected.
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/track/tables",
            json!({"name": "bad", "fields": [{"name": "x", "kind": "blob"}]}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // No fields rejected; empty name rejected.
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/track/tables",
            json!({"name": "bad", "fields": []}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = app
        .oneshot(json_req(
            "POST",
            "/api/track/tables",
            json!({"name": "  ", "fields": [{"name": "x", "kind": "text"}]}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
