// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — a simple file inbox: upload, list, download and delete.
//!
//! Files are streamed to `data/uploads/<NAME>/` in constant memory, and the
//! size limit is `max_upload_mb` from the config. Only metadata lives in the
//! database. To adapt it, add columns (tags, owners) to the table and
//! extend `api::FileRow`.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_ui_route};

pub mod api;
pub mod page;

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_files (
        id            INTEGER PRIMARY KEY,
        original_name TEXT    NOT NULL,
        stored_name   TEXT    NOT NULL UNIQUE,
        size          INTEGER NOT NULL,
        mime          TEXT    NOT NULL,
        created_at    INTEGER NOT NULL
    );
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
