// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Core auth API: login, logout, session info, password change and the
//! first-run admin bootstrap (`/api/setup`).

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use tpt_home_core::auth::session::{self as sessions, COOKIE_NAME};
use tpt_home_core::auth::user::{AuthUser, Role, authenticate, count_users, create_user};
use tpt_home_core::prelude::*;
use tpt_home_core::{auth, config};

#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

fn session_cookie(token: &str, max_age_secs: i64, secure: bool) -> HeaderValue {
    let mut cookie =
        format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}");
    if secure {
        cookie.push_str("; Secure");
    }
    // HeaderValue::from_str only rejects non-visible-ASCII; our token is hex.
    HeaderValue::from_str(&cookie).expect("session cookie is valid header value")
}

/// POST /api/auth/login — public (rate-limited). Sets the session cookie.
pub async fn login(
    State(db): State<DbPool>,
    Json(creds): Json<Credentials>,
) -> Result<impl IntoResponse, ApiError> {
    // Checked before the password, so a locked name costs no hashing.
    if auth::lockout::locked_for(&creds.username).is_some() {
        return Err(ApiError::TooManyRequests);
    }
    let user = match authenticate(&db, &creds.username, &creds.password).await {
        Ok(user) => {
            auth::lockout::record_success(&creds.username);
            user
        }
        Err(_) => {
            auth::lockout::record_failure(&creds.username);
            return Err(ApiError::Unauthorized);
        }
    };
    let token = auth::session::create_session(&db, user.id, config::global().session_ttl()).await?;
    let cookie = session_cookie(
        &token,
        (config::global().session_ttl().as_secs()) as i64,
        config::global().secure_cookies,
    );
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(json!({
            "ok": true,
            "username": user.username,
            "role": user.role().as_str(),
        })),
    ))
}

/// POST /api/auth/logout — revokes the current session and clears the cookie.
pub async fn logout(
    State(db): State<DbPool>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(token) = sessions::parse_session_cookie(&headers) {
        sessions::revoke_session(&db, &token).await?;
    }
    let clear = HeaderValue::from_static("tpt_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    Ok(([(header::SET_COOKIE, clear)], Json(json!({ "ok": true }))))
}

/// GET /api/auth/me — identity of the caller.
pub async fn me(Extension(user): Extension<AuthUser>) -> impl IntoResponse {
    Json(json!({
        "id": user.id,
        "username": user.username,
        "role": user.role.as_str(),
    }))
}

#[derive(Deserialize)]
pub struct ChangePassword {
    pub current: String,
    pub new_password: String,
}

/// POST /api/auth/change-password — verifies the current password, sets the
/// new one and revokes all sessions of the user (including this one).
pub async fn change_password(
    State(db): State<DbPool>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<ChangePassword>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authenticate(&db, &user.username, &body.current)
        .await
        .map_err(|_| ApiError::Unauthorized)?;
    auth::user::set_password(&db, user.id, &body.new_password).await?;
    tpt_home_core::settings::set_must_change_password(&db, user.id, false).await?;
    Ok(Json(json!({ "ok": true, "relogin": true })))
}

/// POST /api/setup — bootstrap the first admin. Only works while the instance
/// has zero users; afterwards it is a 404 (no information leak).
pub async fn setup(
    State(db): State<DbPool>,
    Json(creds): Json<Credentials>,
) -> Result<impl IntoResponse, ApiError> {
    if count_users(&db).await? > 0 {
        return Err(ApiError::NotFound("setup already completed".into()));
    }
    let admin = create_user(&db, &creds.username, &creds.password, Role::Admin).await?;
    let token =
        auth::session::create_session(&db, admin.id, config::global().session_ttl()).await?;
    let cookie = session_cookie(
        &token,
        (config::global().session_ttl().as_secs()) as i64,
        config::global().secure_cookies,
    );
    let mut res = (
        StatusCode::CREATED,
        Json(json!({ "ok": true, "username": admin.username })),
    )
        .into_response();
    res.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(res)
}
