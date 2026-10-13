// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Server-rendered pages of the shell. All of these sit behind the auth
//! resolver (redirect to `/login` when unauthenticated) except the login and
//! setup pages themselves.

use axum::Extension;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Redirect};
use serde::Deserialize;

use tpt_home_core::auth::user::{AuthUser, count_users, list_users};
use tpt_home_core::prelude::*;
use tpt_home_core::registry;
use tpt_home_core::settings;
use tpt_home_ui as ui;

/// GET / — sends fresh instances to setup, everyone else to the dashboard.
pub async fn root(State(db): State<DbPool>) -> impl IntoResponse {
    if count_users(&db).await.unwrap_or(0) == 0 {
        return Redirect::to("/setup");
    }
    Redirect::to("/dashboard")
}

#[derive(Deserialize)]
pub struct LoginQuery {
    pub error: Option<String>,
}

/// GET /login — public.
pub async fn login_page(Query(q): Query<LoginQuery>) -> Html<String> {
    Html(ui::login_page(q.error.as_deref()))
}

/// GET /setup — public while the instance has no users.
pub async fn setup_page(
    State(db): State<DbPool>,
    Query(q): Query<LoginQuery>,
) -> impl IntoResponse {
    if count_users(&db).await.unwrap_or(1) > 0 {
        return Redirect::to("/login").into_response();
    }
    Html(ui::setup_page(q.error.as_deref())).into_response()
}

/// GET /dashboard
/// An admin who has not finished the first-run wizard is sent to it first.
pub async fn dashboard_page(
    State(db): State<DbPool>,
    Extension(user): Extension<AuthUser>,
) -> axum::response::Response {
    if user.is_admin() && !settings::wizard_done(&db).await {
        return Redirect::to("/welcome").into_response();
    }
    let nav = registry::nav_entries();
    let blueprints: Vec<(&str, &str)> = registry::blueprints().map(|b| (b.id, b.name)).collect();
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Html(ui::page(
        "Dashboard",
        &nav,
        "/dashboard",
        &view,
        &ui::dashboard_content(&blueprints),
    ))
    .into_response()
}

/// GET /welcome: the first-run wizard (admins only).
pub async fn welcome_page(
    State(db): State<DbPool>,
    Extension(user): Extension<AuthUser>,
) -> Result<Html<String>, Redirect> {
    if !user.is_admin() {
        return Err(Redirect::to("/dashboard"));
    }
    let name = settings::server_name(&db).await;
    let users: Vec<ui::UserRow> = list_users(&db)
        .await
        .unwrap_or_default()
        .iter()
        .map(|u| ui::UserRow {
            id: u.id,
            username: u.username.clone(),
            role: u.role().as_str().to_string(),
            disabled: u.disabled,
        })
        .collect();
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Ok(Html(ui::page(
        "Welcome",
        &registry::nav_entries(),
        "/welcome",
        &view,
        &ui::welcome_content(&name, &users),
    )))
}

/// GET /settings
pub async fn settings_page(Extension(user): Extension<AuthUser>) -> Html<String> {
    let nav = registry::nav_entries();
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Html(ui::page(
        "Settings",
        &nav,
        "/settings",
        &view,
        &ui::settings_content(),
    ))
}

/// GET /admin/users — admin only (manual check; pages do not go through the
/// macro role layer).
pub async fn users_page(
    State(db): State<DbPool>,
    Extension(user): Extension<AuthUser>,
) -> Result<Html<String>, Redirect> {
    if !user.is_admin() {
        return Err(Redirect::to("/dashboard"));
    }
    let nav = registry::nav_entries();
    let users = list_users(&db).await.unwrap_or_default();
    let rows: Vec<ui::UserRow> = users
        .iter()
        .map(|u| ui::UserRow {
            id: u.id,
            username: u.username.clone(),
            role: u.role().as_str().to_string(),
            disabled: u.disabled,
        })
        .collect();
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Ok(Html(ui::page(
        "Admin · Users",
        &nav,
        "/admin/users",
        &view,
        &ui::users_admin_content(&rows),
    )))
}

