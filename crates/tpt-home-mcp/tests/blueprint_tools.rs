// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Blueprint tools end to end: a real server on a loopback port, an agent's
//! token, and tool calls that go through the server's own routes. The checks
//! that matter are the ones the server makes: a call without credentials is
//! refused, and a call with a token gets the note back.

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::auth::api_token::create_api_token;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};
use tpt_home_mcp::{Caller, handle};

/// Serve the application on a listener bound beforehand.
async fn serve(listener: tokio::net::TcpListener, app: axum::Router) {
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
}

async fn token_for_admin(db: &DbPool, cfg: &Config) -> String {
    let app = build_router(db.clone(), cfg);
    let res = app
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
    assert_eq!(res.status(), StatusCode::CREATED);
    let (user_id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(db)
        .await
        .unwrap();
    let (_, token) = create_api_token(db, user_id, "agent", &["api".to_string()], None)
        .await
        .unwrap();
    token
}

async fn call(caller: &Caller, name: &str, arguments: Value) -> Value {
    let reply = handle(
        caller,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}}),
    )
    .await
    .expect("a reply");
    reply["result"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn blueprint_tools_act_through_the_server_with_the_callers_credential() {
    let _ = std::any::type_name::<tpt_home_bp_notes::api::NoteRow>();
    let _ = std::any::type_name::<tpt_home_bp_track::api::FieldRow>();
    let _ = tpt_home_mcp::__link_anchor();

    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    // The port must be known before the router is built, because the tools
    // call the server on it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    cfg.port = listener.local_addr().unwrap().port();
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let token = token_for_admin(&db, &cfg).await;
    serve(listener, build_router(db.clone(), &cfg)).await;

    let agent = tpt_home_mcp::caller_for_token(&db, &token).await.unwrap();

    // The notes tools are listed for a caller with the API scope.
    let list = handle(
        &agent,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await
    .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in ["notes_create", "notes_list", "notes_get", "track_row_add"] {
        assert!(names.contains(&expected), "{expected} is listed: {names:?}");
    }

    // Create, read back, and delete a note through the tools.
    let created = call(
        &agent,
        "notes_create",
        json!({"title": "Shopping", "body": "milk", "tags": ["home"]}),
    )
    .await;
    assert_eq!(created["isError"], false, "{created}");
    let note: Value =
        serde_json::from_str(created["structuredContent"]["body"].as_str().unwrap()).unwrap();
    let id = note["id"].as_i64().expect("note id");

    let fetched = call(&agent, "notes_get", json!({"id": id})).await;
    assert_eq!(fetched["isError"], false);
    assert!(
        fetched["structuredContent"]["body"]
            .as_str()
            .unwrap()
            .contains("Shopping")
    );

    let listed = call(&agent, "notes_list", json!({"q": "milk"})).await;
    assert!(
        listed["structuredContent"]["body"]
            .as_str()
            .unwrap()
            .contains("Shopping")
    );

    let deleted = call(&agent, "notes_delete", json!({"id": id})).await;
    assert_eq!(deleted["isError"], false, "{deleted}");
    let gone = call(&agent, "notes_get", json!({"id": id})).await;
    assert_eq!(gone["isError"], true, "a deleted note is not found");

    // A missing path argument is refused before any request is made.
    let missing = call(&agent, "notes_get", json!({})).await;
    assert_eq!(missing["isError"], true);
    assert!(
        missing["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("'id' is required")
    );

    // Without the caller's credential the server refuses, and the tool says so.
    let anonymous = Caller {
        forward: Vec::new(),
        ..agent.clone()
    };
    let refused = call(&anonymous, "notes_list", json!({})).await;
    assert_eq!(refused["isError"], true);
    assert_eq!(refused["structuredContent"]["status"], 401);
}
