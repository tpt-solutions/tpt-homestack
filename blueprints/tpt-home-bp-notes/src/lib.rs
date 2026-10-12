// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home-bp-notes` — the Digital Brain.
//!
//! Markdown notes with tags and FTS5 full-text search. This blueprint is the
//! reference for the "Dead Simple" authoring experience: everything the
//! platform needs is declared with four attribute macros.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_route, bp_ui_route};

pub mod api;
pub mod page;
pub mod safe_render;

/// Notes blueprint metadata.
#[blueprint(id = "notes", name = "Notes")]
pub struct NotesBlueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS notes (
        id         INTEGER PRIMARY KEY,
        title      TEXT    NOT NULL DEFAULT '',
        body       TEXT    NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS notes_tags (
        id   INTEGER PRIMARY KEY,
        name TEXT    NOT NULL UNIQUE COLLATE NOCASE
    );

    CREATE TABLE IF NOT EXISTS notes_note_tags (
        note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        tag_id  INTEGER NOT NULL REFERENCES notes_tags(id) ON DELETE CASCADE,
        PRIMARY KEY (note_id, tag_id)
    );

    CREATE INDEX IF NOT EXISTS idx_notes_updated ON notes(updated_at DESC);

    CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
        title, body, content='notes', content_rowid='id'
    );

    CREATE TRIGGER IF NOT EXISTS notes_fts_ai AFTER INSERT ON notes BEGIN
        INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
    END;
    CREATE TRIGGER IF NOT EXISTS notes_fts_ad AFTER DELETE ON notes BEGIN
        INSERT INTO notes_fts(notes_fts, rowid, title, body)
        VALUES ('delete', old.id, old.title, old.body);
    END;
    CREATE TRIGGER IF NOT EXISTS notes_fts_au AFTER UPDATE ON notes BEGIN
        INSERT INTO notes_fts(notes_fts, rowid, title, body)
        VALUES ('delete', old.id, old.title, old.body);
        INSERT INTO notes_fts(rowid, title, body) VALUES (new.id, new.title, new.body);
    END;
"#;

#[bp_route(get, "/api/notes")]
async fn list_items(
    State(db): State<DbPool>,
    axum::extract::Query(q): axum::extract::Query<api::ListQuery>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(api::list(&db, q).await?))
}

#[bp_route(post, "/api/notes")]
async fn create_item(
    State(db): State<DbPool>,
    Json(body): Json<api::NoteInput>,
) -> Result<impl IntoResponse, ApiError> {
    let note = api::create(&db, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(note)))
}

#[bp_route(get, "/api/notes/{id}")]
async fn get_item(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(api::get(&db, id).await?))
}

#[bp_route(put, "/api/notes/{id}")]
async fn update_item(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(body): Json<api::NoteInput>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(api::update(&db, id, body).await?))
}

#[bp_route(delete, "/api/notes/{id}")]
async fn delete_item(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    api::delete(&db, id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[bp_route(get, "/api/notes/{id}/render")]
async fn render_item(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Html(api::render(&db, id).await?))
}

#[bp_ui_route("/notes", name = "Notes")]
async fn notes_view(axum::Extension(user): axum::Extension<AuthUser>) -> impl IntoResponse {
    page::notes_page(&user)
}

/// Link anchor for the binary: see `tpt-home-cli`'s `BLUEPRINT_LINK_ANCHORS`.
/// Never called at runtime; its address forces the linker to keep this
/// crate's registration statics.
#[inline(never)]
pub fn __link_anchor() -> &'static str {
    env!("CARGO_PKG_NAME")
}
