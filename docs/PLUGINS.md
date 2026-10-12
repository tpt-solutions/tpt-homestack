# Writing plugins

Community blueprints are WebAssembly modules run in a `wasmi` sandbox. The
host grants only the capabilities the admin approves at install time and
renders any UI from the manifest, so plugins never ship HTML or JavaScript.

## Anatomy of a plugin

```
my-plugin/
  Cargo.toml        # crate-type = ["cdylib", "rlib"], depends on tpt-home-plugin-sdk
  src/lib.rs        # one handler, one export! call
  plugin.toml       # manifest: identity, permissions, limits, tables, pages
```

```rust
use tpt_home_plugin_sdk::{export, Request, Response, db_query};

fn handle(req: Request) -> Response {
    match req.path.as_str() {
        "/count" => match db_query("SELECT COUNT(*) AS n FROM notes_items") {
            Ok(rows) => Response::json(200, &rows),
            Err(e) => Response::error(500, e),
        },
        _ => Response::error(404, "not found"),
    }
}

export!(handle);
```

Build for the host's wasm target:

```sh
cargo build --release --target wasm32-unknown-unknown
```

## Manifest (`plugin.toml`)

| Section | Purpose |
|---|---|
| `[plugin]` | `id` (lowercase, digits, underscore; namespaces routes and tables), `name`, `version`, `entry` (wasm or script file in the package, or the base URL for `external`), `api_version` (must be `1`), `runtime` (`wasm`, `js` with the `js` feature, or `external`) |
| `[permissions]` | `db`, `kv`, `storage` — all default to denied. A capability is granted only when the manifest requests it **and** the admin approves it at install (`approve` form field, for example `approve=db,kv`). Anything not approved is refused at runtime, and the install response lists requested and approved. |
| `[limits]` | `max_fuel` (instruction budget per call), `max_memory_mb`, `max_millis` (wall-clock budget per call, default 5000, checked at every host call) |
| `[[tables]]` | Tables created as `<id>_<name>` at install; column kinds `text`, `integer`, `real`, `bool` |
| `[[migrations]]` | Forward-only schema changes: `version` (positive, strictly increasing) and `statements` (one each, touching only `<id>_*` tables). Each version runs once per plugin, in one transaction with the install. An edited, already-applied migration refuses the upgrade. There are no down migrations: to undo a change, add a new migration. |
| `[[pages]]` | Declarative pages under `/plugins/<id><path>`: `kind = "table"` (rows from a scoped table) or `kind = "form"` (POSTs to the plugin API) |

The example in [`plugins/tpt-home-bp-calendar/plugin.toml`](../plugins/tpt-home-bp-calendar/plugin.toml) uses every section.

For writing a wasm plugin in a language other than Rust, see the binary
interface in [`PLUGIN_ABI.md`](PLUGIN_ABI.md). For an AI agent writing or
using plugins, see [`AI_PLUGIN_GUIDE.md`](AI_PLUGIN_GUIDE.md); plugins are also
exposed to agents over MCP (`POST /api/mcp`, or `tpt-home mcp --token …`).

## External plugins (`runtime = "external"`)

An external plugin is a process you run yourself, in any language. The
manifest's `entry` is its base URL, which must be plain `http://` on loopback
(`127.0.0.1`, `localhost` or `::1`) with a port and no path:

```toml
[plugin]
id = "ticker"
runtime = "external"
entry = "http://127.0.0.1:8790"
```

- The host forwards `/api/ticker/…` to the process with the same method,
  path, query and body, and returns its status and body. The plugin's
  `max_millis` bounds each exchange. An unreachable process gives `502`, and
  a slow one gives `504`.
- The process reaches host services at `POST /api/plugin-host/ticker/<op>`,
  with a bearer token whose scope is `plugin:ticker`. Operations are
  `db/query`, `db/execute`, `kv/get`, `kv/set`, `kv/del`, `storage/read` and
  `storage/write`. Each one goes through the same rules as the wasm imports:
  approved permissions, table scoping, size caps and the deadline. Denied
  calls return `403` with the reason.
- The host still renders the plugin's pages from the manifest. Its tables are
  created at install, as for other runtimes.

Create the token and package the plugin:

