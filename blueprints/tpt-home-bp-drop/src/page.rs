// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The Drop UI page: upload form (multipart via fetch), file list, share
//! link creation and deletion.

use axum::response::Html;

use tpt_home_core::auth::user::AuthUser;
use tpt_home_core::registry;
use tpt_home_ui as ui;

/// GET /drop
pub fn drop_page(user: &AuthUser) -> Html<String> {
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Html(ui::page(
        "Drop",
        &registry::nav_entries(),
        "/drop",
        &view,
        &content_fragment(),
    ))
}

fn content_fragment() -> String {
    format!(
        "<div class=\"card\">\n\
         <h2 style=\"margin-top:0\">Upload</h2>\n\
         <div class=\"row\">\n\
         <input type=\"file\" id=\"file\" style=\"max-width:340px\">\n\
         <button data-click=\"doUpload\">Upload</button>\n\
         <span class=\"muted\" id=\"upstatus\"></span>\n</div>\n\
         <div id=\"msg\"></div>\n</div>\n\
         <div id=\"list\"></div>\n\
         <script>\n{}\n</script>",
        r#"function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function fmtSize(n) {
  if (n > 1e9) return (n/1e9).toFixed(2) + ' GB';
  if (n > 1e6) return (n/1e6).toFixed(2) + ' MB';
  if (n > 1e3) return (n/1e3).toFixed(1) + ' KB';
  return n + ' B';
}
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
async function doUpload() {
  const input = document.getElementById('file');
  if (!input.files.length) { alert('pick a file first'); return; }
  const fd = new FormData(); fd.append('file', input.files[0]);
  document.getElementById('upstatus').textContent = 'uploading…';
  try {
    const r = await fetch('/api/drop/upload', { method: 'POST', body: fd, headers: { 'X-TPT-Request': '1' } });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    document.getElementById('upstatus').textContent = 'done';
    loadList();
  } catch (e) {
    document.getElementById('upstatus').textContent = '';
    document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>';
  }
}
async function makeShare(id) {
  const hours = prompt('Link expires in how many hours? (empty = never)', '24');
  const password = prompt('Protect with password? (empty = none)', '') || null;
  try {
    const j = await api('/api/drop/' + id + '/share', 'POST', {
      hours: hours ? parseInt(hours, 10) : null, password });
    prompt('Share URL (Ctrl+C):', location.origin + j.url);
    loadList();
  } catch (e) { alert(e.message); }
}
async function delFile(id) {
  if (!confirm('Delete this file?')) return;
  try { await api('/api/drop/' + id, 'DELETE'); loadList(); } catch (e) { alert(e.message); }
}
async function loadList() {
  try {
    const files = await api('/api/drop');
    document.getElementById('list').innerHTML = files.map(f => `
      <div class="card">
        <div class="spread">
          <strong>${esc(f.filename)}</strong>
          <span class="row">
            <a class="btn" href="/api/drop/${f.id}/download">Download</a>
            <button class="secondary" data-click="makeShare:${f.id}">Share</button>
            <button class="danger" data-click="delFile:${f.id}">Delete</button>
          </span>
        </div>
        <div class="muted">${fmtSize(f.size)} · ${new Date(f.created_at * 1000).toLocaleString()}
          ${f.expires_at ? ' · expires ' + new Date(f.expires_at * 1000).toLocaleString() : ''}
          ${f.share_token ? ' · <a href="/api/drop/shared/' + f.share_token + '">shared link</a>' : ''}</div>
      </div>`).join('') || '<div class="card muted">No files yet.</div>';
  } catch (e) {
    document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>';
  }
}
loadList();"#,
    )
}
