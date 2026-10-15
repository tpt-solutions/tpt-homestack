// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! End-to-end API tests over `build_router` (no network): bootstrap, login,
//! sessions, user administration and the middleware guards.

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
        std::mem::forget(dir); // lives for the whole test process
        Config {
            data_dir,
            ..Config::default()
        }
    })
}

async fn router() -> axum::Router {
    tpt_home_core::config::set_global_for_tests(test_config().clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    build_router(db, test_config())
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn json_request(method: &str, uri: &str, body: Value, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn setup_and_login(router: &axum::Router) -> String {
    let res = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/setup",
            json!({"username": "admin", "password": "hunter2hunter2"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "setup failed");
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

#[tokio::test]
async fn healthz_is_public() {
    let res = router()
        .await
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["service"], json!("tpt-homestack"));
}

#[tokio::test]
async fn setup_only_works_once() {
    let app = router().await;
    let cookie = setup_and_login(&app).await;
    assert!(cookie.starts_with("tpt_session="));

    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/setup",
            json!({"username": "evil", "password": "hunter2hunter2"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn protected_endpoints_require_auth() {
    let app = router().await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Pages redirect to /login instead.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers().get("location").unwrap(), "/login");
}

#[tokio::test]
async fn login_me_logout_flow() {
    let app = router().await;
    let _ = setup_and_login(&app).await;

    // Wrong password → 401.
    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/login",
            json!({"username": "admin", "password": "wrong-password"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Correct password → cookie.
    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/login",
            json!({"username": "admin", "password": "hunter2hunter2"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let cookie = cookie.split(';').next().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(json_request(
            "GET",
            "/api/auth/me",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let me = body_json(res).await;
    assert_eq!(me["username"], json!("admin"));
    assert_eq!(me["role"], json!("admin"));

    // Logout revokes the session.
    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/logout",
            json!({}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .oneshot(json_request(
            "GET",
            "/api/auth/me",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_user_management() {
    let app = router().await;
    let cookie = setup_and_login(&app).await;

    // Create a normal user.
    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/users",
            json!({"username": "alice", "password": "alice-password", "role": "user"}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = body_json(res).await;
    let alice_id = created["id"].as_i64().unwrap();

    // List shows both.
    let res = app
        .clone()
        .oneshot(json_request(
            "GET",
            "/api/admin/users",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    let users = body_json(res).await;
    assert_eq!(users.as_array().unwrap().len(), 2);

    // Disable alice → her login fails.
    let res = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/admin/users/{alice_id}"),
            json!({"disabled": true}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/login",
            json!({"username": "alice", "password": "alice-password"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Cannot demote the last admin.
    let res = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/users/1",
            json!({"role": "user"}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // Cannot delete self; cannot delete the last admin.
    let res = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            "/api/admin/users/1",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // Delete alice works.
    let res = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            &format!("/api/admin/users/{alice_id}"),
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn non_admin_cannot_manage_users() {
    let app = router().await;
    let admin_cookie = setup_and_login(&app).await;

    // Admin creates a normal user and logs in as them.
    let _ = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/users",
            json!({"username": "bob", "password": "bob-password-1", "role": "user"}),
            Some(&admin_cookie),
        ))
        .await
        .unwrap();
    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/login",
            json!({"username": "bob", "password": "bob-password-1"}),
            None,
        ))
        .await
        .unwrap();
    let bob_cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let bob_cookie = bob_cookie.split(';').next().unwrap().to_string();

    let res = app
        .oneshot(json_request(
            "GET",
            "/api/admin/users",
            Value::Null,
            Some(&bob_cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_content_type_guard() {
    let app = router().await;
    let _ = setup_and_login(&app).await;
    let res = app
        .clone()
        .oneshot(json_request(
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
    let cookie = cookie.split(';').next().unwrap().to_string();

    // A form-encoded POST with the cookie must be rejected (415).
    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/change-password")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("cookie", &cookie)
        .body(Body::from("current=x&new_password=y"))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn change_password_revokes_sessions() {
    let app = router().await;
    let cookie = setup_and_login(&app).await;

    let res = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/change-password",
            json!({"current": "hunter2hunter2", "new_password": "new-new-new-new"}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Old session revoked…
    let res = app
        .clone()
        .oneshot(json_request(
            "GET",
            "/api/auth/me",
            Value::Null,
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // …and the new password works.
    let res = app
        .oneshot(json_request(
            "POST",
            "/api/auth/login",
            json!({"username": "admin", "password": "new-new-new-new"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_page_renders() {
    let res = router()
        .await
        .oneshot(
            Request::builder()
                .uri("/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("Sign in"));
    assert!(html.contains("tpt_session") || html.contains("doLogin"));
}

/// Router over a database we seed ourselves, so tests can mint API tokens.
async fn router_with_user() -> (axum::Router, DbPool, i64) {
    tpt_home_core::config::set_global_for_tests(test_config().clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let user = tpt_home_core::auth::user::create_user(
        &db,
        "alice",
        "correct horse battery",
        tpt_home_core::auth::user::Role::User,
    )
    .await
    .unwrap();
    let router = build_router(db.clone(), test_config());
    (router, db, user.id)
}

fn bearer_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn bearer_token_authenticates_api_calls() {
    let (router, db, uid) = router_with_user().await;
    let (_, token) = tpt_home_core::auth::api_token::create_api_token(
        &db,
        uid,
        "scripts",
        &["api".to_string()],
        None,
    )
    .await
    .unwrap();

    let res = router
        .clone()
        .oneshot(bearer_get("/api/auth/me", &token))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let me = body_json(res).await;
    assert_eq!(me["username"], "alice");

    let bogus = router
        .oneshot(bearer_get("/api/auth/me", "tpt_bogus"))
        .await
        .unwrap();
    assert_eq!(bogus.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn plugin_scoped_token_is_limited_to_its_plugin() {
    let (router, db, uid) = router_with_user().await;
    let (_, token) = tpt_home_core::auth::api_token::create_api_token(
        &db,
        uid,
        "calendar process",
        &["plugin:calendar".to_string()],
        None,
    )
    .await
    .unwrap();

    // Outside the plugin scope: forbidden, not unauthenticated.
    let res = router
        .clone()
        .oneshot(bearer_get("/api/auth/me", &token))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Pages are never reachable with a bearer token.
    let page = router
        .oneshot(bearer_get("/dashboard", &token))
        .await
        .unwrap();
    assert_ne!(page.status(), StatusCode::OK);
}

#[tokio::test]
async fn cookie_sessions_still_work_alongside_tokens() {
    let (router, _db, _) = router_with_user().await;
    let res = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/auth/login",
            json!({"username": "alice", "password": "correct horse battery"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
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
    let me = router
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);
}

/// Router with `trust_proxy` set, over a fresh database.
async fn router_with_trust_proxy(trust: bool) -> axum::Router {
    tpt_home_core::config::set_global_for_tests(test_config().clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let cfg = Config {
        trust_proxy: trust,
        ..test_config().clone()
    };
    build_router(db, &cfg)
}

/// Sends `n` requests to the same route, each with a different forwarded
/// client address, and returns the status of the last one.
async fn send_with_forwarded_ips(app: &axum::Router, n: u32) -> StatusCode {
    let mut last = StatusCode::OK;
    for i in 0..n {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/me")
                    .header("x-forwarded-for", format!("198.51.100.{}", i % 250))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        last = res.status();
    }
    last
}

#[tokio::test]
async fn forwarded_header_ignored_without_trust_proxy() {
    // Without a proxy, every request shares the peer's bucket, so spoofing a
    // new X-Forwarded-For per request cannot get past the 240-per-minute limit.
    let app = router_with_trust_proxy(false).await;
    let status = send_with_forwarded_ips(&app, 241).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn forwarded_header_keys_the_limiter_with_trust_proxy() {
    // Behind a trusted proxy each forwarded client has its own bucket, so
    // 241 requests from 250 distinct addresses are never limited.
    let app = router_with_trust_proxy(true).await;
    let status = send_with_forwarded_ips(&app, 241).await;
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn login_attempts_are_limited_per_client() {
    // Guessing passwords is capped well below the general API limit.
    // Each attempt uses a different name, so the per-account lockout cannot
    // trigger and only the per-client limit is measured.
    let app = router_with_trust_proxy(false).await;
    let attempt = |i: u32| {
        json_request(
            "POST",
            "/api/auth/login",
            json!({"username": format!("nobody-{i}"), "password": "wrong-guess"}),
            None,
        )
    };
    for i in 0..tpt_home_core::middleware::LOGIN_MAX_DEFAULT {
        let res = app.clone().oneshot(attempt(i)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
    let res = app.clone().oneshot(attempt(999)).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn repeated_wrong_passwords_lock_the_account_for_every_client() {
    // Ten failures lock the name even though the requests have no client
    // address, and the locked name is refused without checking the password.
    let app = router_with_trust_proxy(false).await;
    let name = "lockout-integration-victim";
    let attempt = |password: &str| {
        json_request(
            "POST",
            "/api/auth/login",
            json!({"username": name, "password": password}),
            None,
        )
    };
    for _ in 0..tpt_home_core::auth::lockout::MAX_FAILURES {
        let res = app.clone().oneshot(attempt("wrong-guess")).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
    let res = app.clone().oneshot(attempt("wrong-guess")).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let res = app.oneshot(attempt("the-right-password")).await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn multipart_requires_the_custom_request_header() {
    // A cross-site form can send multipart but cannot set X-TPT-Request, so
    // without the header the request is refused before routing.
    let app = router().await;
    let multipart = |header: bool| {
        let mut b = Request::builder()
            .method("POST")
            .uri("/api/drop/upload")
            .header("content-type", "multipart/form-data; boundary=x");
        if header {
            b = b.header("x-tpt-request", "1");
        }
        b.body(Body::from("--x--\r\n")).unwrap()
    };
    let res = app.clone().oneshot(multipart(false)).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let res = app.oneshot(multipart(true)).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn html_pages_allow_only_nonced_scripts() {
    // Inline script injected into a page must not run: the policy names a
    // per-response nonce, and every script tag the server writes carries it.
    let app = router().await;
    let cookie = setup_and_login(&app).await;
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/welcome")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let csp = res
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(!csp.contains("script-src 'self' 'unsafe-inline'"), "{csp}");
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .expect("policy names a nonce");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.contains(&format!("<script nonce=\"{nonce}\">")),
        "no nonced script"
    );
    assert!(!html.contains("<script>"), "a script tag without the nonce");
    // Styles: no inline style attributes, and the shell's <style> is nonced.
    assert!(!csp.contains("style-src 'self' 'unsafe-inline'"), "{csp}");
    assert!(
        csp.contains(&format!("style-src 'self' 'nonce-{nonce}'")),
        "{csp}"
    );
    assert!(
        html.contains(&format!("<style nonce=\"{nonce}\">")),
        "no nonced style"
    );
    assert!(!html.contains(" style=\""), "an inline style attribute");
}

#[tokio::test]
async fn each_response_gets_a_fresh_nonce() {
    let app = router().await;
    let mut nonces = Vec::new();
    for _ in 0..2 {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let csp = res
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        nonces.push(
            csp.split("'nonce-")
                .nth(1)
                .unwrap()
                .split('\'')
                .next()
                .unwrap()
                .to_string(),
        );
    }
    assert_ne!(nonces[0], nonces[1], "a nonce must not be reused");
}
