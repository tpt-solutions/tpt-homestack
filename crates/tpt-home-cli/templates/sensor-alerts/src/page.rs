// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The sensor-alerts page. Admins see the setup card (webhook and rules);
//! everyone sees the sensors, the alerts, and how to send events.

use axum::response::Html;
use tpt_home_core::prelude::*;

pub fn render(user: &AuthUser) -> Html<String> {
    let view = tpt_home_ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    let body = if user.is_admin() {
        format!("{SETUP}{BODY}")
    } else {
        BODY.to_string()
    };
    Html(tpt_home_ui::page(
        "<TITLE>",
        &registry::nav_entries(),
        "/<NAME>",
        &view,
        &body,
    ))
}

/// Admin-only card: create the webhook, then choose it as the feed and set the rules.
const SETUP: &str = r##"
<div class="card">
  <h2 style="margin-top:0">Setup</h2>
  <p class="muted">1. Create the webhook that sensors will post to. The secret is shown once; store it in the sender.</p>
  <div class="row" style="gap:8px;flex-wrap:wrap">
    <input id="hook-name" maxlength="50" placeholder="webhook name, e.g. house-sensors" style="max-width:280px">
    <button data-click="createHook">Create webhook</button>
  </div>
  <div id="hook-result"></div>
  <p class="muted" style="margin-top:12px">2. Choose the webhook as the feed and set the rules. Quiet hours are in the server's time zone; the server's clock is <span id="server-clock">…</span>.</p>
  <div class="row" style="gap:8px;flex-wrap:wrap">
    <label>Feed <input id="feed" maxlength="50" style="max-width:220px"></label>
    <label>Quiet from (hour) <input id="qfrom" type="number" min="0" max="23" style="max-width:90px"></label>
    <label>to (hour) <input id="qto" type="number" min="0" max="23" style="max-width:90px"></label>
    <label>Silent after (hours) <input id="silent" type="number" min="1" max="720" style="max-width:110px"></label>
    <button data-click="saveSettings">Save</button>
  </div>
  <div id="settings-msg"></div>
</div>
"##;

const BODY: &str = r##"
<div id="msg"></div>
<div class="grid">
  <div class="card">
    <h2 style="margin-top:0">Sensors</h2>
    <div id="sensors"><p class="muted">Loading…</p></div>
  </div>
  <div class="card">
    <h2 style="margin-top:0">Sending events</h2>
    <p class="muted">POST JSON to <code>/api/hooks/&lt;webhook name&gt;</code> with headers <code>X-Tpt-Timestamp</code> (Unix seconds) and <code>X-Tpt-Signature</code> (hex HMAC-SHA256 of <code>&lt;timestamp&gt;.&lt;body&gt;</code>, keyed with the secret).</p>
<pre>import hmac, hashlib, json, time, urllib.request
body = json.dumps({"sensor": "front-door", "state": "open"}).encode()
ts = str(int(time.time()))
sig = hmac.new(SECRET.encode(), ts.encode() + b"." + body, hashlib.sha256).hexdigest()
req = urllib.request.Request(URL, data=body, headers={
    "Content-Type": "application/json",
    "X-Tpt-Timestamp": ts, "X-Tpt-Signature": sig})
urllib.request.urlopen(req)</pre>
  </div>
</div>
<div class="card" style="margin-top:16px">
  <h2 style="margin-top:0">Alerts</h2>
  <div id="alerts"><p class="muted">Loading…</p></div>
</div>
<script>
function esc(s) { const d = document.createElement('div'); d.textContent = s ?? ''; return d.innerHTML; }
function showError(e) { document.getElementById('msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
function when(unix) { return new Date(Number(unix) * 1000).toLocaleString(); }
let settingsShown = false; // fill the form once, so refreshes do not overwrite edits
async function api(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
async function load() {
  try {
    const [status, alerts] = await Promise.all([api('/api/<NAME>/status', 'GET'), api('/api/<NAME>/alerts', 'GET')]);
    const s = status.settings;
    const clockEl = document.getElementById('server-clock');
    if (clockEl) {
      clockEl.textContent = status.server_clock.local_time + ' (UTC' + status.server_clock.utc_offset + ')';
    }
    const feed = document.getElementById('feed');
    if (feed && !settingsShown) {
      settingsShown = true;
      feed.value = s.webhook_name;
      document.getElementById('qfrom').value = s.quiet_from;
      document.getElementById('qto').value = s.quiet_to;
      document.getElementById('silent').value = s.silent_after_hours;
    }
    document.getElementById('sensors').innerHTML = status.sensors.length === 0
      ? '<p class="muted">No sensor has reported yet.</p>'
      : '<table><thead><tr><th>Sensor</th><th>State</th><th>Last seen</th></tr></thead><tbody>' +
        status.sensors.map(x => '<tr><td>' + esc(x.sensor) + '</td><td>' + esc(x.state) + '</td><td>' + esc(when(x.seen_at)) + '</td></tr>').join('') +
        '</tbody></table>';
    document.getElementById('alerts').innerHTML = alerts.length === 0
      ? '<p class="muted">No alerts.</p>'
      : '<table><thead><tr><th>When</th><th>Kind</th><th>Message</th></tr></thead><tbody>' +
        alerts.map(a => '<tr><td>' + esc(when(a.created_at)) + '</td><td>' + esc(a.kind) + '</td><td>' + esc(a.message) + '</td></tr>').join('') +
        '</tbody></table>';
  } catch (e) { showError(e); }
}
async function createHook() {
  const name = document.getElementById('hook-name').value.trim();
  try {
    const j = await api('/api/webhooks', 'POST', {name});
    document.getElementById('hook-result').innerHTML =
      '<div class="msg">Created. Secret (shown once): <code>' + esc(j.secret) + '</code><br>Send events to <code>' + esc(j.url) + '</code></div>';
    document.getElementById('feed').value = j.name;
  } catch (e) { showError(e); }
}
async function saveSettings() {
  const body = {
    webhook_name: document.getElementById('feed').value.trim(),
    quiet_from: parseInt(document.getElementById('qfrom').value, 10),
    quiet_to: parseInt(document.getElementById('qto').value, 10),
    silent_after_hours: parseInt(document.getElementById('silent').value, 10),
  };
  try {
    await api('/api/<NAME>/settings', 'POST', body);
    document.getElementById('settings-msg').innerHTML = '<div class="msg">Saved.</div>';
    await load();
  } catch (e) { document.getElementById('settings-msg').innerHTML = '<div class="msg error">' + esc(e.message) + '</div>'; }
}
load();
setInterval(load, 15000);
</script>"##;
