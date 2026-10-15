// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Sensor events, the two alert rules, and the settings they use.
//!
//! Sensors post signed events to a webhook (see `tpt_home_core::webhook`).
//! The job in `lib.rs` reads each new delivery once, records it, and checks
//! two rules: a door opened during quiet hours, and a sensor that has not
//! reported for too long. Quiet hours use the server's local time zone (the
//! settings page shows its current time and offset).

use axum::http::StatusCode;
use tpt_home_core::prelude::*;
use tpt_home_macros::bp_route;

/// Most deliveries one job run reads, so a backlog is worked off over runs.
const MAX_PER_RUN: i64 = 500;
const DEFAULT_QUIET_FROM: i64 = 23;
const DEFAULT_QUIET_TO: i64 = 6;
const DEFAULT_SILENT_HOURS: i64 = 24;
/// Longest sensor name accepted.
const MAX_SENSOR: usize = 64;

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Sensor {
    pub sensor: String,
    pub state: String,
    pub seen_at: i64,
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct Alert {
    pub id: i64,
    pub created_at: i64,
    pub sensor: String,
    pub kind: String,
    pub message: String,
}

#[derive(serde::Serialize)]
pub struct Settings {
    pub webhook_name: String,
    pub quiet_from: i64,
    pub quiet_to: i64,
    pub silent_after_hours: i64,
}

#[derive(serde::Deserialize)]
pub struct SettingsInput {
    pub webhook_name: String,
    pub quiet_from: i64,
    pub quiet_to: i64,
    pub silent_after_hours: i64,
}

#[bp_route(get, "/api/<NAME>/status")]
async fn status(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let settings = load_settings(&db).await?;
    let sensors: Vec<Sensor> = sqlx::query_as(
        "SELECT e.sensor AS sensor, e.state AS state, e.seen_at AS seen_at
         FROM <NAME>_events e
         JOIN (SELECT sensor, MAX(id) AS last_id FROM <NAME>_events GROUP BY sensor) m
           ON m.last_id = e.id
         ORDER BY e.sensor",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(serde_json::json!({
        "settings": settings,
        "sensors": sensors,
        "server_clock": server_clock(),
    })))
}

#[bp_route(get, "/api/<NAME>/alerts")]
async fn alerts(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<Alert> = sqlx::query_as(
        "SELECT id, created_at, sensor, kind, message FROM <NAME>_alerts
         ORDER BY id DESC LIMIT 100",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

#[bp_route(post, "/api/<NAME>/settings", role = "admin")]
async fn save_settings(
    State(db): State<DbPool>,
    Json(body): Json<SettingsInput>,
) -> Result<impl IntoResponse, ApiError> {
    let name = body.webhook_name.trim().to_string();
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM webhooks WHERE name = ?1")
        .bind(&name)
        .fetch_optional(&db)
        .await?;
    if exists.is_none() {
        return Err(ApiError::bad_request(
            "no webhook with that name; create it first on the Webhooks admin page",
        ));
    }
    if !(0..=23).contains(&body.quiet_from) || !(0..=23).contains(&body.quiet_to) {
        return Err(ApiError::bad_request(
            "quiet hours must be whole hours from 0 to 23 (server local time)",
        ));
    }
    if !(1..=720).contains(&body.silent_after_hours) {
        return Err(ApiError::bad_request("silent-after must be between 1 and 720 hours"));
    }
    put_setting(&db, "webhook_name", &name).await?;
    put_setting(&db, "quiet_from", &body.quiet_from.to_string()).await?;
    put_setting(&db, "quiet_to", &body.quiet_to.to_string()).await?;
    put_setting(&db, "silent_after_hours", &body.silent_after_hours.to_string()).await?;
    Ok((StatusCode::OK, Json(load_settings(&db).await?)))
}

/// One pass of the rules. Called once a minute by the job.
pub async fn run_checks(db: &DbPool) -> Result<(), ApiError> {
    let Some(name) = get_setting(db, "webhook_name").await? else {
        return Ok(());
    };
    if name.is_empty() {
        return Ok(());
    }
    let webhook: Option<(i64,)> = sqlx::query_as("SELECT id FROM webhooks WHERE name = ?1")
        .bind(&name)
        .fetch_optional(db)
        .await?;
    let Some((webhook_id,)) = webhook else {
        return Ok(());
    };

    let cursor: i64 = get_setting(db, "cursor")
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let settings = load_settings(db).await?;
    let deliveries: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT id, received_at, body FROM webhook_deliveries
         WHERE webhook_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
    )
    .bind(webhook_id)
    .bind(cursor)
    .bind(MAX_PER_RUN)
    .fetch_all(db)
    .await?;

    let mut last_id = cursor;
    for (delivery_id, received_at, body) in deliveries {
        last_id = delivery_id;
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        let sensor = payload["sensor"].as_str().unwrap_or("").trim().to_string();
        let state = payload["state"].as_str().unwrap_or("").trim().to_string();
        if sensor.is_empty() || state.is_empty() || sensor.len() > MAX_SENSOR {
            continue;
        }

        sqlx::query(
            "INSERT INTO <NAME>_events (delivery_id, sensor, state, seen_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(delivery_id) DO NOTHING",
        )
        .bind(delivery_id)
        .bind(&sensor)
        .bind(&state)
        .bind(received_at)
        .execute(db)
        .await?;

        if state == "open"
            && in_quiet_hours(local_hour(received_at), settings.quiet_from, settings.quiet_to)
        {
            // The alert's own time is stored, and the page shows it in the viewer's zone.
            let message = format!("{sensor} opened during quiet hours");
            add_alert(db, received_at, &sensor, "quiet-open", &message).await?;
        }
    }
    if last_id > cursor {
        put_setting(db, "cursor", &last_id.to_string()).await?;
    }

    // Silence: a sensor whose newest event is older than the limit, and which
    // has not already been reported silent since that event.
    let now = unix_now();
    let limit = settings.silent_after_hours * 3600;
    let last_seen: Vec<(String, i64)> =
        sqlx::query_as("SELECT sensor, MAX(seen_at) FROM <NAME>_events GROUP BY sensor")
            .fetch_all(db)
            .await?;
    for (sensor, seen) in last_seen {
        if now - seen < limit {
            continue;
        }
        let reported: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM <NAME>_alerts
             WHERE sensor = ?1 AND kind = 'silent' AND created_at > ?2",
        )
        .bind(&sensor)
        .bind(seen)
        .fetch_optional(db)
        .await?;
        if reported.is_none() {
            let message = format!(
                "{sensor} has not reported for {} hours",
                (now - seen) / 3600
            );
            add_alert(db, now, &sensor, "silent", &message).await?;
        }
    }
    Ok(())
}

