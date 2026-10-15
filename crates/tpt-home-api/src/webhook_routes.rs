// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Webhooks: admin-managed endpoints with a shared secret, and a public
//! receiver that accepts a signed POST and stores the body.
//!
//! Admin routes live under `/api/webhooks` (role `admin`). The receiver is
//! `POST /api/hooks/{name}`; it answers 401 for an unknown name and for a bad
//! signature alike, so callers cannot tell which names exist. Accepted bodies
//! are kept in `webhook_deliveries` for blueprints to read; the newest 200 per
//! webhook are kept.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;

use tpt_home_core::prelude::*;
use tpt_home_core::webhook;

/// Largest accepted body, in bytes.
const MAX_BODY: usize = 64 * 1024;
/// How far the signed timestamp may differ from the server clock.
const WINDOW_SECS: i64 = 300;
/// Deliveries kept per webhook; older ones are pruned on receipt.
const KEEP_DELIVERIES: i64 = 200;
/// Deliveries returned by the admin listing.
const LIST_DELIVERIES: i64 = 20;

#[derive(serde::Deserialize)]
pub struct NewWebhook {
    pub name: String,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct WebhookView {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub last_received_at: Option<i64>,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct DeliveryView {
    pub id: i64,
    pub received_at: i64,
    pub body: String,
}

/// `GET /api/webhooks`
pub async fn list(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<WebhookView> =
        sqlx::query_as("SELECT id, name, created_at, last_received_at FROM webhooks ORDER BY name")
            .fetch_all(&db)
            .await?;
    Ok(Json(rows))
}

/// `POST /api/webhooks`: returns the secret once. It cannot be shown again.
pub async fn create(
    State(db): State<DbPool>,
    Json(body): Json<NewWebhook>,
) -> Result<impl IntoResponse, ApiError> {
    let name = body.name.trim();
    if !valid_name(name) {
        return Err(ApiError::bad_request(
            "name must be 1-50 characters: lowercase letters, digits, _ or -",
        ));
    }
    let taken: Option<(i64,)> = sqlx::query_as("SELECT id FROM webhooks WHERE name = ?1")
        .bind(name)
        .fetch_optional(&db)
        .await?;
    if taken.is_some() {
        return Err(ApiError::Conflict(
            "a webhook with that name already exists".into(),
        ));
    }

    // Two v4 uuids = 256 bits of OS randomness.
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO webhooks (name, secret, created_at) VALUES (?1, ?2, ?3) RETURNING id",
    )
    .bind(name)
    .bind(&secret)
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "name": name,
            "secret": secret,
            "url": format!("/api/hooks/{name}"),
        })),
    ))
}

/// `DELETE /api/webhooks/{id}`: removes the secret and its deliveries.
pub async fn delete(
    State(db): State<DbPool>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let result = sqlx::query("DELETE FROM webhooks WHERE id = ?1")
        .bind(id)
        .execute(&db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("webhook not found"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/webhooks/{id}/deliveries`: the newest accepted bodies.
pub async fn deliveries(
    State(db): State<DbPool>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM webhooks WHERE id = ?1")
        .bind(id)
        .fetch_optional(&db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found("webhook not found"));
    }
    let rows: Vec<DeliveryView> = sqlx::query_as(
        "SELECT id, received_at, body FROM webhook_deliveries
         WHERE webhook_id = ?1 ORDER BY id DESC LIMIT ?2",
    )
    .bind(id)
    .bind(LIST_DELIVERIES)
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

/// `POST /api/hooks/{name}` (public). Headers: `X-Tpt-Timestamp` (Unix
/// seconds) and `X-Tpt-Signature` (hex HMAC-SHA256 of `"<timestamp>.<body>"`).
pub async fn receive(
    State(db): State<DbPool>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    if body.len() > MAX_BODY {
        return Err(ApiError::bad_request("body is larger than 64 KiB"));
    }
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, secret FROM webhooks WHERE name = ?1")
            .bind(&name)
            .fetch_optional(&db)
            .await?;
    // Unknown name and bad signature look the same to the caller.
    let Some((webhook_id, secret)) = row else {
        return Err(ApiError::Unauthorized);
    };

    let header = |key: &str| {
        headers
            .get(key)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
    };
    let verified = webhook::verify(
        secret.as_bytes(),
        header("x-tpt-timestamp"),
        header("x-tpt-signature"),
        &body,
        unix_now(),
        WINDOW_SECS,
    );
    if let Err(err) = verified {
        tracing::debug!(webhook = %name, reason = %err, "webhook rejected");
        return Err(ApiError::Unauthorized);
    }

    let text =
        std::str::from_utf8(&body).map_err(|_| ApiError::bad_request("body must be UTF-8 text"))?;
    let now = unix_now();
    let stored = sqlx::query(
        "INSERT INTO webhook_deliveries (webhook_id, received_at, body) VALUES (?1, ?2, ?3)",
    )
    .bind(webhook_id)
    .bind(now)
    .bind(text)
    .execute(&db)
    .await?;
    let delivery_id = stored.last_insert_rowid();
    sqlx::query("UPDATE webhooks SET last_received_at = ?2 WHERE id = ?1")
        .bind(webhook_id)
        .bind(now)
        .execute(&db)
        .await?;
    sqlx::query(
        "DELETE FROM webhook_deliveries WHERE webhook_id = ?1 AND id NOT IN (
             SELECT id FROM webhook_deliveries WHERE webhook_id = ?1 ORDER BY id DESC LIMIT ?2)",
    )
    .bind(webhook_id)
    .bind(KEEP_DELIVERIES)
    .execute(&db)
    .await?;

    // Blueprint handlers (`#[bp_webhook]`) run in their own tasks; the sender
    // gets its 202 without waiting for them.
    tpt_home_core::dispatch_webhook(
        &db,
        tpt_home_core::WebhookDelivery {
            id: delivery_id,
            webhook: name.clone(),
            received_at: now,
            body: text.to_string(),
        },
    );

    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

/// Names: 1-50 characters of lowercase letters, digits, `_` or `-`. The name
/// is part of the URL, so nothing else is allowed.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 50
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn names_fit_in_a_url_path() {
        assert!(valid_name("garage-door_1"));
        assert!(!valid_name(""));
        assert!(!valid_name("Garage"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("a b"));
        assert!(!valid_name(&"x".repeat(51)));
    }
}
