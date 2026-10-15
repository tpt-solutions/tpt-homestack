// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — fetch a URL on a schedule and keep what came back.
//!
//! An admin sets a source URL on the page. Once an hour the job fetches it
//! (10 second timeout, at most 64 KiB kept) and stores the status, the body
//! and any error. Use it for a status page, a price feed, or an API you poll.
//!
//! Security: the URL is chosen by an admin and the server makes the request,
//! so it can reach anything the server can reach, including internal
//! addresses. Only point it at sources you trust.

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

    CREATE TABLE IF NOT EXISTS <NAME>_fetches (
        id         INTEGER PRIMARY KEY,
        url        TEXT    NOT NULL,
        status     INTEGER,
        body       TEXT    NOT NULL DEFAULT '',
        error      TEXT,
        fetched_at INTEGER NOT NULL
    );
"#;

/// Runs one hour after boot, then every hour. Does nothing until a URL is set.
#[bp_job(every = "1h")]
pub async fn fetch_source(db: DbPool) -> Result<(), ApiError> {
    api::run_fetch(&db).await?;
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
