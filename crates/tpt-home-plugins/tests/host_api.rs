// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `POST /api/plugin-host/<id>/<op>`, the host services for external plugin
//! processes, through the real router. Only a token scoped to that plugin may
//! use it: a browser session, a general `api` token, or a token for another
//! plugin are refused. Approved permissions and table scoping still apply.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::auth::api_token::create_api_token;
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::DbPool;
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::manifest::Permissions;
use tpt_home_plugins::registry;

const MANIFEST: &str = r#"
[plugin]
id = "ticker"
name = "Ticker"
version = "0.1.0"
runtime = "external"
entry = "http://127.0.0.1:9"

[permissions]
db = true

[[tables]]
name = "items"
[[tables.columns]]
name = "label"
kind = "text"
"#;

async fn setup() -> (axum::Router, DbPool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    set_global_for_tests(cfg.clone());
    // The host services open the database file under the data directory, so
    // the test uses that same file.
    let db = tpt_home_core::db::connect(&cfg.db_path()).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    std::fs::create_dir_all(dir.path().join("plugins")).unwrap();
    let staged = dir.path().join("staged");
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join("plugin.toml"), MANIFEST).unwrap();
    let granted = Permissions {
        db: true,
        ..Permissions::default()
    };
    lifecycle::install_from_dir(&db, &staged, true, &granted)
        .await
        .expect("install the external plugin");
    assert!(registry::get_enabled("ticker").is_some());

    (build_router(db.clone(), &cfg), db, dir)
}

async fn token(db: &DbPool, owner: i64, scopes: &[&str]) -> String {
    let scopes: Vec<String> = scopes.iter().map(|s| s.to_string()).collect();
    create_api_token(db, owner, "host", &scopes, None)
        .await
        .unwrap()
        .1
}

async fn post(
    app: &axum::Router,
    uri: &str,
    auth: Option<(&str, &str)>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some((name, value)) = auth {
        builder = builder.header(name, value);
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn only_a_token_scoped_to_the_plugin_reaches_its_host_services() {
    let (app, db, _dir) = setup().await;

    // An admin is needed to own the tokens; create one through the setup route.
    let setup = post(
        &app,
        "/api/setup",
        None,
        json!({"username": "admin", "password": "hunter2hunter2"}),
    )
    .await;
    assert_eq!(setup.0, StatusCode::CREATED);
    let session = {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"username": "admin", "password": "hunter2hunter2"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        res.headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    };
    let (owner,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(&db)
        .await
        .unwrap();
    let scoped = token(&db, owner, &["plugin:ticker"]).await;
    let other = token(&db, owner, &["plugin:other"]).await;
    let general = token(&db, owner, &["api"]).await;

    let insert = json!({"sql": "INSERT INTO ticker_items (label) VALUES ('milk')"});
    let uri = "/api/plugin-host/ticker/db/execute";

    // The scoped token works, and the write lands in the plugin's table.
    let (status, body) = post(
        &app,
        uri,
        Some(("authorization", &format!("Bearer {scoped}"))),
        insert.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.to_string().contains("rows_affected"), "{body}");

    // A session, a general API token, and another plugin's token are refused.
    for auth in [
        ("cookie", session.as_str()),
        ("authorization", &format!("Bearer {general}") as &str),
    ] {
        let (status, _) = post(&app, uri, Some(auth), insert.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "refused for {}", auth.0);
    }
    let (status, _) = post(
        &app,
        uri,
        Some(("authorization", &format!("Bearer {other}"))),
        insert,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Anonymous callers never get past the guard.
    let (status, _) = post(&app, uri, None, json!({"sql": "SELECT 1"})).await;
    assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);
}
