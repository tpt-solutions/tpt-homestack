// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — sensor events and alerts.
//!
//! Sensors, or a home-automation system such as Home Assistant, POST signed
//! events to a webhook. An admin creates the webhook on the page and sets it
//! as the feed. Each event is a JSON body with a `sensor` and a `state`:
//!
//! ```json
//! { "sensor": "front-door", "state": "open" }
//! ```
//!
//! Once a minute a job reads each new delivery once and raises two kinds of
//! alert: a door opened during quiet hours, and a sensor that has not reported
//! for too long. Quiet hours follow the server's local time zone; alert times
//! are shown in each viewer's own zone. Alerts appear on the page; this blueprint does
//! not send email or chat messages, so check the page or add a notifier.
//!
//! This is an aid, not a certified alarm system. Do not rely on it alone to
//! protect people or property.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_job, bp_migration, bp_ui_route};

pub mod api;
pub mod page;

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    -- One row per delivery read from the feed webhook.
    CREATE TABLE IF NOT EXISTS <NAME>_events (
        id          INTEGER PRIMARY KEY,
        delivery_id INTEGER NOT NULL UNIQUE,
        sensor      TEXT    NOT NULL,
        state       TEXT    NOT NULL,
        seen_at     INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS <NAME>_events_sensor ON <NAME>_events (sensor, id);

    CREATE TABLE IF NOT EXISTS <NAME>_alerts (
        id         INTEGER PRIMARY KEY,
        created_at INTEGER NOT NULL,
        sensor     TEXT    NOT NULL,
        kind       TEXT    NOT NULL,
        message    TEXT    NOT NULL
    );
"#;

/// Runs one minute after boot, then every minute.
#[bp_job(every = "1m")]
pub async fn check_sensors(db: DbPool) -> Result<(), ApiError> {
    api::run_checks(&db).await
}

#[bp_ui_route("/<NAME>", name = "<TITLE>")]
async fn <NAME>_page(axum::Extension(user): axum::Extension<AuthUser>) -> impl IntoResponse {
    page::render(&user)
}

/// Link anchor for the binary: `tpt-home-cli`'s `BLUEPRINT_LINK_ANCHORS`
/// takes its address so the linker keeps this crate's registration statics.
/// `tpt-home new` adds the matching entry; don't remove this function.
#[inline(never)]
pub fn __link_anchor() -> &'static str {
    env!("CARGO_PKG_NAME")
}
