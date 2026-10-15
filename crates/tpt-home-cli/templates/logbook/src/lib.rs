// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — a dated log of numbers and notes.
//!
//! Use it for meter readings, service history, weigh-ins, or anything you
//! record once in a while and want totals for. Entries carry a calendar
//! date, a label and a value; the summary gives count, total and average.
//! To adapt it, change the table columns, `api::NewEntry` and the form.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_ui_route};

pub mod api;
pub mod page;

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_entries (
        id         INTEGER PRIMARY KEY,
        logged_on  TEXT    NOT NULL,
        label      TEXT    NOT NULL,
        value      REAL    NOT NULL,
        notes      TEXT    NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS <NAME>_entries_day ON <NAME>_entries (logged_on);
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
