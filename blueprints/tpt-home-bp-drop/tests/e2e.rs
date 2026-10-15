// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Drop blueprint end-to-end: metadata CRUD and public share guarding over
//! the real router. (Streaming upload is covered by the multipart path in
//! `api::upload`; the chunk loop is exercised manually in the smoke test.)

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
    let bp = tpt_home_bp_drop::DropBlueprint;
    assert_eq!(bp.id(), "drop");
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

async fn authed_router() -> (axum::Router, String, DbPool) {
    tpt_home_core::config::set_global_for_tests(test_config().clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let app = build_router(db.clone(), test_config());
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
    (app, cookie.split(';').next().unwrap().to_string(), db)
}

#[tokio::test]
async fn drop_list_and_share_guards() {
    let (app, cookie, _db) = authed_router().await;

    // Empty list.
    let res = app
        .clone()
        .oneshot(json_req("GET", "/api/drop", Value::Null, Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!([]));

    // Unknown share token → 404 (no file, no leak).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/drop/shared/no-such-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Unauthenticated download of a drop → 401 (route is protected).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/drop/1/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Manual cleanup endpoint works (removes nothing).
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/drop/cleanup",
            json!({}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["removed"], json!(0));
}

#[tokio::test]
async fn share_link_requires_password_when_set() {
    let (app, cookie, db) = authed_router().await;

    // Insert a drop row + tiny file directly (upload streaming itself is
    // exercised via multipart in the smoke test).
    let uploads = test_config().uploads_dir().join("drop");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("secret.txt"), "hello drop").unwrap();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO drop_files (filename, stored_path, size, mime, created_at)
         VALUES ('secret.txt', 'secret.txt', 10, 'text/plain', 0) RETURNING id",
    )
    .fetch_one(&db)
    .await
    .unwrap();

    // Create a password-protected share.
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            &format!("/api/drop/{id}/share"),
            json!({"hours": 1, "password": "link-password"}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v: Value = {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    };
    let url = v["url"].as_str().unwrap().to_string();
    assert_eq!(v["password_protected"], json!(true));

    // No password → 401.
    let res = app
        .clone()
        .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Wrong password → 401.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{url}?pw=wrong"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Correct password → the file streams back.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{url}?pw=link-password"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(&bytes[..], b"hello drop");
}

#[tokio::test]
async fn multipart_upload_streams_and_downloads() {
    let (app, cookie, _db) = authed_router().await;

    let boundary = "XyZZy123";
    let payload = "hello streaming world";
    let crlf = "\r\n";
    let body = format!(
        "--{boundary}{crlf}Content-Disposition: form-data; name=\"file\"; filename=\"hello world.txt\"{crlf}Content-Type: text/plain{crlf}{crlf}{payload}{crlf}--{boundary}--{crlf}"
    );
    let req = Request::builder()
        .method("POST")
        .uri("/api/drop/upload")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header("content-length", body.len())
        .header("cookie", &cookie)
        .header("x-tpt-request", "1")
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "body: {}",
        String::from_utf8_lossy(&bytes)
    );
    let uploaded: Value = serde_json::from_slice(&bytes).unwrap();
    let id = uploaded["id"].as_i64().unwrap();
    assert_eq!(uploaded["filename"], json!("hello world.txt"));
    assert_eq!(uploaded["size"], json!(payload.len()));

    // Download through the authenticated route, filename sanitized in the
    // Content-Disposition header.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/drop/{id}/download"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let disposition = res
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&bytes[..], payload.as_bytes());
    assert!(disposition.contains("hello world.txt"));

    // Cleanup endpoint removes expired only; delete removes the row+file.
    let res = app
        .clone()
        .oneshot(json_req(
            "DELETE",
            &format!("/api/drop/{id}"),
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let stored = test_config().uploads_dir().join("drop");
    // `secret.txt` may legitimately exist here (seeded by the share test,
    // which runs in parallel); our uploaded file must be gone.
    let leftovers: Vec<_> = std::fs::read_dir(&stored)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != "secret.txt")
                .collect()
        })
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "file should be deleted with the row: {leftovers:?}"
    );
}
