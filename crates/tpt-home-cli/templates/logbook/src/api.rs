// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! HTTP API for the logbook: list, summary, add and delete entries.

use axum::http::StatusCode;
use tpt_home_core::prelude::*;
use tpt_home_macros::bp_route;

/// Longest label and notes accepted, in characters.
const MAX_LABEL: usize = 100;
const MAX_NOTES: usize = 2_000;
/// Most entries returned by the list endpoint.
const LIST_LIMIT: i64 = 200;

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Entry {
    pub id: i64,
    pub logged_on: String,
    pub label: String,
    pub value: f64,
    pub notes: String,
}

#[derive(serde::Deserialize)]
pub struct NewEntry {
    /// Calendar date as `YYYY-MM-DD`.
    pub logged_on: String,
    pub label: String,
    pub value: f64,
    #[serde(default)]
    pub notes: String,
}

#[bp_route(get, "/api/<NAME>/entries")]
async fn list_entries(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<Entry> = sqlx::query_as(
        "SELECT id, logged_on, label, value, notes FROM <NAME>_entries
         ORDER BY logged_on DESC, id DESC LIMIT ?1",
    )
    .bind(LIST_LIMIT)
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

#[bp_route(get, "/api/<NAME>/summary")]
async fn summary(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let (count, total): (i64, f64) =
        sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(value), 0.0) FROM <NAME>_entries")
            .fetch_one(&db)
            .await?;
    let latest: Option<(String, f64)> = sqlx::query_as(
        "SELECT logged_on, value FROM <NAME>_entries ORDER BY logged_on DESC, id DESC LIMIT 1",
    )
    .fetch_optional(&db)
    .await?;
    Ok(Json(serde_json::json!({
        "count": count,
        "total": total,
        "average": (count > 0).then(|| total / count as f64),
        "latest_on": latest.as_ref().map(|(day, _)| day.clone()),
        "latest_value": latest.map(|(_, value)| value),
    })))
}

#[bp_route(post, "/api/<NAME>/entries")]
async fn create_entry(
    State(db): State<DbPool>,
    Json(body): Json<NewEntry>,
) -> Result<impl IntoResponse, ApiError> {
    let label = body.label.trim();
    if label.is_empty() || label.chars().count() > MAX_LABEL {
        return Err(ApiError::bad_request("label must be 1-100 characters"));
    }
    if !is_valid_date(&body.logged_on) {
        return Err(ApiError::bad_request("date must be a real date as YYYY-MM-DD"));
    }
    if !body.value.is_finite() {
        return Err(ApiError::bad_request("value must be a number"));
    }
    if body.notes.chars().count() > MAX_NOTES {
        return Err(ApiError::bad_request("notes are too long"));
    }

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_entries (logged_on, label, value, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
    )
    .bind(&body.logged_on)
    .bind(label)
    .bind(body.value)
    .bind(body.notes.trim())
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[bp_route(delete, "/api/<NAME>/entries/{id}")]
async fn delete_entry(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let result = sqlx::query("DELETE FROM <NAME>_entries WHERE id = ?1")
        .bind(id)
        .execute(&db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("entry not found"));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// True only for real calendar dates written as `YYYY-MM-DD`.
pub fn is_valid_date(text: &str) -> bool {
    if !text.is_ascii() || text.len() != 10 {
        return false;
    }
    let bytes = text.as_bytes();
    let shape_ok = bytes.iter().enumerate().all(|(i, &c)| match i {
        4 | 7 => c == b'-',
        _ => c.is_ascii_digit(),
    });
    if !shape_ok {
        return false;
    }
    let year: u32 = text[0..4].parse().unwrap_or(0);
    let month: u32 = text[5..7].parse().unwrap_or(0);
    let day: u32 = text[8..10].parse().unwrap_or(0);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::is_valid_date;

    #[test]
    fn accepts_real_dates_only() {
        assert!(is_valid_date("2026-10-12"));
        assert!(is_valid_date("2024-02-29"));
        assert!(!is_valid_date("2025-02-29"));
        assert!(!is_valid_date("2026-13-01"));
        assert!(!is_valid_date("2026-00-10"));
        assert!(!is_valid_date("2026-10-00"));
        assert!(!is_valid_date("2026/10/12"));
        assert!(!is_valid_date("12-10-2026"));
        assert!(!is_valid_date("2026-10-1x"));
        assert!(!is_valid_date("２０２６-10-12"));
    }
}
