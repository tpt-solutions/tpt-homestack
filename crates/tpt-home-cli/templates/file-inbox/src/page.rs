// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The file inbox page: an upload form and the list of stored files.

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
  <h2 style="margin-top:0">Upload a file</h2>
  <input id="file" type="file">
  <div class="row" style="margin-top:8px"><button onclick="upload()">Upload</button></div>
  <div id="msg"></div>
</div>
<div class="card">
  <h2 style="margin-top:0">Files</h2>
  <div id="list"><p class="muted">Loading…</p></div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function showError(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
function human(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KiB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MiB';
}
async function load() {
  try {
    const r = await fetch('/api/<NAME>/files');
    const rows = await r.json();
    const list = document.getElementById('list');
    if (rows.length === 0) { list.innerHTML = '<p class="muted">No files yet.</p>'; return; }
    list.innerHTML = '<table><thead><tr><th>Name</th><th>Size</th><th>Type</th><th></th></tr></thead><tbody>' +
      rows.map(f => '<tr><td>' + esc(f.original_name) + '</td><td>' + human(Number(f.size)) + '</td><td>' + esc(f.mime) +
        '</td><td><a href="/api/<NAME>/files/' + Number(f.id) + '">Download</a> ' +
        '<button class="secondary" onclick="removeFile(' + Number(f.id) + ')">Delete</button></td></tr>'
      ).join('') + '</tbody></table>';
  } catch (e) { showError(e); }
}
async function upload() {
  const input = document.getElementById('file');
  if (!input.files.length) { showError(new Error('choose a file first')); return; }
  const form = new FormData();
  form.append('file', input.files[0]);
  try {
    const r = await fetch('/api/<NAME>/files', {method: 'POST', body: form});
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    input.value = '';
    document.getElementById('msg').innerHTML = '';
    await load();
  } catch (e) { showError(e); }
}
async function removeFile(id) {
  if (!confirm('Delete this file?')) return;
  try {
    const r = await fetch('/api/<NAME>/files/' + id, {method: 'DELETE', headers: {'Content-Type': 'application/json'}});
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    await load();
  } catch (e) { showError(e); }
}
load();
</script>"##;
