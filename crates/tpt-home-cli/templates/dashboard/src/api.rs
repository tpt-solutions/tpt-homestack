// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! HTTP API: record readings, and the grouped overview the cards are drawn from.

use axum::http::StatusCode;
use tpt_home_core::prelude::*;
use tpt_home_macros::bp_route;

/// Longest metric name accepted.
const MAX_METRIC: usize = 50;

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct MetricCard {
    pub metric: String,
    pub count: i64,
    /// Average over the last seven days; `None` if nothing was recorded then.
    pub avg_7d: Option<f64>,
    pub latest: Option<f64>,
    pub latest_at: Option<i64>,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Reading {
    pub id: i64,
    pub metric: String,
    pub value: f64,
    pub recorded_at: i64,
}

#[derive(serde::Deserialize)]
pub struct NewReading {
    pub metric: String,
    pub value: f64,
}

#[bp_route(get, "/api/<NAME>/overview")]
async fn overview(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let week_ago = unix_now() - 7 * 86_400;
    let cards: Vec<MetricCard> = sqlx::query_as(
        "SELECT r.metric AS metric,
                COUNT(*) AS count,
                AVG(CASE WHEN r.recorded_at >= ?1 THEN r.value END) AS avg_7d,
                (SELECT x.value FROM <NAME>_readings x WHERE x.metric = r.metric
                 ORDER BY x.recorded_at DESC, x.id DESC LIMIT 1) AS latest,
                MAX(r.recorded_at) AS latest_at
         FROM <NAME>_readings r
         GROUP BY r.metric
         ORDER BY r.metric",
    )
    .bind(week_ago)
    .fetch_all(&db)
    .await?;
    Ok(Json(cards))
}

#[bp_route(get, "/api/<NAME>/readings")]
async fn recent_readings(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<Reading> = sqlx::query_as(
        "SELECT id, metric, value, recorded_at FROM <NAME>_readings ORDER BY id DESC LIMIT 50",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

#[bp_route(post, "/api/<NAME>/readings")]
async fn add_reading(
    State(db): State<DbPool>,
    Json(body): Json<NewReading>,
) -> Result<impl IntoResponse, ApiError> {
    let metric = body.metric.trim();
    if metric.is_empty() || metric.len() > MAX_METRIC || !metric.chars().all(is_metric_char) {
        return Err(ApiError::bad_request(
            "metric must be 1-50 characters: lowercase letters, digits, _ or -",
        ));
    }
    if !body.value.is_finite() {
        return Err(ApiError::bad_request("value must be a number"));
    }
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_readings (metric, value, recorded_at) VALUES (?1, ?2, ?3) RETURNING id",
    )
    .bind(metric)
    .bind(body.value)
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

fn is_metric_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::is_metric_char;

    #[test]
    fn metric_names_are_simple_identifiers() {
        assert!("power_kwh-2".chars().all(is_metric_char));
        assert!(!"Power kWh".chars().all(is_metric_char));
        assert!(!"a/b".chars().all(is_metric_char));
    }
}
