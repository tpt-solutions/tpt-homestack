// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The scheduled-fetch page: source URL (admins), manual fetch, stored responses.

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
  <h2 style="margin-top:0">Source</h2>
  <p class="muted">Only admins can change the source or fetch now. The job fetches it once an hour.</p>
  <div class="row" style="gap:8px;flex-wrap:wrap">
    <input id="url" type="url" placeholder="https://example.com/status.json" style="max-width:420px">
    <button data-click="saveSource">Save</button>
    <button class="secondary" data-click="fetchNow">Fetch now</button>
  </div>
  <div class="muted" id="current" style="margin-top:8px">Loading…</div>
  <div id="msg"></div>
</div>
<div class="card">
  <h2 style="margin-top:0">Responses</h2>
  <div id="list"><p class="muted">Loading…</p></div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function showError(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
function when(unix) { return new Date(Number(unix) * 1000).toLocaleString(); }
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
async function load() {
  try {
    const [settings, fetches] = await Promise.all([api('/api/<NAME>/settings', 'GET'), api('/api/<NAME>/fetches', 'GET')]);
    document.getElementById('current').textContent = settings.source_url
      ? 'Current source: ' + settings.source_url
      : 'No source set yet.';
    document.getElementById('list').innerHTML = fetches.length === 0
      ? '<p class="muted">Nothing fetched yet.</p>'
      : '<table><thead><tr><th>When</th><th>Status</th><th>Body</th></tr></thead><tbody>' +
        fetches.map(f => '<tr><td>' + esc(when(f.fetched_at)) + '</td><td>' +
          (f.status == null ? 'no response' : Number(f.status)) +
          (f.error ? '<div class="msg error">' + esc(f.error) + '</div>' : '') +
          '</td><td><pre style="white-space:pre-wrap;max-height:8em;overflow:auto">' + esc(f.body_preview) + '</pre></td></tr>'
        ).join('') + '</tbody></table>';
  } catch (e) { showError(e); }
}
async function saveSource() {
  try {
    await api('/api/<NAME>/settings', 'POST', {source_url: document.getElementById('url').value.trim()});
    document.getElementById('msg').innerHTML = '';
    await load();
  } catch (e) { showError(e); }
}
async function fetchNow() {
  try {
    await api('/api/<NAME>/fetch-now', 'POST', {});
    document.getElementById('msg').innerHTML = '';
    await load();
  } catch (e) { showError(e); }
}
load();
</script>"##;
