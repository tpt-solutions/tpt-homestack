// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The dashboard page: one card per metric, plus a form to record readings.

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
  <h2 class="mt0">Record a reading</h2>
  <div class="row s-gap-8px-flex-wrap-wrap">
    <input id="metric" maxlength="50" placeholder="metric, e.g. power_kwh" class="s-max-width-240px">
    <input id="value" type="number" step="any" placeholder="value" class="s-max-width-160px">
    <button data-click="record">Record</button>
  </div>
  <div id="msg"></div>
</div>
<div id="cards" class="s-display-flex-flex-wrap-wrap-gap-16px">
  <p class="muted">Loading…</p>
</div>
<div class="card s-margin-top-16px">
  <h2 class="mt0">Latest readings</h2>
  <div id="recent"><p class="muted">Loading…</p></div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function showError(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
function fmt(n) { return n == null ? '–' : Number(n).toLocaleString(undefined, {maximumFractionDigits: 2}); }
function when(unix) { return unix == null ? '–' : new Date(Number(unix) * 1000).toLocaleString(); }
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
async function load() {
  try {
    const [cards, recent] = await Promise.all([api('/api/<NAME>/overview', 'GET'), api('/api/<NAME>/readings', 'GET')]);
    document.getElementById('cards').innerHTML = cards.length === 0
      ? '<p class="muted">No metrics yet. Record a reading to start a card.</p>'
      : cards.map(c =>
          '<div class="card s-flex-1-1-220px-max-width-300px-margin-0">' +
            '<div class="muted">' + esc(c.metric) + '</div>' +
            '<div class="s-font-size-2em-font-weight-600">' + fmt(c.latest) + '</div>' +
            '<div class="muted">7-day average ' + fmt(c.avg_7d) + '</div>' +
            '<div class="muted">' + Number(c.count) + ' readings · last ' + esc(when(c.latest_at)) + '</div>' +
          '</div>'
        ).join('');
    document.getElementById('recent').innerHTML = recent.length === 0
      ? '<p class="muted">Nothing recorded yet.</p>'
      : '<table><thead><tr><th>Metric</th><th>Value</th><th>When</th></tr></thead><tbody>' +
        recent.map(r => '<tr><td>' + esc(r.metric) + '</td><td>' + fmt(r.value) + '</td><td>' + esc(when(r.recorded_at)) + '</td></tr>').join('') +
        '</tbody></table>';
  } catch (e) { showError(e); }
}
async function record() {
  const metric = document.getElementById('metric').value.trim();
  const value = parseFloat(document.getElementById('value').value);
  try {
    await api('/api/<NAME>/readings', 'POST', {metric, value});
    document.getElementById('value').value = '';
    document.getElementById('msg').innerHTML = '';
    await load();
  } catch (e) { showError(e); }
}
load();
</script>"##;
