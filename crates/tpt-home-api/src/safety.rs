// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Safe defaults on protected routes, in one middleware:
//!
//! - A user whose password an admin set must choose their own before using
//!   the app. Until then, pages redirect to Settings and API calls get a 403,
//!   except the calls needed to change the password and sign out.
//! - While the server is exposed beyond this machine (see
//!   `Config::exposed`), every page shows a warning, and so does a pending
//!   password change.
//!
//! The notices are added to the HTML after the page's `<main>` tag, so they
//! also appear on blueprint pages. The middleware runs inside the auth
//! resolver, so it sees the signed-in user.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};

use tpt_home_core::auth::user::AuthUser;
use tpt_home_core::db::DbPool;
use tpt_home_core::settings;

/// State for [`guard`]: the database for the password flag, and whether the
/// exposed warning applies.
pub struct Safety {
    pub db: DbPool,
    pub exposed: bool,
}

/// Paths a user may use while a password change is pending.
const ALLOWED_WHILE_CHANGING: &[&str] = &[
    "/settings",
    "/api/auth/change-password",
    "/api/auth/logout",
    "/api/auth/me",
    "/api/settings/server",
];

/// Marks where the page content starts. Every page from `tpt_home_ui::page`
/// has it, and the notices go right after it.
const MAIN_MARKER: &str = "<main class=\"main\">";
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;

const EXPOSED_NOTICE: &str = "<div class=\"msg error\" role=\"alert\"><strong>Exposed to the network.</strong> \
This server accepts connections from beyond this computer. Use HTTPS, a strong admin password, \
and a login in front of any public address. See the remote access guide.</div>";

const CHANGE_NOTICE: &str = "<div class=\"msg error\" role=\"alert\">An administrator set your password. \
<a href=\"/settings\">Choose your own password</a> to continue.</div>";

pub async fn guard(State(safety): State<Arc<Safety>>, req: Request, next: Next) -> Response {
    let user = req.extensions().get::<AuthUser>().cloned();
    let Some(user) = user else {
        return next.run(req).await;
    };
    let pending = settings::must_change_password(&safety.db, user.id).await;

    if pending && !ALLOWED_WHILE_CHANGING.contains(&req.uri().path()) {
        let wants_html = req
            .headers()
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html"));
        if wants_html {
            return Redirect::to("/settings").into_response();
        }
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":\"password change required\"}",
        )
            .into_response();
    }

    let res = next.run(req).await;
    let mut notices = String::new();
    if safety.exposed {
        notices.push_str(EXPOSED_NOTICE);
    }
    if pending {
        notices.push_str(CHANGE_NOTICE);
    }
    if notices.is_empty() || !is_html_page(&res) {
        return res;
    }
    add_notices(res, &notices).await
}

fn is_html_page(res: &Response) -> bool {
    res.status() == StatusCode::OK
        && res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html"))
}

/// Insert `notices` after the page's main tag. A page without the tag is
/// returned unchanged.
async fn add_notices(res: Response, notices: &str) -> Response {
    let (mut parts, body) = res.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_PAGE_BYTES).await {
        Ok(b) => b,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let html = String::from_utf8_lossy(&bytes);
    let out = match html.find(MAIN_MARKER) {
        Some(at) => {
            let cut = at + MAIN_MARKER.len();
            format!("{}{}{}", &html[..cut], notices, &html[cut..])
        }
        None => html.into_owned(),
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CONTENT_LENGTH, HeaderValue::from(out.len()));
    Response::from_parts(parts, Body::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn notices_go_after_the_main_tag() {
        let page = Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_LENGTH, "40")
            .body(Body::from("<html><main class=\"main\">x</main>"))
            .unwrap();
        let out = add_notices(page, "<b>n</b>").await;
        let body = axum::body::to_bytes(out.into_body(), 1024).await.unwrap();
        assert_eq!(
            String::from_utf8_lossy(&body),
            "<html><main class=\"main\"><b>n</b>x</main>"
        );
    }

    #[tokio::test]
    async fn pages_without_the_marker_are_unchanged() {
        let page = Response::builder()
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from("<p>plain</p>"))
            .unwrap();
        let out = add_notices(page, "<b>n</b>").await;
        let body = axum::body::to_bytes(out.into_body(), 1024).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&body), "<p>plain</p>");
    }
}
