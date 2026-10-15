// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The logbook page: summary, a form to add an entry, and the entry list.

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
  <div class="row s-gap-24px">
    <div><div class="muted">Entries</div><strong id="s-count">–</strong></div>
    <div><div class="muted">Total</div><strong id="s-total">–</strong></div>
    <div><div class="muted">Average</div><strong id="s-average">–</strong></div>
    <div><div class="muted">Latest</div><strong id="s-latest">–</strong></div>
  </div>
</div>
<div class="card">
  <h2 class="mt0">Add an entry</h2>
  <label for="day">Date</label>
  <input id="day" type="date" class="s-max-width-220px">
  <label for="label">Label</label>
  <input id="label" maxlength="100" placeholder="e.g. Electricity meter">
  <label for="value">Value</label>
  <input id="value" type="number" step="any" class="s-max-width-220px">
  <label for="notes">Notes</label>
  <textarea id="notes" rows="2" maxlength="2000" placeholder="Optional"></textarea>
  <div class="row mt8"><button data-click="addEntry">Add</button></div>
  <div id="msg"></div>
</div>
<div class="card">
  <h2 class="mt0">Entries</h2>
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
function showError(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
function today() {
  const d = new Date();
  const pad = n => String(n).padStart(2, '0');
  return d.getFullYear() + '-' + pad(d.getMonth() + 1) + '-' + pad(d.getDate());
}
async function loadSummary() {
  const s = await api('/api/<NAME>/summary', 'GET');
  document.getElementById('s-count').textContent = s.count;
  document.getElementById('s-total').textContent = s.total.toFixed(2);
  document.getElementById('s-average').textContent = s.average == null ? '–' : s.average.toFixed(2);
  document.getElementById('s-latest').textContent = s.latest_on ? s.latest_on + ' (' + s.latest_value + ')' : '–';
}
async function loadEntries() {
  const rows = await api('/api/<NAME>/entries', 'GET');
  const list = document.getElementById('list');
  if (rows.length === 0) { list.innerHTML = '<p class="muted">No entries yet.</p>'; return; }
  list.innerHTML = '<table><thead><tr><th>Date</th><th>Label</th><th>Value</th><th>Notes</th><th></th></tr></thead><tbody>' +
    rows.map(r => '<tr><td>' + esc(r.logged_on) + '</td><td>' + esc(r.label) + '</td><td>' + Number(r.value) +
      '</td><td>' + esc(r.notes) + '</td><td><button class="secondary" data-click="removeEntry:' + Number(r.id) + '">Delete</button></td></tr>'
    ).join('') + '</tbody></table>';
}
async function refresh() {
  try { await Promise.all([loadSummary(), loadEntries()]); } catch (e) { showError(e); }
}
async function addEntry() {
  const body = {
    logged_on: document.getElementById('day').value || today(),
    label: document.getElementById('label').value,
    value: parseFloat(document.getElementById('value').value),
    notes: document.getElementById('notes').value,
  };
  try {
    await api('/api/<NAME>/entries', 'POST', body);
    document.getElementById('label').value = '';
    document.getElementById('value').value = '';
    document.getElementById('notes').value = '';
    document.getElementById('msg').innerHTML = '';
    await refresh();
  } catch (e) { showError(e); }
}
async function removeEntry(id) {
  if (!confirm('Delete this entry?')) return;
  try { await api('/api/<NAME>/entries/' + id, 'DELETE'); await refresh(); }
  catch (e) { showError(e); }
}
document.getElementById('day').value = today();
refresh();
</script>"##;
