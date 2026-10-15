// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — a dashboard of metric cards.
//!
//! Readings are recorded per metric (for example `power_kwh` or `water_l`).
//! The overview endpoint groups them in SQL and returns one card per metric
//! with its latest value, its 7-day average and how many readings it has.
//! To adapt it, change what the cards show in `api::overview` and the page.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_ui_route};

pub mod api;
pub mod page;

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_readings (
        id          INTEGER PRIMARY KEY,
        metric      TEXT    NOT NULL,
        value       REAL    NOT NULL,
        recorded_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS <NAME>_readings_metric ON <NAME>_readings (metric, recorded_at);
"#;

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
