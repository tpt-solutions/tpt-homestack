// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home-bp-drop` — Secure Drop.
//!
//! Temporary file sharing: stream uploads straight to disk (constant
//! memory, large-file safe), share links with optional password and expiry,
//! and a cron-style background task that deletes expired files.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_job, bp_migration, bp_route, bp_ui_route};

pub mod api;
pub mod cleanup;
pub mod page;

/// Drop blueprint metadata.
#[blueprint(id = "drop", name = "Drop")]
pub struct DropBlueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS drop_files (
        id            INTEGER PRIMARY KEY,
        filename      TEXT    NOT NULL,
        stored_path   TEXT    NOT NULL,
        size          INTEGER NOT NULL,
        mime          TEXT    NOT NULL DEFAULT 'application/octet-stream',
        share_token   TEXT    UNIQUE,
        share_password_hash TEXT,
        expires_at    INTEGER,
        created_at    INTEGER NOT NULL,
        owner_id      INTEGER REFERENCES users(id) ON DELETE SET NULL
    );
    CREATE INDEX IF NOT EXISTS idx_drop_expiry ON drop_files(expires_at);
"#;

#[bp_route(get, "/api/drop")]
async fn list_drops(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(api::list(&db).await?))
}

#[bp_route(post, "/api/drop/upload")]
async fn upload_drop(
    State(db): State<DbPool>,
    axum::Extension(user): axum::Extension<AuthUser>,
    multipart: axum::extract::Multipart,
) -> Result<impl IntoResponse, ApiError> {
    api::upload(&db, user.id, multipart).await
}

#[bp_route(get, "/api/drop/{id}/download")]
async fn download_drop(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    api::download(&db, id).await
}

#[bp_route(post, "/api/drop/{id}/share")]
async fn share_drop(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<api::ShareInput>,
) -> Result<impl IntoResponse, ApiError> {
    api::share(&db, id, body).await
}

/// Public share link: `GET /api/drop/shared/{token}?pw=...` (optional
/// password as a query parameter; see SECURITY.md for the trade-off note).
#[bp_route(get, "/api/drop/shared/{token}", public)]
async fn shared_download(
    State(db): State<DbPool>,
    axum::extract::Path(token): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<api::SharePwQuery>,
) -> Result<axum::response::Response, ApiError> {
    api::shared(&db, token, q).await
}

#[bp_route(delete, "/api/drop/{id}")]
async fn delete_drop(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    api::delete(&db, id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[bp_route(post, "/api/drop/cleanup")]
async fn cleanup_now(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let removed = api::cleanup_now(&db).await?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[bp_ui_route("/drop", name = "Drop")]
async fn drop_view(axum::Extension(user): axum::Extension<AuthUser>) -> impl IntoResponse {
    page::drop_page(&user)
}

/// Delete expired shares and their files, once a minute.
#[bp_job(every = "1m", name = "drop-expiry-cleanup")]
async fn expire_files(db: DbPool) -> Result<(), ApiError> {
    cleanup::sweep(&db).await?;
    Ok(())
}

/// Link anchor for the binary: see `tpt-home-cli`'s `BLUEPRINT_LINK_ANCHORS`.
/// Never called at runtime; its address forces the linker to keep this
/// crate's registration statics.
#[inline(never)]
pub fn __link_anchor() -> &'static str {
    env!("CARGO_PKG_NAME")
}
