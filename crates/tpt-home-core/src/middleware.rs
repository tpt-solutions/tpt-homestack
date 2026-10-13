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

/// Header a client must send with multipart uploads; any value works.
pub const REQUEST_HEADER: &str = "x-tpt-request";

/// Defense-in-depth CSRF guard: state-changing `/api` calls must carry
/// `Content-Type: application/json`, `multipart/form-data` (uploads, which also
/// need the `X-TPT-Request` header) or `text/csv` (imports). HTML forms can
/// only send `text/plain`, urlencoded or multipart bodies, and any other type
/// from another origin needs a CORS preflight, which is not granted by default.
/// JSON and CSV are therefore safe; multipart is safe because a form cannot
/// set the custom header. `SameSite=Lax` cookies are the primary line of
/// defense; this is the second.
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
        if !matches!(
            ct.as_str(),
            "application/json" | "multipart/form-data" | "text/csv"
        ) {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{\"error\":\"Content-Type must be application/json, multipart/form-data or text/csv\"}",
            )
                .into_response();
        }
        // A cross-site HTML form can send multipart without a preflight, but
        // it cannot set a custom header. Requiring one closes that gap even
        // where SameSite cookies are not enforced.
        if ct == "multipart/form-data" && !req.headers().contains_key(REQUEST_HEADER) {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "{\"error\":\"multipart requests must send the X-TPT-Request header\"}",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Add standard security headers to every response.
/// Largest HTML page whose scripts are rewritten with a nonce. Pages are small
/// server-rendered strings; anything larger is refused rather than buffered.
const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;

/// Policy for responses that carry no page script. Scripts must come from this
/// origin; inline scripts and inline event handlers are refused.
const STATIC_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; object-src 'none'; base-uri 'self'; form-action 'self'; \
     frame-ancestors 'none'";

/// Security headers on every response. An HTML page gets a fresh nonce: its
/// `<script>` tags carry it, and the policy allows only those scripts, so
/// inline script injected into a page does not run.
///
/// Styles still allow `'unsafe-inline'`: the pages use `style=` attributes.
/// Removing that needs the same treatment and is not done yet.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let res = next.run(req).await;
    let (mut parts, body) = res.into_parts();

    let is_html = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    // Downloads are attachments of user-uploaded files; never rewrite them.
    let is_download = parts.headers.contains_key(header::CONTENT_DISPOSITION);

    let mut csp = STATIC_CSP.to_string();
    let body = if is_html && !is_download {
        match axum::body::to_bytes(body, MAX_HTML_BYTES).await {
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => {
                    csp = format!(
                        "default-src 'self'; script-src 'self' 'nonce-{nonce}'; \
                         style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                         object-src 'none'; base-uri 'self'; form-action 'self'; \
                         frame-ancestors 'none'"
                    );
                    // The length changes with the nonce, so the old header goes.
                    parts.headers.remove(header::CONTENT_LENGTH);
                    axum::body::Body::from(
                        text.replace("<script>", &format!("<script nonce=\"{nonce}\">")),
                    )
                }
                Err(_) => axum::body::Body::from(bytes),
            },
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "page too large").into_response();
            }
        }
    } else {
        body
    };

    parts.headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    parts
        .headers
        .insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    parts
        .headers
        .insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    if let Ok(value) = HeaderValue::from_str(&csp) {
        parts.headers.insert("Content-Security-Policy", value);
    }
    Response::from_parts(parts, body)
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
    /// Stricter cap for the login bucket (`/api/auth/login`, `/api/setup`).
    login_max: u32,
    window: Duration,
    trust_proxy: bool,
    hits: Mutex<HitTable>,
}

/// Per-client hit counts: `(window start, hits)`, plus when stale entries were
/// last removed.
struct HitTable {
    counts: HashMap<String, (Instant, u32)>,
    pruned_at: Instant,
}

impl HitTable {
    fn new() -> Mutex<Self> {
        Mutex::new(Self {
            counts: HashMap::new(),
            pruned_at: Instant::now(),
        })
    }
}

impl RateLimiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(LimiterInner {
                max,
                login_max: LOGIN_MAX_DEFAULT,
                window,
                trust_proxy: false,
                hits: HitTable::new(),
            }),
        }
    }

    /// Key clients by `X-Forwarded-For` instead of the socket peer. Only set
    /// this when a reverse proxy you control is the sole way to reach the
    /// server; otherwise any client can spoof the header to dodge the limit.
    pub fn trust_proxy(self, trust: bool) -> Self {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(LimiterInner {
                max: inner.max,
                login_max: inner.login_max,
                window: inner.window,
                trust_proxy: trust,
                hits: HitTable::new(),
            }),
        }
    }

    /// Record a hit for `key`; `false` means over the general limit.
    pub fn check(&self, key: &str) -> bool {
        self.check_with_max(key, self.inner.max)
    }

    /// Record a hit for `key` against an explicit cap; `false` means over it.
    fn check_with_max(&self, key: &str, max: u32) -> bool {
        let mut table = self.inner.hits.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        // Removing stale clients scans the whole table, so it runs once per
        // window rather than on every request. An old entry is still reset when
        // its client next appears, so the limit does not change.
        if now.duration_since(table.pruned_at) >= self.inner.window {
            let window = self.inner.window;
            table
                .counts
                .retain(|_, (start, _)| now.duration_since(*start) < window);
            table.pruned_at = now;
        }
        let entry = table.counts.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.inner.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= max
    }
}

/// Login and first-run setup attempts allowed per client per window. Argon2
/// makes each guess slow, but this bound stops a steady guessing run.
pub const LOGIN_MAX_DEFAULT: u32 = 20;

/// Client key for the limiter: the direct peer address, or, when
/// `trust_proxy` is set, the reverse-proxy-forwarded IP when present.
pub fn client_key(req: &Request, trust_proxy: bool) -> String {
    if trust_proxy
        && let Some(ip) = req
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
    if path == "/api/auth/login" || path.starts_with("/api/setup") {
        "login"
    } else {
        "api"
    }
}

/// Rate limiting middleware; use with
/// `axum::middleware::from_fn_with_state(limiter, rate_limit_fn)`.
/// Login and setup get a strict bucket, everything else the general one.
pub async fn rate_limit_fn(
    State(limiter): State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let bucket = bucket_for(req.uri().path());
    let key = format!("{bucket}:{}", client_key(&req, limiter.inner.trust_proxy));
    let max = if bucket == "login" {
        limiter.inner.login_max
    } else {
        limiter.inner.max
    };
    if limiter.check_with_max(&key, max) {
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
    fn forwarded_header_only_trusted_behind_proxy() {
        let req = || {
            Request::builder()
                .header("x-forwarded-for", "203.0.113.9, 10.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        assert_eq!(client_key(&req(), true), "203.0.113.9");
        assert_eq!(client_key(&req(), false), "unknown");
    }

    #[test]
    fn bucket_selection() {
        assert_eq!(bucket_for("/api/auth/login"), "login");
        assert_eq!(bucket_for("/api/setup"), "login");
        assert_eq!(bucket_for("/api/auth/me"), "api");
        assert_eq!(bucket_for("/api/notes"), "api");
    }

    #[test]
    fn login_bucket_has_its_own_cap() {
        let rl = RateLimiter::new(240, Duration::from_secs(60));
        for _ in 0..LOGIN_MAX_DEFAULT {
            assert!(rl.check_with_max("login:ip", LOGIN_MAX_DEFAULT));
        }
        assert!(!rl.check_with_max("login:ip", LOGIN_MAX_DEFAULT));
        assert!(rl.check("api:ip"), "the general bucket is unaffected");
    }
}