/// GET /admin/jobs: scheduled jobs and their recent runs (admins only).
pub async fn jobs_page(Extension(user): Extension<AuthUser>) -> Result<Html<String>, Redirect> {
    if !user.is_admin() {
        return Err(Redirect::to("/dashboard"));
    }
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Ok(Html(ui::page(
        "Admin · Jobs",
        &registry::nav_entries(),
        "/admin/jobs",
        &view,
        JOBS_BODY,
    )))
}

const JOBS_BODY: &str = r##"
<div class="card">
  <div class="spread">
    <span class="muted">Scheduled background jobs. Refreshes every 10 seconds; history resets on restart.</span>
    <button class="secondary" data-click="load">Refresh</button>
  </div>
  <div id="msg"></div>
  <div id="jobs"><p class="muted">Loading…</p></div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function when(unix) { return unix == null ? '–' : new Date(Number(unix) * 1000).toLocaleString(); }
function outcome(o) {
  if (o == null) return '<span class="muted">not run yet</span>';
  const cls = o === 'ok' ? 'badge' : (o === 'failed' ? 'badge warn' : 'badge danger');
  return '<span class="' + cls + '">' + esc(o) + '</span>';
}
async function load() {
  try {
    const r = await fetch('/api/admin/jobs');
    const jobs = await r.json();
    if (!r.ok) throw new Error(jobs.error || r.statusText);
    document.getElementById('msg').innerHTML = '';
    document.getElementById('jobs').innerHTML = jobs.length === 0
      ? '<p class="muted">No scheduled jobs are registered.</p>'
      : '<div style="overflow-x:auto"><table><thead><tr><th>Blueprint</th><th>Job</th><th>Every</th><th>Last started</th><th>Last finished</th><th>Last result</th><th>Next run</th><th>Runs</th><th>Failures</th></tr></thead><tbody>' +
        jobs.map(j =>
          '<tr><td>' + esc(j.blueprint) + '</td><td>' + esc(j.name) + '</td><td>' + Number(j.every_secs) + ' s</td>' +
          '<td>' + esc(when(j.last_started_at)) + '</td><td>' + esc(when(j.last_finished_at)) + '</td>' +
          '<td>' + outcome(j.last_outcome) + (j.last_error ? '<div class="muted">' + esc(j.last_error) + '</div>' : '') + '</td>' +
          '<td>' + esc(when(j.next_run_at)) + '</td><td>' + Number(j.runs) + '</td><td>' + Number(j.failures) + '</td></tr>'
        ).join('') + '</tbody></table></div>';
  } catch (e) {
    document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>';
  }
}
load();
setInterval(load, 10000);
</script>"##;

/// GET /admin/status: health, backups and restore help (admins only).
pub async fn status_page(Extension(user): Extension<AuthUser>) -> Result<Html<String>, Redirect> {
    if !user.is_admin() {
        return Err(Redirect::to("/dashboard"));
    }
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Ok(Html(ui::page(
        "Admin · Status",
        &registry::nav_entries(),
        "/admin/status",
        &view,
        STATUS_BODY,
    )))
}

const STATUS_BODY: &str = r##"
<div class="card">
  <div class="spread">
    <span class="muted">Server health and backups. Refreshes every 30 seconds.</span>
    <button class="secondary" data-click="load">Refresh</button>
  </div>
  <div id="msg"></div>
  <div id="health"><p class="muted">Loading…</p></div>
