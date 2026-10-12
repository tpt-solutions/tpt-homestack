# Writing plugins with an AI agent

This guide is for an AI agent (or the person directing one) writing a
tpt-homestack plugin. It covers the sandboxed JavaScript plugin, the simplest
kind to write, and how to reach the running server over MCP.

## What you can build

A plugin is a small program that answers HTTP-style requests under
`/api/<id>/…` and may declare tables, pages and form actions in its manifest.
The host renders the pages; you never ship HTML or client JavaScript. Plugins
can read and write their own tables and a key/value store, if the admin
approves those permissions.

## Layout

```
my-plugin/
  plugin.toml   manifest (identity, permissions, tables, pages, limits)
  main.js       the script; must define handle(req)

The entry file must sit at the top of the package with a plain name (no
directories), so `entry = "main.js"`. Extra files may go under `assets/` or
`src/` in the package.
```

Minimal `plugin.toml`:

```toml
[plugin]
id = "notes_quick"          # lowercase letters, digits, underscore; not starting with a digit
name = "Quick notes"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[permissions]
db = true                   # approved by the admin at install

[[tables]]
name = "notes"
[[tables.columns]]
name = "body"
kind = "text"

[[pages]]
path = "/add"
title = "Add a note"
kind = "form"
fields = ["body"]
action = "add"              # POST /api/notes_quick/add; also an MCP tool
```

## The script contract

Your script defines a global `handle(req)` and returns:

```js
function handle(req) {
  // req: { method, path, query, body }   path is relative to /api/<id>
  return { status: 200, content_type: "application/json", body: "[]" };
}
```

The only host object is the frozen global `tpt`. The typings are in
[`crates/tpt-home-plugins/types/tpt.d.ts`](../crates/tpt-home-plugins/types/tpt.d.ts).

| call | needs | returns |
|---|---|---|
| `tpt.db.query(sql)` | `db` | array of rows, or `null` if denied |
| `tpt.db.execute(sql)` | `db` | `true`/`false` |
| `tpt.kv.get/set/del` | `kv` | value or `null`; `true`/`false` for writes |
| `tpt.storage.read/write` | `storage` | file text or `null`; `true`/`false` |
| `tpt.log(...)` | none | nothing (100 lines kept per call) |

Denied or failed calls never throw. Check for `null`/`false`.

## Rules the sandbox enforces

- **Tables:** SQL may touch only tables named `<id>_<name>`. Here that means
  `notes_quick_notes`, created from the manifest at install. Other tables are
  refused. There are no bound parameters, so quote values with a helper that
  doubles single quotes, as in `docs/examples/external-python/app.py`.
- **No dynamic code:** `eval`, `Function`, and the constructor escapes are
  removed. Modules, `import`, `require`, timers and `fetch` do not exist.
- **Limits:** loops and recursion are capped (roughly from `max_fuel`), and a
  call has `max_millis` of wall-clock time. Source is capped at 1 MiB. Heap has
  no hard cap, so keep data sizes modest.
- **Source only:** the script is the whole program. Build nothing; the host
  parses it at install.

## Packaging and installing

```sh
tpt-home plugin build my-plugin                  # writes notes_quick-0.1.0.tptp
curl -H "Authorization: Bearer $ADMIN_TOKEN" -F approve=db -F package=@notes_quick-0.1.0.tptp \
     http://127.0.0.1:8080/api/plugins/install
curl -X POST -H "Content-Type: application/json" -H "Authorization: Bearer $ADMIN_TOKEN" \
     -d '{}' http://127.0.0.1:8080/api/plugins/notes_quick/enable
```

Install and enable need an admin token. The CLI install command is not
implemented yet, so use the API as above.

## Using the server over MCP

Point an MCP client at the server with a token. Create one as the agent's
owner, with the narrowest scope that works:

```sh
tpt-home token create alice --name agent --scope plugin:notes_quick   # one plugin
tpt-home token create alice --name agent --scope api                  # everything alice may use
```

Two ways to connect:

- **HTTP:** `POST http://<host>/api/mcp` with `Authorization: Bearer <token>`.
  Requests are rate limited by the server (240 a minute per client).
- **stdio:** `tpt-home mcp --token <token>` run as a child process. Logs go to
  stderr, so stdout carries only protocol lines. Not rate limited, because it
  is a local process.

Tools you will see:

- `plugins_list` shows enabled plugins, their tables, pages and permissions.
- `plugin_call` makes one request to any enabled plugin route.
- `<plugin>_<action>` is generated for each form page with an `action`. Its
  arguments are the form fields.

Scopes limit what you see. A `plugin:<id>` token gets no tools for other
plugins, and calls to them are refused.

## Checklist before you hand it over

1. `plugin.toml` parses, and `entry` names the script file at the top of the plugin directory.
2. Every SQL statement names only `<id>_<table>` tables.
3. `handle` returns `status`, `content_type` and `body` on every path,
   including the not-found and bad-input cases.
4. Denied or `null` results are handled, not assumed.
5. Install, then call the page and each form action once through MCP.
