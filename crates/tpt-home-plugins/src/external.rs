// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Forwarding for out-of-process plugins (`runtime = "external"`).
//!
//! The process listens on a loopback URL named by the manifest. The host
//! forwards `/api/<id>/…` to it with the same method, path, query and body,
//! and returns its status, content type and body. The plugin's
//! `max_millis` bounds the whole exchange, and responses are capped at the
//! host's 4 MiB limit.

use std::sync::OnceLock;
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

use crate::host::MAX_STRING_RESULT;

type HttpClient = Client<HttpConnector, Full<Bytes>>;

fn client() -> &'static HttpClient {
    static CLIENT: OnceLock<HttpClient> = OnceLock::new();
    CLIENT.get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
}

/// Forward one request to an external plugin and return its response.
///
/// Failures become host errors: `504` when the plugin is too slow, `502` when
/// it is unreachable or misbehaves. The plugin's own status codes pass through.
pub async fn forward(
    base: &str,
    method: &str,
    path: &str,
    query: &str,
    body: Option<String>,
    timeout: Duration,
) -> Response {
    let uri = if query.is_empty() {
        format!("{base}{path}")
    } else {
        format!("{base}{path}?{query}")
    };
    let method = match method.parse::<axum::http::Method>() {
        Ok(method) => method,
        Err(_) => return upstream_error(StatusCode::BAD_REQUEST, "unsupported method"),
    };
    let has_body = body.is_some();
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if has_body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let request = match builder.body(Full::new(Bytes::from(body.unwrap_or_default()))) {
        Ok(request) => request,
        Err(_) => return upstream_error(StatusCode::BAD_REQUEST, "malformed request"),
    };

    let exchange = async {
        let response = client()
            .request(request)
            .await
            .map_err(|_| upstream_error(StatusCode::BAD_GATEWAY, "plugin process unreachable"))?;
        let status = response.status();
        let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
        let bytes = Limited::new(response.into_body(), MAX_STRING_RESULT as usize)
            .collect()
            .await
            .map_err(|_| {
                upstream_error(
                    StatusCode::BAD_GATEWAY,
                    "plugin response too large or cut off",
                )
            })?
            .to_bytes();
        Ok::<_, Response>((status, content_type, bytes))
    };

    match tokio::time::timeout(timeout, exchange).await {
        Err(_) => upstream_error(
            StatusCode::GATEWAY_TIMEOUT,
            "plugin exceeded its time limit",
        ),
        Ok(Err(response)) => response,
        Ok(Ok((status, content_type, bytes))) => {
            let content_type =
                content_type.unwrap_or_else(|| HeaderValue::from_static("text/plain"));
            (status, [(header::CONTENT_TYPE, content_type)], bytes).into_response()
        }
    }
}

fn upstream_error(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({ "error": message }).to_string();
    (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body,
    )
        .into_response()
}
