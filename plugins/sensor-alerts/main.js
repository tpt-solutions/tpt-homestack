// Sensor alerts: a door opened during quiet hours, and a sensor that has gone
// silent. Port of the sensor-alerts starter template.
//
// Signed events arrive at /hooks/feed and are recorded at once. The one-minute
// job at /check raises silence alerts, then sends each unsent alert to the
// "alerts" channel. An alert is marked sent only when the channel answers 2xx,
// so a failed send is retried on the next run.
//
// Quiet hours use the server's local time. This is an aid, not a certified
// alarm system.

const CHANNEL = "alerts";
const MAX_SENSOR = 64;
const MAX_PER_RUN = 20;
const DEFAULTS = { quiet_from: 23, quiet_to: 6, silent_after_hours: 24 };

function json(status, value) {
  return { status: status, content_type: "application/json", body: JSON.stringify(value) };
}

// The sandbox has no bound parameters, so text is quoted by hand.
function sqlText(text) {
  return "'" + String(text).replace(/'/g, "''") + "'";
}

function parseBody(text) {
  if (text === null || text === undefined) return null;
  try {
    return JSON.parse(text);
  } catch (e) {
    return null;
  }
}

function setting(key) {
  const raw = tpt.kv.get(key);
  const n = Number(raw);
  return raw === null || raw === "" || Number.isNaN(n) ? DEFAULTS[key] : n;
}

// True when `hour` is inside the quiet window. The window wraps past midnight
// when `from` is later than `to`; equal values turn it off.
function inQuiet(hour, from, to) {
  if (from === to) return false;
  return from < to ? hour >= from && hour < to : hour >= from || hour < to;
}

function addAlert(at, sensor, kind, message) {
  return tpt.db.execute(
    "INSERT INTO sensor_alerts_alerts (created_at, sensor, kind, message, notified) VALUES (" +
      Math.trunc(at) + ", " + sqlText(sensor) + ", " + sqlText(kind) + ", " + sqlText(message) + ", 0)"
  );
}

function onEvent(body) {
  const payload = parseBody(body);
  const sensor = payload && typeof payload.sensor === "string" ? payload.sensor.trim() : "";
  const state = payload && typeof payload.state === "string" ? payload.state.trim() : "";
  if (sensor === "" || state === "" || [...sensor].length > MAX_SENSOR || [...state].length > MAX_SENSOR) {
    // Accepted but not recorded: a bad event should not make the sender retry.
    return json(200, { ok: true, recorded: false });
  }
  const now = tpt.now();
  const saved = tpt.db.execute(
    "INSERT INTO sensor_alerts_events (sensor, state, seen_at) VALUES (" +
      sqlText(sensor) + ", " + sqlText(state) + ", " + now.unix + ")"
  );
  if (!saved) return json(500, { error: "could not record the event" });

  if (state === "open" && inQuiet(now.local_hour, setting("quiet_from"), setting("quiet_to"))) {
    addAlert(now.unix, sensor, "quiet-open", sensor + " opened during quiet hours");
  }
  return json(200, { ok: true, recorded: true });
}

// A sensor whose newest event is older than the limit is silent. It is
// reported once per silence: a later event re-arms it.
function checkSilence(now) {
  const limit = setting("silent_after_hours") * 3600;
  const rows = tpt.db.query("SELECT sensor, MAX(seen_at) AS seen FROM sensor_alerts_events GROUP BY sensor");
  if (rows === null) return;
  for (const row of rows) {
    const seen = Number(row.seen);
    if (now - seen < limit) continue;
    const reported = tpt.db.query(
      "SELECT id FROM sensor_alerts_alerts WHERE sensor = " + sqlText(row.sensor) +
        " AND kind = 'silent' AND created_at > " + seen
    );
    if (reported === null || reported.length > 0) continue;
    addAlert(now, row.sensor, "silent", row.sensor + " has not reported for " + Math.floor((now - seen) / 3600) + " hours");
  }
}

// Send unsent alerts, oldest first. Returns how many the channel accepted.
function sendPending() {
  const rows = tpt.db.query(
    "SELECT id, message FROM sensor_alerts_alerts WHERE notified = 0 ORDER BY id LIMIT " + MAX_PER_RUN
  );
  if (rows === null) return 0;
  let sent = 0;
  for (const row of rows) {
    if (tpt.notify(CHANNEL, String(row.message))) {
      if (tpt.db.execute("UPDATE sensor_alerts_alerts SET notified = 1 WHERE id = " + Number(row.id))) sent++;
    }
  }
  return sent;
}

function runCheck() {
  const now = tpt.now().unix;
  checkSilence(now);
  return { sent: sendPending() };
}

// Missing and empty values are not numbers, even though Number("") is 0.
function number(value) {
  return value === undefined || value === null || String(value).trim() === "" ? NaN : Number(value);
}

function saveSettings(user, body) {
  if (!user || user.role !== "admin") return json(403, { error: "only admins can change alert settings" });
  const input = parseBody(body);
  if (input === null) return json(400, { error: "body must be JSON" });
  const from = number(input.quiet_from);
  const to = number(input.quiet_to);
  const hours = number(input.silent_after_hours);
  if (!Number.isInteger(from) || from < 0 || from > 23 || !Number.isInteger(to) || to < 0 || to > 23) {
    return json(400, { error: "quiet hours must be whole hours from 0 to 23 (server local time)" });
  }
  if (!Number.isInteger(hours) || hours < 1 || hours > 720) {
    return json(400, { error: "silent-after must be between 1 and 720 hours" });
  }
  const saved =
    tpt.kv.set("quiet_from", String(from)) &&
    tpt.kv.set("quiet_to", String(to)) &&
    tpt.kv.set("silent_after_hours", String(hours));
  if (!saved) return json(500, { error: "could not save the settings" });
  return json(200, { quiet_from: from, quiet_to: to, silent_after_hours: hours });
}

function handle(req) {
  const path = req.path.replace(/\/+$/, "") || "/";
  if (path === "/hooks/feed" && req.method === "POST") return onEvent(req.body);
  if (path === "/check" && req.method === "POST") return json(200, runCheck());
  if (path === "/settings" && req.method === "POST") return saveSettings(req.user, req.body);
  return json(404, { error: "no such route" });
}
