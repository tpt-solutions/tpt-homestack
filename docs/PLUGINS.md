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
| `[plugin]` | `id` (lowercase, digits, underscore; namespaces routes and tables), `name`, `version`, `entry` (wasm file in the package), `api_version` (must be `1`) |
| `[permissions]` | `db`, `kv`, `storage` — all default to denied; approved by the admin at install |
| `[limits]` | `max_fuel` (instruction budget per call), `max_memory_mb` |
| `[[tables]]` | Tables created as `<id>_<name>` at install; column kinds `text`, `integer`, `real`, `bool` |
| `[[pages]]` | Declarative pages under `/plugins/<id><path>`: `kind = "table"` (rows from a scoped table) or `kind = "form"` (POSTs to the plugin API) |

The example in [`plugins/tpt-home-bp-calendar/plugin.toml`](../plugins/tpt-home-bp-calendar/plugin.toml) uses every section.

## Host services (ABI v1)

| SDK function | Capability | Notes |
|---|---|---|
| `db_query(sql)` / `db_execute(sql)` | `db` | One statement. Only tables named `<id>_<name>` are accessible; `ATTACH`, `PRAGMA`, `VACUUM` and similar are rejected. No bound parameters: quote values with `sql_quote`. Results are capped at 1,000 rows. |
| `kv_get` / `kv_set` / `kv_del` | `kv` | Plugin-scoped key/value store. Writes without the grant are silently ignored. |
| `storage_read` / `storage_write` | `storage` | UTF-8 files in the plugin's data directory; names are confined to that directory. |
| `log` | always | Bounded to 100 lines per call. |

A denied service returns `Err` from the SDK function, so handlers can report
it instead of trapping.

## Packages

A `.tptp` package is an uncompressed tar holding `plugin.toml`, the wasm
entry, optional `assets/`, and `checksums.sha256`. Unpacking verifies every
file's checksum (including `plugin.toml`) and rejects traversal paths and
unexpected files before anything is written. Upload via
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
