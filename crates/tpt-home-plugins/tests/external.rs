// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Forwarding to out-of-process plugins, against a real loopback server.

use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use http_body_util::BodyExt;
use tpt_home_plugins::external::forward;
use tpt_home_plugins::manifest::validate_external_entry;

/// Start a loopback server and return its base URL.
async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

async fn body_text(res: axum::response::Response) -> (StatusCode, String) {
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn forwards_method_path_query_and_passes_status_through() {
    let router = Router::new().route(
        "/events",
        get(
            |q: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                (
                    StatusCode::CREATED,
                    format!("from={}", q.get("from").cloned().unwrap_or_default()),
                )
            },
        ),
    );
    let base = serve(router).await;
    let res = forward(
        &base,
        "GET",
        "/events",
        "from=2026",
        None,
        Duration::from_secs(5),
    )
    .await;
    let (status, body) = body_text(res).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body, "from=2026");
}

#[tokio::test]
async fn slow_plugin_times_out_with_504() {
    let router = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            "late"
        }),
    );
    let base = serve(router).await;
    let res = forward(&base, "GET", "/slow", "", None, Duration::from_millis(100)).await;
    assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn unreachable_plugin_is_a_bad_gateway() {
    // Bind then drop, so the port is free and nothing listens on it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let res = forward(
        &format!("http://127.0.0.1:{port}"),
        "GET",
        "/x",
        "",
        None,
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn posts_the_body_through() {
    let router = Router::new().route(
        "/echo",
        axum::routing::post(|body: String| async move { body }),
    );
    let base = serve(router).await;
    let res = forward(
        &base,
        "POST",
        "/echo",
        "",
        Some("{\"label\":\"milk\"}".into()),
        Duration::from_secs(5),
    )
    .await;
    let (_, body) = body_text(res).await;
    assert_eq!(body, "{\"label\":\"milk\"}");
}

#[test]
fn external_entries_must_be_bare_loopback_http_urls() {
    assert!(validate_external_entry("http://127.0.0.1:8790").is_ok());
    assert!(validate_external_entry("http://localhost:9000").is_ok());
    assert!(validate_external_entry("http://[::1]:9000").is_ok());

    assert!(
        validate_external_entry("https://127.0.0.1:8790").is_err(),
        "https"
    );
    assert!(
        validate_external_entry("http://example.com:8790").is_err(),
        "remote host"
    );
    assert!(
        validate_external_entry("http://0.0.0.0:8790").is_err(),
        "unspecified address"
    );
    assert!(
        validate_external_entry("http://127.0.0.1").is_err(),
        "no port"
    );
    assert!(
        validate_external_entry("http://127.0.0.1:8790/api").is_err(),
        "path"
    );
    assert!(
        validate_external_entry("http://127.0.0.1:8790/?x=1").is_err(),
        "query"
    );
    assert!(validate_external_entry("plugin.wasm").is_err(), "not a URL");
}
