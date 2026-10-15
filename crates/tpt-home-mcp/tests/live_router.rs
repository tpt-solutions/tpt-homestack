// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `POST /api/mcp` through the full application router: the `/api` guard
//! must authenticate the bearer token before the MCP handler runs, and a
//! request without a valid token must never reach the tools.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::auth::api_token::create_api_token;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

#[tokio::test]
async fn the_mcp_endpoint_sits_behind_the_api_guard() {
    // Keep the MCP handlers linked into this test binary.
    let _ = tpt_home_mcp::__link_anchor();

    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let app = build_router(db.clone(), &cfg);

    let setup = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username": "admin", "password": "hunter2hunter2"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::CREATED);
    let (user_id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(&db)
        .await
        .unwrap();
    let (_id, token) = create_api_token(&db, user_id, "agent", &["api".to_string()], None)
        .await
        .unwrap();

    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string();

    // No token: refused by the guard, with no JSON-RPC reply.
    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .body(Body::from(list.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // A made-up token is refused the same way.
    let forged = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .header("authorization", "Bearer not-a-real-token")
                .body(Body::from(list.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);

    // The real token reaches the MCP handler and gets the tool list.
    let ok = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(list))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let bytes = ok.into_body().collect().await.unwrap().to_bytes();
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["id"], 1);
    assert!(reply["result"]["tools"].is_array(), "{reply}");
}
