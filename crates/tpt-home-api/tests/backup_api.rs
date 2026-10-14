// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Admin backup API and health endpoint through the real router: take a
//! backup, download it, upload it back, and the refusals (non-admin, bad
//! file, missing custom header, names that are not backups).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::DbPool;

const ADMIN_PW: &str = "hunter2hunter2";
const USER_PW: &str = "correct horse battery";

/// A fresh data directory per test, so backup folders do not interfere.
fn config(dir: &tempfile::TempDir) -> Config {
    Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

async fn app(cfg: &Config) -> (axum::Router, DbPool) {
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    // A file database, as the server uses. VACUUM INTO does not write a file
    // from an in-memory database.
    let db: DbPool = tpt_home_core::db::connect(&cfg.db_path()).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    (build_router(db.clone(), cfg), db)
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
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

/// Create the first admin through setup, then return its session cookie.
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
    assert_eq!(res.status(), StatusCode::CREATED, "setup failed");
    cookie_from(&res)
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
    cookie_from(&res)
}

fn post_empty_json(uri: &str, cookie: &str) -> Request<Body> {
    json_req("POST", uri, json!({}), cookie)
}

/// A multipart body with one file field, as a browser form would send it.
fn multipart(field: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = "TPTBOUNDARY";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"backup.db\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn upload_req(
    cookie: &str,
    content_type: &str,
    body: Vec<u8>,
    custom_header: bool,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/admin/backups/upload")
        .header("content-type", content_type)
        .header("cookie", cookie);
    if custom_header {
        builder = builder.header("x-tpt-request", "1");
    }
    builder.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn backup_is_taken_listed_and_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    let (app, _db) = app(&cfg).await;
    let admin = admin_cookie(&app).await;

    let res = send(&app, post_empty_json("/api/admin/backups", &admin)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let made = body_json(res).await;
    let name = made["name"].as_str().unwrap().to_string();
    assert!(
        made["path"]
            .as_str()
            .unwrap()
            .ends_with(&format!("tpt-{name}.db"))
    );
    assert!(made["db_bytes"].as_u64().unwrap() > 0);

    let res = send(&app, get("/api/admin/backups", &admin)).await;
    let list = body_json(res).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], name.as_str());

    let res = send(
        &app,
        get(&format!("/api/admin/backups/{name}/download"), &admin),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "application/octet-stream");
    let bytes = body_bytes(res).await;
    assert!(bytes.starts_with(b"SQLite format 3\0"), "not a SQLite file");
}

#[tokio::test]
async fn uploaded_database_becomes_a_backup_and_bad_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    let (app, _db) = app(&cfg).await;
    let admin = admin_cookie(&app).await;

    // A real database to upload: take a backup, then download it.
    let made = body_json(send(&app, post_empty_json("/api/admin/backups", &admin)).await).await;
    let name = made["name"].as_str().unwrap();
    let real = body_bytes(
        send(
            &app,
            get(&format!("/api/admin/backups/{name}/download"), &admin),
        )
        .await,
    )
    .await;

    let (ct, body) = multipart("file", &real);
    let res = send(&app, upload_req(&admin, &ct, body, true)).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    let uploaded = body_json(res).await;
    assert!(uploaded["path"].as_str().unwrap().contains("backups"));

    let list = body_json(send(&app, get("/api/admin/backups", &admin)).await).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        2,
        "upload kept as a new backup"
    );

    // Not a database: refused, and nothing is kept.
    let (ct, body) = multipart("file", b"this is not a database");
    let res = send(&app, upload_req(&admin, &ct, body, true)).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let list = body_json(send(&app, get("/api/admin/backups", &admin)).await).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    let leftovers: Vec<_> = std::fs::read_dir(cfg.data_dir.join("backups"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".upload-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging file left behind: {leftovers:?}"
    );

    // Wrong form field name.
    let (ct, body) = multipart("document", &real);
    let res = send(&app, upload_req(&admin, &ct, body, true)).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // The custom header is required for multipart, as elsewhere.
    let (ct, body) = multipart("file", &real);
    let res = send(&app, upload_req(&admin, &ct, body, false)).await;
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn names_that_are_not_backups_are_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    let (app, _db) = app(&cfg).await;
    let admin = admin_cookie(&app).await;
    for name in ["abc", "999999", "..%2Fsecret", "1%2F2"] {
        let res = send(
            &app,
            get(&format!("/api/admin/backups/{name}/download"), &admin),
        )
        .await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "name {name}");
    }
}

#[tokio::test]
async fn backup_api_and_health_are_admin_only() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    let (app, _db) = app(&cfg).await;
    let admin = admin_cookie(&app).await;
    let user = user_cookie(&app, &admin).await;

    for uri in ["/api/admin/backups", "/api/admin/health"] {
        let res = send(&app, get(uri, &user)).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{uri}");
    }
    let res = send(&app, post_empty_json("/api/admin/backups", &user)).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // The status page sends non-admins back to the dashboard. The account was
    // created by an admin, so it first chooses its own password.
    let res = send(
        &app,
        json_req(
            "POST",
            "/api/auth/change-password",
            json!({"current": USER_PW, "new_password": "alice-chosen-pass"}),
            &user,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    let user = login_cookie(&app, "alice", "alice-chosen-pass").await;
    let res = send(&app, get("/admin/status", &user)).await;
    assert!(res.status().is_redirection());
    assert_eq!(res.headers()["location"], "/dashboard");
}

#[tokio::test]
async fn health_reports_version_and_backup_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir);
    cfg.backup_every_hours = 24;
    cfg.backup_keep = 3;
    let (app, _db) = app(&cfg).await;
    let admin = admin_cookie(&app).await;

    let health = body_json(send(&app, get("/api/admin/health", &admin)).await).await;
    assert_eq!(health["version"], tpt_home_core::VERSION);
    assert_eq!(health["backups"], 0);
    assert!(health["last_backup_at"].is_null());
    assert_eq!(health["backup_every_hours"], 24);
    assert_eq!(health["backup_keep"], 3);
    assert_eq!(health["failing_jobs"], 0);
    // The data directory is on a real disk, so both figures are known and
    // the free space cannot exceed the total.
    let free = health["disk_free_bytes"].as_u64().expect("free space");
    let total = health["disk_total_bytes"].as_u64().expect("total space");
    assert!(total > 0 && free <= total);

    send(&app, post_empty_json("/api/admin/backups", &admin)).await;
    let health = body_json(send(&app, get("/api/admin/health", &admin)).await).await;
    assert_eq!(health["backups"], 1);
    assert!(health["last_backup_at"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn status_page_renders_for_admins() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    let (app, _db) = app(&cfg).await;
    let admin = admin_cookie(&app).await;
    let res = send(&app, get("/admin/status", &admin)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let html = String::from_utf8(body_bytes(res).await).unwrap();
    assert!(html.contains("Admin · Status"));
    assert!(html.contains("/api/admin/backups/upload"));
}

/// Sign in and return the session cookie.
async fn login_cookie(app: &axum::Router, username: &str, password: &str) -> String {
    let res = app
        .clone()
        .oneshot(json_req(
            "POST",
            "/api/auth/login",
            json!({"username": username, "password": password}),
            "",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "login as {username}");
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
