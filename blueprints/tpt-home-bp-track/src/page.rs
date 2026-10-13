// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The Track UI page: define tables, browse rows, add/edit/delete.

use axum::response::Html;

use tpt_home_core::auth::user::AuthUser;
use tpt_home_core::registry;
use tpt_home_ui as ui;

/// GET /track
pub fn track_page(user: &AuthUser) -> Html<String> {
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Html(ui::page(
        "Track",
        &registry::nav_entries(),
        "/track",
        &view,
        &content_fragment(),
    ))
}

fn content_fragment() -> String {
    format!(
        "<div class=\"card\">\n\
         <div class=\"spread\">\n<h2 style=\"margin:0\">Tables</h2>\n\
         <button data-click=\"showNewTable\">New table</button>\n</div>\n\
         <div id=\"newtable\" style=\"display:none\">\n\
         <label for=\"tname\">Table name</label><input id=\"tname\" style=\"max-width:280px\">\n\
         <div id=\"fields\"></div>\n\
         <div class=\"row\" style=\"margin-top:8px\">\n\
         <button class=\"secondary\" data-click=\"addFieldRow\">+ field</button>\n\
         <button data-click=\"createTable\">Create</button>\n</div>\n</div>\n\
         <div id=\"msg\"></div>\n</div>\n\
         <div id=\"tables\"></div>\n\
         <div id=\"rowsview\"></div>\n\
         <script>\n{}\n</script>",
        r#"function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
function fail(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
let TABLES = [];
let fieldRowCount = 0;
function showNewTable() {
  const el = document.getElementById('newtable');
  el.style.display = el.style.display === 'none' ? 'block' : 'none';
  if (el.style.display === 'block' && fieldRowCount === 0) { addFieldRow(); addFieldRow(); }
}
function addFieldRow() {
  const div = document.createElement('div');
  div.className = 'row'; div.style.marginTop = '6px';
  div.innerHTML = '<input class="fname" placeholder="field name" style="max-width:200px">' +
    '<select class="fkind" style="max-width:140px">' +
    '<option>text</option><option>integer</option><option>real</option><option>bool</option></select>';
  document.getElementById('fields').appendChild(div);
  fieldRowCount++;
}
async function createTable() {
  const name = document.getElementById('tname').value.trim();
  const fields = [...document.querySelectorAll('#fields .row')].map(r => ({
    name: r.querySelector('.fname').value.trim(),
    kind: r.querySelector('.fkind').value,
  })).filter(f => f.name);
  try {
    await api('/api/track/tables', 'POST', { name, fields });
    document.getElementById('newtable').style.display = 'none';
    document.getElementById('fields').innerHTML = ''; fieldRowCount = 0;
    loadTables();
  } catch (e) { fail(e); }
}
async function loadTables() {
  try {
    TABLES = await api('/api/track/tables');
    document.getElementById('tables').innerHTML = TABLES.map(t => `
      <div class="card">
        <div class="spread">
          <strong>${esc(t.name)}</strong>
          <span class="row">
            <button class="secondary" data-click="openRows:${t.id}">Open</button>
            <button class="danger" data-click="delTable:${t.id}">Delete</button>
          </span>
        </div>
        <div class="muted">${t.fields.map(f => esc(f.name) + ' (' + f.kind + ')').join(' · ')}</div>
      </div>`).join('') || '<div class="card muted">No tables yet — create one above.</div>';
  } catch (e) { fail(e); }
}
async function delTable(id) {
  if (!confirm('Delete this table and all its rows?')) return;
  try { await api('/api/track/tables/' + id, 'DELETE'); loadTables(); document.getElementById('rowsview').innerHTML = ''; } catch (e) { fail(e); }
}
function fieldByCol(t, col) { return t.fields.find(f => 'f_' + f.id === col); }
async function openRows(id) {
  const t = TABLES.find(x => x.id === id);
  try {
    const rows = await api('/api/track/tables/' + id + '/rows');
    const headers = '<tr><th>#</th>' + t.fields.map(f => '<th>' + esc(f.name) + '</th>').join('') + '<th></th></tr>';
    const body = rows.map(r => '<tr><td>' + r.id + '</td>' + t.fields.map(f => {
      const v = r['f_' + f.id];
      return '<td>' + esc(v === null || v === undefined ? '' : String(v)) + '</td>';
    }).join('') +
      '<td><button class="danger" data-click="delRow:' + id + ':' + r.id + '">×</button></td></tr>').join('');
    const addForm = '<div class="row" style="margin-top:10px">' +
      t.fields.map(f => `<input id="add_${f.id}" placeholder="${esc(f.name)} (${f.kind})" style="max-width:170px">`).join('') +
      '<button data-click="addRow:' + id + '">Add row</button></div>';
    document.getElementById('rowsview').innerHTML =
      '<div class="card"><h2 style="margin-top:0">' + esc(t.name) + '</h2>' +
      '<div style="overflow-x:auto"><table><thead>' + headers + '</thead><tbody>' + body + '</tbody></table></div>' + addForm + '</div>';
  } catch (e) { fail(e); }
}
function coerce(kind, raw) {
  if (!raw) return null;
  if (kind === 'integer') { const n = parseInt(raw, 10); if (isNaN(n)) throw new Error('not an integer: ' + raw); return n; }
  if (kind === 'real') { const x = parseFloat(raw); if (isNaN(x)) throw new Error('not a number: ' + raw); return x; }
  if (kind === 'bool') { return raw === 'true' || raw === '1'; }
  return raw;
}
async function addRow(id) {
  const t = TABLES.find(x => x.id === id);
  const values = {};
  try {
    for (const f of t.fields) {
      const raw = document.getElementById('add_' + f.id).value;
      if (raw !== '') values['f_' + f.id] = coerce(f.kind, raw);
    }
    await api('/api/track/tables/' + id + '/rows', 'POST', values);
    openRows(id);
  } catch (e) { fail(e); }
}
async function delRow(id, row) {
  try { await api('/api/track/tables/' + id + '/rows/' + row, 'DELETE'); openRows(id); } catch (e) { fail(e); }
}
loadTables();"#,
    )
}