</div>
<div class="card">
  <div class="spread">
    <h3 style="margin:0">Backups</h3>
    <button id="make" type="button">Back up now</button>
  </div>
  <p class="muted" style="margin-top:8px">A backup is a copy of the database and the uploads. Downloads contain the database only. Restoring needs the server stopped: run <code>tpt-home restore &lt;path&gt;</code> with the path shown below.</p>
  <div id="backups"><p class="muted">Loading…</p></div>
  <form id="upform" style="margin-top:14px">
    <label for="upfile">Upload a database file as a new backup</label>
    <div class="spread" style="margin-top:6px">
      <input id="upfile" type="file" accept=".db,application/x-sqlite3,application/octet-stream" required>
      <button type="submit" class="secondary">Upload</button>
    </div>
  </form>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function when(unix) { return unix == null ? 'never' : new Date(Number(unix) * 1000).toLocaleString(); }
function size(b) { return b < 1048576 ? (b / 1024).toFixed(1) + ' KiB' : (b / 1048576).toFixed(1) + ' MiB'; }
function show(text, ok) { toast(text, ok); }
async function api(path, opts) {
  const r = await fetch(path, opts || {});
  const body = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(body.error || r.statusText);
  return body;
}
async function loadHealth() {
  try {
    const h = await api('/api/admin/health');
    const cell = (label, value) => '<div><div class="muted">' + esc(label) + '</div><div>' + value + '</div></div>';
    const failing = h.failing_jobs > 0
      ? '<span class="badge danger">' + h.failing_jobs + ' failing</span>'
      : '<span class="badge">' + h.jobs + ' ok</span>';
    document.getElementById('health').innerHTML = '<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:14px">' +
      cell('Version', esc(h.version)) +
      cell('Uptime', Math.floor(h.uptime_secs / 3600) + ' h ' + Math.floor(h.uptime_secs % 3600 / 60) + ' min') +
      cell('Blueprints', h.blueprints) +
      cell('Database', size(h.db_bytes)) +
      cell('Jobs', failing) +
      cell('Last backup', esc(when(h.last_backup_at))) +
      cell('Scheduled backups', h.backup_every_hours > 0 ? 'every ' + h.backup_every_hours + ' h, keep ' + h.backup_keep : 'off') +
      '</div>';
  } catch (e) { show(e.message, false); }
}
async function loadBackups() {
  try {
    const list = await api('/api/admin/backups');
    document.getElementById('backups').innerHTML = list.length === 0
      ? '<p class="muted">No backups yet.</p>'
      : '<div style="overflow-x:auto"><table><thead><tr><th>Taken</th><th>Database</th><th>Uploads</th><th>Restore path</th><th></th></tr></thead><tbody>' +
        list.map(b => '<tr><td>' + esc(when(b.created_at)) + '</td><td>' + size(b.db_bytes) + '</td><td>' + (b.has_uploads ? 'yes' : 'no') +
          '</td><td><code>' + esc(b.path) + '</code></td><td><a href="/api/admin/backups/' + encodeURIComponent(b.name) + '/download">Download database</a></td></tr>').join('') +
        '</tbody></table></div>';
  } catch (e) { show(e.message, false); }
}
function load() { loadHealth(); loadBackups(); }
document.getElementById('make').addEventListener('click', async () => {
  try {
    const b = await api('/api/admin/backups', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
    show('Backup written: ' + b.path, true);
    loadBackups(); loadHealth();
  } catch (e) { show(e.message, false); }
});
document.getElementById('upform').addEventListener('submit', async (ev) => {
  ev.preventDefault();
  const file = document.getElementById('upfile').files[0];
  if (!file) return false;
  const form = new FormData();
  form.append('file', file, file.name);
  try {
    const b = await api('/api/admin/backups/upload', { method: 'POST', headers: { 'X-TPT-Request': '1' }, body: form });
    show('Uploaded as a new backup. Restore it with: tpt-home restore ' + b.path, true);
    document.getElementById('upfile').value = '';
    loadBackups(); loadHealth();
  } catch (e) { show(e.message, false); }
  return false;
});
load();
setInterval(load, 30000);
</script>"##;
