// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Shared HTTP middleware: session resolution, role guards, rate limiting,
//! CSRF/content-type check, request ids and security headers.
//!
//! Everything here is a plain named `async fn` so both the core router and
//! macro-generated layers can use them via `axum::middleware::from_fn` /
//! `from_fn_with_state` without any unnameable types in public signatures.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tracing::Instrument;

use crate::auth::api_token::{TokenScopes, resolve_api_token, scope_allows};
use crate::auth::session::{parse_session_cookie, resolve_session};
use crate::auth::user::AuthUser;
use crate::db::DbPool;

/// Resolve the session cookie into an [`AuthUser`] request extension.
///
/// Unauthenticated requests get `401` JSON for `/api/...` paths and a
/// redirect to `/login` for page paths. The login/setup/share endpoints are
/// mounted outside this layer. Use with
/// `axum::middleware::from_fn_with_state(db.clone(), auth_resolver)`.
///
/// A cookie session wins. Otherwise an `Authorization: Bearer` API token is
/// accepted, but only on `/api/` paths and only within the token's scopes
/// (see [`crate::auth::api_token::scope_allows`]); anything else is `403`.
pub async fn auth_resolver(State(db): State<DbPool>, req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/");

    if let Some(token) = parse_session_cookie(req.headers())
        && let Ok(Some(user)) = resolve_session(&db, &token).await
    {
        let mut req = req;
        req.extensions_mut().insert(user);
        req.extensions_mut()
            .insert(TokenScopes(vec!["api".to_string()]));
        return next.run(req).await;
    }

    if let Some(token) = parse_bearer(req.headers()) {
        return match resolve_api_token(&db, &token).await {
            Ok(Some(resolved)) if is_api && scope_allows(&resolved.scopes, req.uri().path()) => {
                let mut req = req;
                req.extensions_mut().insert(resolved.user);
                req.extensions_mut().insert(TokenScopes(resolved.scopes));
                next.run(req).await
            }
            Ok(Some(_)) => (
                StatusCode::FORBIDDEN,
                "{\"error\":\"token scope does not allow this path\"}",
            )
                .into_response(),
            _ => (
                StatusCode::UNAUTHORIZED,
                "{\"error\":\"authentication required\"}",
            )
                .into_response(),
        };
    }

    if is_api {
        (
            StatusCode::UNAUTHORIZED,
            "{\"error\":\"authentication required\"}",
        )
            .into_response()
    } else {
        axum::response::Redirect::to("/login").into_response()
    }
}

/// The bearer token from an `Authorization: Bearer <token>` header, if any.
fn parse_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// Require the authenticated user to satisfy a role. Admins satisfy every
/// requirement; a missing identity is `401`, an insufficient one `403`.
///
/// Called from generated/macro code as:
/// `axum::middleware::from_fn(move |req, next| async move {
///     role_guard(req, next, "admin").await
/// })`
pub async fn role_guard(req: Request, next: Next, required: &'static str) -> Response {
    let holder = req.extensions().get::<AuthUser>();
    if holder.is_some_and(|u| u.role.allows(Some(required))) {
        return next.run(req).await;
    }
    if holder.is_some() {
        (
            StatusCode::FORBIDDEN,
            "{\"error\":\"insufficient permissions\"}",
        )
            .into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "{\"error\":\"authentication required\"}",
        )
            .into_response()
    }
}

/// Defense-in-depth CSRF guard: state-changing `/api` calls must carry
/// `Content-Type: application/json` (or `multipart/form-data` for uploads).
/// Cross-site HTML forms cannot set that header without a CORS preflight,
/// which is not granted by default. `SameSite=Lax` cookies are the primary
/// line of defense; this is the second.
pub async fn csrf_guard(req: Request, next: Next) -> Response {
    let mutating = !matches!(
        req.method(),
        &axum::http::Method::GET | &axum::http::Method::HEAD | &axum::http::Method::OPTIONS
    );
    if mutating && req.uri().path().starts_with("/api/") {
        let ct = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| {
                ct.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        if ct != "application/json" && ct != "multipart/form-data" {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{\"error\":\"Content-Type must be application/json (or multipart/form-data)\"}",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Add standard security headers to every response.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    // Inline script/style is required by the server-rendered shell for now;
    // tightened CSP when the asset pipeline lands (Phase 3/7).
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; img-src 'self' data:",
        ),
    );
    res
}

/// Attach a unique request id and a tracing span to each request.
pub async fn request_id(req: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let span = tracing::info_span!(
        "http",
        rid = %id,
        method = %req.method(),
        path = %req.uri().path()
    );
    let mut res = next.run(req).instrument(span).await;
    if let Ok(hv) = HeaderValue::from_str(&id) {
        res.headers_mut().insert("x-request-id", hv);
    }
    res
}

/// Fixed-window in-memory rate limiter keyed by client IP + bucket.
/// Sufficient for a home server: no external state, a few KiB at most.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<LimiterInner>,
}

struct LimiterInner {
    max: u32,
    window: Duration,
    hits: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(LimiterInner {
                max,
                window,
                hits: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Record a hit for `key`; `false` means over the limit.
    pub fn check(&self, key: &str) -> bool {
        let mut hits = self.inner.hits.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        hits.retain(|_, (start, _)| now.duration_since(*start) < self.inner.window);
        let entry = hits.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.inner.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.inner.max
    }
}

/// Client key for the limiter: the reverse-proxy-forwarded IP when present,
/// otherwise the direct peer address.
pub fn client_key(req: &Request) -> String {
    if let Some(ip) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return ip.to_string();
    }
    req.extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn bucket_for(path: &str) -> &'static str {
    if path.starts_with("/api/auth") || path.starts_with("/api/setup") {
        "auth"
    } else {
        "api"
    }
}

/// Rate limiting middleware; use with
/// `axum::middleware::from_fn_with_state(limiter, rate_limit_fn)`.
/// Auth endpoints get a strict bucket, everything else a general one.
pub async fn rate_limit_fn(
    State(limiter): State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let key = format!("{}:{}", bucket_for(req.uri().path()), client_key(&req));
    if limiter.check(&key) {
        next.run(req).await
    } else {
        let mut res = (
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\":\"too many requests\"}",
        )
            .into_response();
        res.headers_mut()
            .insert("Retry-After", HeaderValue::from_static("60"));
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_enforces_window() {
        let rl = RateLimiter::new(2, Duration::from_secs(60));
        assert!(rl.check("ip"));
        assert!(rl.check("ip"));
        assert!(!rl.check("ip"));
        assert!(rl.check("other-ip"), "independent keys");
    }

    #[test]
    fn bucket_selection() {
        assert_eq!(bucket_for("/api/auth/login"), "auth");
        assert_eq!(bucket_for("/api/setup"), "auth");
        assert_eq!(bucket_for("/api/notes"), "api");
    }
}