/// Hour of day (0-23) at `unix` in the server's local time zone. Quiet hours
/// are compared in this zone, so "23 to 6" means the household's night.
pub fn local_hour(unix: i64) -> i64 {
    use chrono::{Local, TimeZone, Timelike};
    Local
        .timestamp_opt(unix, 0)
        .single()
        .map(|t| t.hour() as i64)
        .unwrap_or(0)
}

/// True when `hour` falls inside the quiet window. The window wraps past
/// midnight when `from` is later than `to`; equal values disable it.
pub fn in_quiet_hours(hour: i64, from: i64, to: i64) -> bool {
    if from == to {
        return false;
    }
    if from < to {
        (from..to).contains(&hour)
    } else {
        hour >= from || hour < to
    }
}

/// The server's current local time and UTC offset, for the settings page.
fn server_clock() -> serde_json::Value {
    let now = chrono::Local::now();
    serde_json::json!({
        "local_time": now.format("%Y-%m-%d %H:%M").to_string(),
        "utc_offset": now.format("%:z").to_string(),
    })
}

async fn add_alert(
    db: &DbPool,
    at: i64,
    sensor: &str,
    kind: &str,
    message: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO <NAME>_alerts (created_at, sensor, kind, message) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(at)
    .bind(sensor)
    .bind(kind)
    .bind(message)
    .execute(db)
    .await?;
    Ok(())
}

async fn get_setting(db: &DbPool, key: &str) -> Result<Option<String>, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM <NAME>_settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|(v,)| v))
}

async fn put_setting(db: &DbPool, key: &str, value: &str) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO <NAME>_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

async fn load_settings(db: &DbPool) -> Result<Settings, ApiError> {
    let number = |v: Option<String>, default: i64| v.and_then(|s| s.parse().ok()).unwrap_or(default);
    Ok(Settings {
        webhook_name: get_setting(db, "webhook_name").await?.unwrap_or_default(),
        quiet_from: number(get_setting(db, "quiet_from").await?, DEFAULT_QUIET_FROM),
        quiet_to: number(get_setting(db, "quiet_to").await?, DEFAULT_QUIET_TO),
        silent_after_hours: number(
            get_setting(db, "silent_after_hours").await?,
            DEFAULT_SILENT_HOURS,
        ),
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::in_quiet_hours;

    #[test]
    fn quiet_window_wraps_past_midnight() {
        // 23:00 to 06:00.
        assert!(in_quiet_hours(23, 23, 6));
        assert!(in_quiet_hours(0, 23, 6));
        assert!(in_quiet_hours(5, 23, 6));
        assert!(!in_quiet_hours(6, 23, 6));
        assert!(!in_quiet_hours(12, 23, 6));
        // Same-day window 09:00 to 17:00.
        assert!(in_quiet_hours(9, 9, 17));
        assert!(!in_quiet_hours(17, 9, 17));
        // Equal bounds disable the rule.
        assert!(!in_quiet_hours(3, 5, 5));
    }

    #[test]
    fn local_hour_follows_the_server_zone() {
        // Whatever the server's zone is, the hour must match chrono's own
        // conversion of the same instant, not a fixed UTC offset.
        use chrono::{Local, TimeZone, Timelike};
        let unix = 1_700_000_000;
        let expected = Local.timestamp_opt(unix, 0).single().unwrap().hour() as i64;
        assert_eq!(super::local_hour(unix), expected);
    }
}
