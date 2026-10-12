// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! HTTP API: the admin-set source URL, a manual fetch, and the stored responses.

use std::time::Duration;

use axum::http::StatusCode;
use tpt_home_core::prelude::*;
use tpt_home_macros::bp_route;

/// Most response bytes kept per fetch.
const MAX_BODY: usize = 64 * 1024;
/// Longest source URL accepted.
const MAX_URL: usize = 2_048;
const SETTING_URL: &str = "source_url";

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Fetch {
    pub id: i64,
    pub url: String,
    pub status: Option<i64>,
    /// First 500 characters of the body, for the list view.
    pub body_preview: String,
    pub error: Option<String>,
    pub fetched_at: i64,
}

#[derive(serde::Deserialize)]
pub struct SourceSetting {
    /// Empty string clears the source.
    pub source_url: String,
}

#[bp_route(get, "/api/<NAME>/settings")]
async fn get_settings(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let url = source_url(&db).await?;
    Ok(Json(serde_json::json!({ "source_url": url })))
}

#[bp_route(post, "/api/<NAME>/settings", role = "admin")]
async fn set_settings(
    State(db): State<DbPool>,
    Json(body): Json<SourceSetting>,
) -> Result<impl IntoResponse, ApiError> {
    let url = body.source_url.trim();
    if url.is_empty() {
        sqlx::query("DELETE FROM <NAME>_settings WHERE key = ?1")
            .bind(SETTING_URL)
            .execute(&db)
            .await?;
        return Ok(Json(serde_json::json!({ "source_url": null })));
    }
    if !valid_url(url) {
        return Err(ApiError::bad_request(
            "source_url must start with http:// or https:// and contain no spaces",
        ));
    }
    sqlx::query(
        "INSERT INTO <NAME>_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(SETTING_URL)
    .bind(url)
    .execute(&db)
    .await?;
    Ok(Json(serde_json::json!({ "source_url": url })))
}

#[bp_route(get, "/api/<NAME>/fetches")]
async fn list_fetches(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<Fetch> = sqlx::query_as(
        "SELECT id, url, status, substr(body, 1, 500) AS body_preview, error, fetched_at
         FROM <NAME>_fetches ORDER BY id DESC LIMIT 20",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

#[bp_route(post, "/api/<NAME>/fetch-now", role = "admin")]
async fn fetch_now(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    match run_fetch(&db).await? {
        Some(id) => Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id })))),
        None => Err(ApiError::bad_request("set a source URL first")),
    }
}

/// Fetch the source once and store the result. `Ok(None)` if no URL is set.
/// Network failures are stored as a row with `error` set, not returned as errors.
pub async fn run_fetch(db: &DbPool) -> Result<Option<i64>, ApiError> {
    let Some(url) = source_url(db).await? else {
        return Ok(None);
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("tpt-homestack/<NAME>")
        .build()
        .map_err(|e| ApiError::internal(format!("http client: {e}")))?;

    let mut response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(e) => return store(db, &url, None, "", Some(e.to_string())).await.map(Some),
    };
    let status = response.status().as_u16() as i64;
    let mut body: Vec<u8> = Vec::new();
    let mut error = None;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let room = MAX_BODY.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if body.len() >= MAX_BODY {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    store(db, &url, Some(status), &text, error).await.map(Some)
}

async fn store(
    db: &DbPool,
    url: &str,
    status: Option<i64>,
    body: &str,
    error: Option<String>,
) -> Result<i64, ApiError> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_fetches (url, status, body, error, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
    )
    .bind(url)
    .bind(status)
    .bind(body)
    .bind(error)
    .bind(unix_now())
    .fetch_one(db)
    .await?;
    Ok(id)
}

async fn source_url(db: &DbPool) -> Result<Option<String>, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM <NAME>_settings WHERE key = ?1")
        .bind(SETTING_URL)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|(url,)| url))
}

fn valid_url(text: &str) -> bool {
    (text.starts_with("http://") || text.starts_with("https://"))
        && text.len() <= MAX_URL
        && !text.chars().any(char::is_whitespace)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::valid_url;

    #[test]
    fn only_plain_http_urls_are_accepted() {
        assert!(valid_url("https://example.com/status.json"));
        assert!(valid_url("http://192.168.1.10:8080/api"));
        assert!(!valid_url("ftp://example.com"));
        assert!(!valid_url("example.com"));
        assert!(!valid_url("https://exa mple.com"));
    }
}