```sh
tpt-home token create admin --name ticker-process --scope plugin:ticker
tpt-home plugin build docs/examples/external-python        # writes ticker-0.1.0.tptp
# Install through the admin API (the CLI install command is not implemented yet):
curl -H "Authorization: Bearer $ADMIN_TOKEN" -F approve=db -F package=@ticker-0.1.0.tptp      http://127.0.0.1:8080/api/plugins/install
curl -X POST -H "Content-Type: application/json" -H "Authorization: Bearer $ADMIN_TOKEN"      -d '{}' http://127.0.0.1:8080/api/plugins/ticker/enable
```

A working example is in [`examples/external-python/`](examples/external-python/).
It is a single Python file using only the standard library. It stores labels
in its own table and is covered by an end-to-end run.

Tokens are issued by `tpt-home token`, not by plugins. A token can reach only
the routes its scope names, and it is refused with `403` anywhere else. See
the API token section of the docs.

## JS script plugins (`runtime = "js"`)

A script plugin is a single JavaScript file (`entry`, for example `main.js`)
that defines a global `handle(req)`, returning `{status, content_type, body}`.
It runs on the Boa engine, which is compiled in only with the `js` cargo
feature (on by default in the `tpt-home` binary; build with
`--no-default-features` for a smaller binary that rejects `runtime = "js"`).

The host surface is the frozen `tpt` global, typed in
[`crates/tpt-home-plugins/types/tpt.d.ts`](../crates/tpt-home-plugins/types/tpt.d.ts).
Denied calls return `null` or `false` rather than throwing. Scripts cannot
use `eval`, `Function`, the function-constructor escapes, modules, timers or
`require`. Source is capped at 1 MiB.

Limits: `max_fuel` sets Boa's loop-iteration budget (roughly `max_fuel / 50`,
so 50M gives about a million iterations per loop); recursion is capped at 256.
`max_millis` is enforced at every host call. Boa has no heap cap, so the
script's memory use is bounded only by the source size and the host's string
caps. Treat untrusted scripts with that in mind.

## Host services (ABI v1)

| SDK function | Capability | Notes |
|---|---|---|
| `db_query(sql)` / `db_execute(sql)` | `db` | One statement. Only tables named `<id>_<name>` are accessible; `ATTACH`, `PRAGMA`, `VACUUM` and similar are rejected. No bound parameters: quote values with `sql_quote`. Results are capped at 1,000 rows. |
| `kv_get` / `kv_set` / `kv_del` | `kv` | Plugin-scoped key/value store. |
| `storage_read` / `storage_write` | `storage` | UTF-8 files in the plugin's data directory; names are confined to that directory. |
| `log` | always | Bounded to 100 lines per call. |

Failure contract for every capability-gated import:

- A denied capability, an out-of-scope statement, a missing file or any other
  ordinary failure returns `-1` to the guest. The SDK turns that into `Err`
  (reads return `Err`; writes return `Result<(), String>`), so handlers can
  report it. The host logs the reason at debug level.
- Only a passed `max_millis` deadline or exhausted `max_fuel` traps the call.

Wall-clock limits are enforced at host-call boundaries. A guest that loops
without calling the host is bounded by `max_fuel` alone, because the wasm
engine has no preemptive interrupt.

## Packages

A `.tptp` package is an uncompressed tar holding `plugin.toml`, the wasm
entry, optional `assets/` and `src/` trees, and `checksums.sha256`. The entry
may have any plain file name the manifest declares. Unpacking verifies every
file's checksum (including `plugin.toml`) and rejects traversal paths and
unexpected files before anything is written.

Installing compiles the wasm entry before touching the disk or database, so a
broken package leaves any previous install intact. A failure after that point
restores the previous package (upgrade) or removes the new package, tables and
row (fresh install). Upload via
`POST /api/plugins/install` (admin, multipart field `package`).

## Testing

The example plugin's end-to-end test runs the compiled wasm through the real
sandbox:

```sh
cargo test -p tpt-home-plugins --test calendar_plugin -- --ignored
```

It is ignored by default because it performs a nested wasm build. It needs
the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`).

Debug builds of the wasmi interpreter need more stack than the default 2 MiB
(a fuel-limited infinite loop overflows it), so the tests run plugin calls on
roomier threads. The server uses the default stack, which is fine for release
builds.

Debug-build crashes on Windows (an access violation on the first plugin DB call) went
away after upgrading to rusqlite 0.32, which bundles a newer SQLite.
Release builds are unaffected either way.
