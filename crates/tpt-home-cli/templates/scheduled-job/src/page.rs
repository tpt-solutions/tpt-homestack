// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The scheduled-job page: record events, and see the job's snapshots.

use axum::response::Html;
use tpt_home_core::prelude::*;

pub fn render(user: &AuthUser) -> Html<String> {
    let view = tpt_home_ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Html(tpt_home_ui::page(
        "<TITLE>",
        &registry::nav_entries(),
        "/<NAME>",
        &view,
        BODY,
    ))
}

const BODY: &str = r##"
<div class="card">
  <h2 style="margin-top:0">Record an event</h2>
  <input id="note" maxlength="500" placeholder="What happened?">
  <div class="row" style="margin-top:8px"><button data-click="addEvent">Record</button></div>
  <div id="msg"></div>
</div>
<div class="grid">
  <div class="card">
    <h2 style="margin-top:0">Snapshots</h2>
    <p class="muted">The job runs hourly and stores the number of events from the last 7 days, one row per day.</p>
    <div id="snapshots"><p class="muted">Loading…</p></div>
  </div>
  <div class="card">
    <h2 style="margin-top:0">Recent events</h2>
    <div id="events"><p class="muted">Loading…</p></div>
  </div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function showError(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
function timeString(unix) { return new Date(Number(unix) * 1000).toLocaleString(); }
async function load() {
  try {
    const [snaps, events] = await Promise.all([api('/api/<NAME>/snapshots', 'GET'), api('/api/<NAME>/events', 'GET')]);
    document.getElementById('snapshots').innerHTML = snaps.length === 0
      ? '<p class="muted">No snapshot yet. The first one is taken an hour after the server starts.</p>'
      : '<table><thead><tr><th>Taken</th><th>Events, last 7 days</th></tr></thead><tbody>' +
        snaps.map(s => '<tr><td>' + esc(timeString(s.taken_at)) + '</td><td>' + Number(s.events_7d) + '</td></tr>').join('') +
        '</tbody></table>';
    document.getElementById('events').innerHTML = events.length === 0
      ? '<p class="muted">No events yet.</p>'
      : '<ul>' + events.map(e => '<li>' + esc(e.note) + ' <span class="muted">' + esc(timeString(e.created_at)) + '</span></li>').join('') + '</ul>';
  } catch (e) { showError(e); }
}
async function addEvent() {
  const note = document.getElementById('note').value;
  try {
    await api('/api/<NAME>/events', 'POST', {note});
    document.getElementById('note').value = '';
    document.getElementById('msg').innerHTML = '';
    await load();
  } catch (e) { showError(e); }
}
load();
</script>"##;
