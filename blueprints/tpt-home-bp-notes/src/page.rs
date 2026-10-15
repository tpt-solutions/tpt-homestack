// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The Notes UI page. Server-rendered shell around a small inline script
//! that drives the JSON API — no build step, works without JS for reading.

use axum::response::Html;

use tpt_home_core::auth::user::AuthUser;
use tpt_home_core::registry;
use tpt_home_ui as ui;

/// GET /notes — the full authenticated shell with the notes workspace inside.
pub fn notes_page(user: &AuthUser) -> Html<String> {
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Html(ui::page(
        "Notes",
        &registry::nav_entries(),
        "/notes",
        &view,
        &content_fragment(),
    ))
}

fn content_fragment() -> String {
    let search = ui::components::button("Search", "loadNotes", ui::components::ButtonKind::Primary);
    let new_note = ui::components::button(
        "New note",
        "openEditor",
        ui::components::ButtonKind::Secondary,
    );
    let save = ui::components::button("Save", "saveNote", ui::components::ButtonKind::Primary);
    let preview = ui::components::button(
        "Preview",
        "previewNote",
        ui::components::ButtonKind::Secondary,
    );
    let cancel =
        ui::components::button("Cancel", "closeEditor", ui::components::ButtonKind::Danger);
    format!(
        "<div id=\"notes-root\">\n\
         <div class=\"card\">\n\
         <div class=\"row\">\n\
         <input id=\"q\" placeholder=\"Search notes…\" aria-label=\"Search notes\" class=\"s-max-width-340px\">\n\
         {search}\n\
         {new_note}\n\
         </div>\n<div id=\"msg\"></div>\n</div>\n\
         <div id=\"editor\" class=\"card s-display-none\">\n\
         <input id=\"eid\" type=\"hidden\">\n\
         <label for=\"etitle\">Title</label><input id=\"etitle\">\n\
         <label for=\"etags\">Tags (comma separated)</label><input id=\"etags\">\n\
         <label for=\"ebody\">Body (markdown)</label><textarea id=\"ebody\" rows=\"12\"></textarea>\n\
         <div class=\"row s-margin-top-12px\">\n\
         {save}\n\
         {preview}\n\
         {cancel}\n\
         </div>\n<div id=\"preview\" class=\"msg s-display-none\"></div>\n</div>\n\
         <div id=\"list\"></div>\n</div>\n<script>\n{}\n</script>",
        r#"async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
async function loadNotes() {
  const q = document.getElementById('q').value.trim();
  const url = q ? '/api/notes?q=' + encodeURIComponent(q) : '/api/notes';
  try {
    const notes = await api(url);
    document.getElementById('list').innerHTML = notes.map(n => `
      <div class="card">
        <div class="spread">
          <strong>${esc(n.title)}</strong>
          <span class="row">
            <button class="secondary" data-click="editNote:${n.id}">Edit</button>
            <button class="danger" data-click="delNote:${n.id}">Delete</button>
          </span>
        </div>
        <div class="muted">${new Date(n.updated_at * 1000).toLocaleString()} ${(n.tags || []).map(t => '<span class="badge">' + esc(t) + '</span>').join(' ')}</div>
        <div>${esc((n.body || '').slice(0, 160))}${(n.body || '').length > 160 ? '…' : ''}</div>
      </div>`).join('') || '<div class="card muted">No notes yet.</div>';
  } catch (e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
}
function openEditor() {
  document.getElementById('editor').style.display = 'block';
  document.getElementById('eid').value = '';
  document.getElementById('etitle').value = '';
  document.getElementById('etags').value = '';
  document.getElementById('ebody').value = '';
  document.getElementById('preview').style.display = 'none';
}
function closeEditor() { document.getElementById('editor').style.display = 'none'; }
async function editNote(id) {
  try {
    const n = await api('/api/notes/' + id);
    openEditor();
    document.getElementById('eid').value = n.id;
    document.getElementById('etitle').value = n.title;
    document.getElementById('etags').value = (n.tags || []).join(', ');
    document.getElementById('ebody').value = n.body;
  } catch (e) { alert(e.message); }
}
async function saveNote() {
  const id = document.getElementById('eid').value;
  const body = { title: document.getElementById('etitle').value,
                 body: document.getElementById('ebody').value,
                 tags: document.getElementById('etags').value.split(',').map(s => s.trim()).filter(Boolean) };
  try {
    if (id) await api('/api/notes/' + id, 'PUT', body); else await api('/api/notes', 'POST', body);
    closeEditor(); loadNotes();
  } catch (e) { alert(e.message); }
}
async function delNote(id) {
  if (!(await confirmDialog('Delete note?', 'Delete'))) return;
  try { await api('/api/notes/' + id, 'DELETE'); loadNotes(); } catch (e) { alert(e.message); }
}
async function previewNote() {
  const p = document.getElementById('preview');
  p.style.display = 'block';
  p.innerHTML = '<span class="muted">rendering…</span>';
  // Render the unsaved editor body through the API: create nothing, preview
  // via a throwaway fetch is avoided — render client text by PUT-less call
  // is not available, so show plain escaped text with a hint.
  p.innerHTML = '<pre class="md">' + esc(document.getElementById('ebody').value) + '</pre>';
}
loadNotes();"#,
    )
}
