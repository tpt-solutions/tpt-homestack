// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — a blueprint with a background job.
//!
//! People record events through the page; once an hour the job counts the
//! events from the last seven days and stores that as the day's snapshot.
//! Replace the body of `take_snapshot` with your own work (pull data, tidy
//! up, send a report) and change `every` to set how often it runs.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_job, bp_migration, bp_ui_route};

pub mod api;
pub mod page;

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_events (
        id         INTEGER PRIMARY KEY,
        note       TEXT    NOT NULL,
        created_at INTEGER NOT NULL
    );

    -- One row per day (days since the Unix epoch). A later run on the same
    -- day overwrites the earlier one, so the row always holds the latest value.
    CREATE TABLE IF NOT EXISTS <NAME>_snapshots (
        id         INTEGER PRIMARY KEY,
        day        INTEGER NOT NULL UNIQUE,
        events_7d  INTEGER NOT NULL,
        taken_at   INTEGER NOT NULL
    );
"#;

/// Runs one hour after boot, then every hour.
#[bp_job(every = "1h")]
pub async fn take_snapshot(db: DbPool) -> Result<(), ApiError> {
    let now = api::unix_now();
    let day = now / 86_400;
    let week_ago = now - 7 * 86_400;

    let (events,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM <NAME>_events WHERE created_at >= ?1")
            .bind(week_ago)
            .fetch_one(&db)
            .await?;
    sqlx::query(
        "INSERT INTO <NAME>_snapshots (day, events_7d, taken_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(day) DO UPDATE SET events_7d = excluded.events_7d, taken_at = excluded.taken_at",
    )
    .bind(day)
    .bind(events)
    .bind(now)
    .execute(&db)
    .await?;
    Ok(())
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
