// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <TITLE> — a tpt-homestack blueprint: a list of records you can add,
//! view and delete.
//!
//! Starting point for "keep a list of things" apps: inventory, contacts,
//! bookmarks, a logbook. To adapt it, change the table columns, the
//! `Record` shape and the form fields. The routes and page follow the same
//! pattern, so the change stays in one place.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_route, bp_ui_route};

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_records (
        id         INTEGER PRIMARY KEY,
        title      TEXT    NOT NULL,
        notes      TEXT    NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL
    );
"#;

/// Length limits stop a single request from storing a huge blob.
const MAX_TITLE: usize = 200;
const MAX_NOTES: usize = 5_000;
/// Page size for the list; add paging when a user needs more than this.
const LIST_LIMIT: i64 = 500;

#[derive(serde::Serialize)]
pub struct Record {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub created_at: i64,
}

#[derive(serde::Deserialize)]
pub struct NewRecord {
    pub title: String,
    #[serde(default)]
    pub notes: String,
}

#[bp_route(get, "/api/<NAME>/records")]
pub async fn list_records(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, title, notes, created_at FROM <NAME>_records ORDER BY id DESC LIMIT ?1",
    )
    .bind(LIST_LIMIT)
    .fetch_all(&db)
    .await?;
    let records: Vec<Record> = rows
        .into_iter()
        .map(|(id, title, notes, created_at)| Record { id, title, notes, created_at })
        .collect();
    Ok(Json(records))
}

#[bp_route(post, "/api/<NAME>/records")]
pub async fn create_record(
    State(db): State<DbPool>,
    Json(body): Json<NewRecord>,
) -> Result<impl IntoResponse, ApiError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("title must not be empty"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(ApiError::bad_request("title is too long"));
    }
    if body.notes.chars().count() > MAX_NOTES {
        return Err(ApiError::bad_request("notes are too long"));
    }

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_records (title, notes, created_at) VALUES (?1, ?2, ?3) RETURNING id",
    )
    .bind(title)
    .bind(body.notes.trim())
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[bp_route(delete, "/api/<NAME>/records/{id}")]
pub async fn delete_record(
    State(db): State<DbPool>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let result = sqlx::query("DELETE FROM <NAME>_records WHERE id = ?1")
        .bind(id)
        .execute(&db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("record not found"));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
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
        PAGE_BODY,
    )
}

const PAGE_BODY: &str = r##"
<div class="card">
  <h2 class="mt0">Add a record</h2>
  <label for="title">Title</label>
  <input id="title" maxlength="200" placeholder="What is it?">
  <label for="notes">Notes</label>
  <textarea id="notes" rows="3" maxlength="5000" placeholder="Optional"></textarea>
  <div class="row mt8">
    <button data-click="addRecord">Add</button>
  </div>
  <div id="msg"></div>
</div>
<div class="card">
  <h2 class="mt0">Records</h2>
  <div id="list"><p class="muted">Loading…</p></div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
function showError(e) {
  document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>';
}
async function load() {
  try {
    const rows = await api('/api/<NAME>/records', 'GET');
    const list = document.getElementById('list');
    if (rows.length === 0) { list.innerHTML = '<p class="muted">Nothing yet.</p>'; return; }
    list.innerHTML = rows.map(r =>
      '<div class="card">' +
        '<div class="spread"><strong>' + esc(r.title) + '</strong>' +
        '<button class="secondary" data-click="removeRecord:' + Number(r.id) + '">Delete</button></div>' +
        (r.notes ? '<p class="muted s-white-space-pre-wrap">' + esc(r.notes) + '</p>' : '') +
      '</div>'
    ).join('');
  } catch (e) { showError(e); }
}
async function addRecord() {
  const title = document.getElementById('title').value;
  const notes = document.getElementById('notes').value;
  try {
    await api('/api/<NAME>/records', 'POST', {title, notes});
    document.getElementById('title').value = '';
    document.getElementById('notes').value = '';
    document.getElementById('msg').innerHTML = '';
    await load();
  } catch (e) { showError(e); }
}
async function removeRecord(id) {
  if (!confirm('Delete this record?')) return;
  try { await api('/api/<NAME>/records/' + id, 'DELETE'); await load(); }
  catch (e) { showError(e); }
}
load();
</script>"##;

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
