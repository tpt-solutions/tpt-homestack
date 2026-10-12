// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <NAME> — a tpt-homestack blueprint.
//!
//! This is the whole platform integration surface: one metadata macro, one
//! SQL migration, routes, and a UI page. Delete what you don't need and
//! write your business logic; the macros do the wiring.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_route, bp_ui_route};

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_items (
        id         INTEGER PRIMARY KEY,
        title      TEXT    NOT NULL,
        created_at INTEGER NOT NULL
    );
"#;

#[bp_route(get, "/api/<NAME>/items")]
pub async fn list_items(
    State(db): State<DbPool>,
) -> Result<impl IntoResponse, ApiError> {
    let items: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, title FROM <NAME>_items ORDER BY id DESC")
            .fetch_all(&db)
            .await?;
    Ok(Json(items))
}

#[bp_route(post, "/api/<NAME>/items")]
pub async fn create_item(
    State(db): State<DbPool>,
    Json(body): Json<CreateItem>,
) -> Result<impl IntoResponse, ApiError> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_items (title, created_at) VALUES (?1, ?2) RETURNING id",
    )
    .bind(body.title.trim())
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(serde::Deserialize)]
pub struct CreateItem {
    pub title: String,
}

#[bp_ui_route("/<NAME>", name = "<TITLE>")]
pub async fn <NAME>_page(
    axum::Extension(user): axum::Extension<AuthUser>,
) -> impl IntoResponse {
    let view = tpt_home_ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    tpt_home_ui::page(
        "<TITLE>",
        &registry::nav_entries(),
        "/<NAME>",
        &view,
        "<h2>Welcome to <TITLE></h2><p class=\"muted\">Replace this page with your UI.</p>",
    )
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Link anchor for the binary: `tpt-home-cli`'s `BLUEPRINT_LINK_ANCHORS`
/// takes its address so the linker keeps this crate's registration statics.
/// `tpt-home new` adds the matching entry; don't remove this function.
#[inline(never)]
pub fn __link_anchor() -> &'static str {
    env!("CARGO_PKG_NAME")
}
