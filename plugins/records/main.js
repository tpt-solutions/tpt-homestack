// Records: a list of things you can add and delete. Same limits and rules as
// the `records` starter template.

// Length limits stop one request from storing a huge blob.
const MAX_TITLE = 200;
const MAX_NOTES = 5000;
// Rows returned by one list call; the list page shows what this returns.
const LIST_LIMIT = 500;

function json(status, value) {
  return { status: status, content_type: "application/json", body: JSON.stringify(value) };
}

// The sandbox has no bound parameters, so text is quoted by hand: single
// quotes are doubled. Integers come from Number(), never from raw input.
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

function listRecords() {
  const rows = tpt.db.query(
    "SELECT id, title, notes, created_at FROM records_records ORDER BY id DESC LIMIT " + LIST_LIMIT
  );
  if (rows === null) return json(500, { error: "database unavailable" });
  return json(200, rows);
}

function addRecord(body) {
  const input = parseBody(body);
  if (input === null) return json(400, { error: "body must be JSON" });
  const title = String(input.title === undefined ? "" : input.title).trim();
  const notes = String(input.notes === undefined ? "" : input.notes).trim();
  if (title === "") return json(400, { error: "title must not be empty" });
  if ([...title].length > MAX_TITLE) return json(400, { error: "title is too long" });
  if ([...notes].length > MAX_NOTES) return json(400, { error: "notes are too long" });

  const saved = tpt.db.execute(
    "INSERT INTO records_records (title, notes, created_at) VALUES (" +
      sqlText(title) + ", " + sqlText(notes) + ", " + Math.trunc(tpt.now().unix) + ")"
  );
  if (!saved) return json(500, { error: "could not save the record" });
  return json(201, { ok: true });
}

function deleteRecord(body) {
  const input = parseBody(body);
  const id = input === null ? NaN : Number(input.id);
  if (!Number.isInteger(id) || id <= 0) return json(400, { error: "id must be a positive integer" });

  const found = tpt.db.query("SELECT id FROM records_records WHERE id = " + id);
  if (found === null) return json(500, { error: "database unavailable" });
  if (found.length === 0) return json(404, { error: "record not found" });

  const removed = tpt.db.execute("DELETE FROM records_records WHERE id = " + id);
  if (!removed) return json(500, { error: "could not delete the record" });
  return json(200, { ok: true });
}

function handle(req) {
  const path = req.path.replace(/\/+$/, "") || "/";
  if (path === "/records" && req.method === "GET") return listRecords();
  if (path === "/records" && req.method === "POST") return addRecord(req.body);
  if (path === "/delete" && req.method === "POST") return deleteRecord(req.body);
  return json(404, { error: "no such route" });
}
