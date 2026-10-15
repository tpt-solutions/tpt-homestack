// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Notification channels: named outbound targets an admin configures once,
//! such as a chat webhook. A plugin that declares `notify_channels` and is
//! approved for `notify` may send to them with `tpt.notify(channel, text)`.
//!
//! Admin routes live under `/api/notify/channels`. The channel URL is stored
//! as plain text, because it usually carries its own secret, the same as
//! webhook secrets. The list never returns the URL. A delivery outcome is
//! kept on the channel (`last_sent_at`, `last_error`), so a broken channel is
//! visible without logs.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;
use tpt_home_core::db::DbPool;
use tpt_home_core::error::ApiError;

/// Longest channel URL accepted.
const MAX_URL: usize = 2_048;

inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "GET",
        path: "/api/notify/channels",
        role: Some("admin"),
        register: |router| {
            router.route(
                "/api/notify/channels",
                axum::routing::get(list_channels).post(save_channel).layer(tpt_home_core::admin_only!()),
            )
        },
    }
}

inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "DELETE",
        path: "/api/notify/channels/{name}",
        role: Some("admin"),
        register: |router| {
            router.route(
                "/api/notify/channels/{name}",
                axum::routing::delete(delete_channel).layer(tpt_home_core::admin_only!()),
            )
        },
    }
}

/// Body of `POST /api/notify/channels`.
#[derive(Debug, Deserialize)]
pub struct ChannelBody {
    pub name: String,
    pub url: String,
}

/// `GET /api/notify/channels` (admin): names and delivery state, no URLs.
pub async fn list_channels(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<(String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT name, created_at, last_sent_at, last_error FROM notification_channels ORDER BY name",
    )
    .fetch_all(&db)
    .await?;
    let channels: Vec<_> = rows
        .into_iter()
        .map(|(name, created_at, last_sent_at, last_error)| {
            json!({
                "name": name,
                "created_at": created_at,
                "last_sent_at": last_sent_at,
                "last_error": last_error,
            })
        })
        .collect();
    Ok(Json(channels))
}

/// `POST /api/notify/channels` (admin): create a channel, or replace its URL.
/// Replacing clears the last delivery state.
pub async fn save_channel(
    State(db): State<DbPool>,
    Json(body): Json<ChannelBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !valid_name(&body.name) {
        return Err(ApiError::bad_request(
            "name must be 1-50 characters: lowercase letters, digits, '_' or '-'",
        ));
    }
    if body.name == crate::email::EMAIL_CHANNEL {
        return Err(ApiError::bad_request(
            "the email channel is set up with the email account (/api/notify/email)",
        ));
    }
    if !valid_url(&body.url) {
        return Err(ApiError::bad_request(
            "url must be an http:// or https:// address without spaces (max 2048 characters)",
        ));
    }
    sqlx::query(
        "INSERT INTO notification_channels (name, url, created_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (name) DO UPDATE SET
             url = excluded.url,
             created_at = excluded.created_at,
             last_sent_at = NULL,
             last_error = NULL",
    )
    .bind(&body.name)
    .bind(&body.url)
    .bind(unix_now())
    .execute(&db)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "name": body.name }))))
}

/// `DELETE /api/notify/channels/{name}` (admin). Plugins that name the channel
/// get `false` from `tpt.notify` afterwards.
pub async fn delete_channel(
    State(db): State<DbPool>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if name == crate::email::EMAIL_CHANNEL {
        return Err(ApiError::bad_request(
            "remove the email account (DELETE /api/notify/email) to delete the email channel",
        ));
    }
    let result = sqlx::query("DELETE FROM notification_channels WHERE name = ?1")
        .bind(&name)
        .execute(&db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found(format!("no channel '{name}'")));
    }
    Ok(Json(json!({ "ok": true })))
}

/// 1-50 characters: lowercase letters, digits, `_` or `-`. The name is part
/// of the plugin's manifest, so it stays to the same plain set.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 50
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// `http://` or `https://`, no spaces or control characters, bounded length.
fn valid_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && url.len() <= MAX_URL
        && url.len() > "https://".len()
        && !url.chars().any(|c| c.is_control() || c == ' ')
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
