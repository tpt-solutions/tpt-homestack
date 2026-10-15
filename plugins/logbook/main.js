// Logbook: dated entries with a label and a number. Same limits and rules as
// the `logbook` starter template.

// Length limits stop one request from storing a huge blob.
const MAX_LABEL = 100;
const MAX_NOTES = 2000;

function json(status, value) {
  return { status: status, content_type: "application/json", body: JSON.stringify(value) };
}

// The sandbox has no bound parameters, so text is quoted by hand: single
// quotes are doubled. Numbers come from Number(), never from raw input.
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

// True only for real calendar dates written as YYYY-MM-DD.
function isValidDate(text) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(text)) return false;
  const year = Number(text.slice(0, 4));
  const month = Number(text.slice(5, 7));
  const day = Number(text.slice(8, 10));
  if (month < 1 || month > 12) return false;
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
  return day >= 1 && day <= days;
}

function addEntry(body) {
  const input = parseBody(body);
  if (input === null) return json(400, { error: "body must be JSON" });
  const loggedOn = String(input.logged_on === undefined ? "" : input.logged_on).trim();
  const label = String(input.label === undefined ? "" : input.label).trim();
  const valueText = String(input.value === undefined ? "" : input.value).trim();
  const notes = String(input.notes === undefined ? "" : input.notes).trim();

  if (label === "" || [...label].length > MAX_LABEL) {
    return json(400, { error: "label must be 1-100 characters" });
  }
  if (!isValidDate(loggedOn)) {
    return json(400, { error: "date must be a real date as YYYY-MM-DD" });
  }
  const value = valueText === "" ? NaN : Number(valueText);
  if (!Number.isFinite(value)) return json(400, { error: "value must be a number" });
  if ([...notes].length > MAX_NOTES) return json(400, { error: "notes are too long" });

  const saved = tpt.db.execute(
    "INSERT INTO logbook_entries (logged_on, label, value, notes, created_at) VALUES (" +
      sqlText(loggedOn) + ", " + sqlText(label) + ", " + String(value) + ", " +
      sqlText(notes) + ", " + Math.trunc(tpt.now().unix) + ")"
  );
  if (!saved) return json(500, { error: "could not save the entry" });
  return json(201, { ok: true });
}

function deleteEntry(body) {
  const input = parseBody(body);
  const id = input === null ? NaN : Number(input.id);
  if (!Number.isInteger(id) || id <= 0) return json(400, { error: "id must be a positive integer" });

  const found = tpt.db.query("SELECT id FROM logbook_entries WHERE id = " + id);
  if (found === null) return json(500, { error: "database unavailable" });
  if (found.length === 0) return json(404, { error: "entry not found" });

  const removed = tpt.db.execute("DELETE FROM logbook_entries WHERE id = " + id);
  if (!removed) return json(500, { error: "could not delete the entry" });
  return json(200, { ok: true });
}

function handle(req) {
  const path = req.path.replace(/\/+$/, "") || "/";
  if (path === "/entries" && req.method === "POST") return addEntry(req.body);
  if (path === "/delete" && req.method === "POST") return deleteEntry(req.body);
  return json(404, { error: "no such route" });
}
