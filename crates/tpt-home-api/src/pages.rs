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
pub async fn dashboard_page(Extension(user): Extension<AuthUser>) -> Html<String> {
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
    <button class="secondary" onclick="load()">Refresh</button>
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
      : '<table><thead><tr><th>Blueprint</th><th>Job</th><th>Every</th><th>Last started</th><th>Last finished</th><th>Last result</th><th>Next run</th><th>Runs</th><th>Failures</th></tr></thead><tbody>' +
        jobs.map(j =>
          '<tr><td>' + esc(j.blueprint) + '</td><td>' + esc(j.name) + '</td><td>' + Number(j.every_secs) + ' s</td>' +
          '<td>' + esc(when(j.last_started_at)) + '</td><td>' + esc(when(j.last_finished_at)) + '</td>' +
          '<td>' + outcome(j.last_outcome) + (j.last_error ? '<div class="muted">' + esc(j.last_error) + '</div>' : '') + '</td>' +
          '<td>' + esc(when(j.next_run_at)) + '</td><td>' + Number(j.runs) + '</td><td>' + Number(j.failures) + '</td></tr>'
        ).join('') + '</tbody></table>';
  } catch (e) {
    document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>';
  }
}
load();
setInterval(load, 10000);
</script>"##;
