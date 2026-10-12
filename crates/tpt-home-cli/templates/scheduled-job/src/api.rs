// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! HTTP API: record events, and read the events and snapshots the job makes.

use axum::http::StatusCode;
use tpt_home_core::prelude::*;
use tpt_home_macros::bp_route;

/// Longest note accepted, in characters.
const MAX_NOTE: usize = 500;

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Event {
    pub id: i64,
    pub note: String,
    pub created_at: i64,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Snapshot {
    pub day: i64,
    pub events_7d: i64,
    pub taken_at: i64,
}

#[derive(serde::Deserialize)]
pub struct NewEvent {
    pub note: String,
}

#[bp_route(get, "/api/<NAME>/events")]
async fn list_events(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<Event> = sqlx::query_as(
        "SELECT id, note, created_at FROM <NAME>_events ORDER BY id DESC LIMIT 50",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

#[bp_route(post, "/api/<NAME>/events")]
async fn add_event(
    State(db): State<DbPool>,
    Json(body): Json<NewEvent>,
) -> Result<impl IntoResponse, ApiError> {
    let note = body.note.trim();
    if note.is_empty() || note.chars().count() > MAX_NOTE {
        return Err(ApiError::bad_request("note must be 1-500 characters"));
    }
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_events (note, created_at) VALUES (?1, ?2) RETURNING id",
    )
    .bind(note)
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[bp_route(get, "/api/<NAME>/snapshots")]
async fn list_snapshots(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<Snapshot> = sqlx::query_as(
        "SELECT day, events_7d, taken_at FROM <NAME>_snapshots ORDER BY day DESC LIMIT 60",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
