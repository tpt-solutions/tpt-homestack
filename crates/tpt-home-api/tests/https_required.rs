// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `require_https` through the real router: plain-HTTP page requests are
//! redirected, other methods are refused, a proxy's `X-Forwarded-Proto` counts
//! only with `trust_proxy`, and nothing changes when the option is off.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

use tpt_home_api::build_router;
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

async fn app(require_https: bool, trust_proxy: bool) -> axum::Router {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        require_https,
        trust_proxy,
        ..Config::default()
    };
    // Keep the directory alive for the whole test process.
    std::mem::forget(dir);
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    build_router(db, &cfg)
}

async fn send(
    app: &axum::Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
) -> axum::response::Response {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "home.example.com");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn plain_http_pages_redirect_to_https_when_required() {
    let app = app(true, false).await;
    let res = send(&app, Method::GET, "/login?next=/notes", &[]).await;
    assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        res.headers().get("location").unwrap(),
        "https://home.example.com/login?next=/notes"
    );
}

#[tokio::test]
async fn other_methods_over_plain_http_are_refused_when_required() {
    let app = app(true, false).await;
    let res = send(&app, Method::POST, "/api/auth/login", &[]).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_forwarded_https_request_is_believed_only_behind_a_trusted_proxy() {
    let untrusted = app(true, false).await;
    let res = send(
        &untrusted,
        Method::GET,
        "/login",
        &[("x-forwarded-proto", "https")],
    )
    .await;
    assert_eq!(
        res.status(),
        StatusCode::PERMANENT_REDIRECT,
        "header ignored"
    );

    let trusted = app(true, true).await;
    let res = send(
        &trusted,
        Method::GET,
        "/login",
        &[("x-forwarded-proto", "https")],
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK, "the proxy says HTTPS");
    let res = send(
        &trusted,
        Method::GET,
        "/login",
        &[("x-forwarded-proto", "http")],
    )
    .await;
    assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
}

#[tokio::test]
async fn nothing_changes_when_the_option_is_off() {
    let app = app(false, false).await;
    let res = send(&app, Method::GET, "/login", &[]).await;
    assert_eq!(res.status(), StatusCode::OK);
}
