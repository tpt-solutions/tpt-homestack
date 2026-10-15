# tpt-homestack

**One binary, one file, zero bloat, infinite extensibility.**

tpt-homestack is an ultra-lean, self-hosted full-stack platform for
resource-constrained home servers: a single Rust binary backed by one SQLite
file. It replaces the usual microservice pile-up with a compile-time
blueprint architecture — features are plug-and-play Rust modules ("blueprints")
that register their routes, migrations and UI pages with one attribute macro.

| Target | tpt-homestack | Typical Supabase/Node stack |
|---|---|---|
| Idle RAM | < 30 MB | ~1500 MB |
| Startup | < 0.5 s | ~15–30 s |
| Disk | < 30 MiB (one binary) | ~2000 MB (Docker images) |
| CPU idle | ~0% | 1–3% |

## Who it's for

tpt-homestack serves small personal apps (notes, file drop, trackers, logs,
simple automations) to your browser and phone from one lean binary. There are
two ways to run it:

- **On an existing PC** (Linux or Windows): install the one binary next to
  whatever else you use the machine for. It doesn't take over the machine, and
  you can still install anything else, including Docker containers such as
  Jellyfin, Home Assistant or Ollama.
- **As a dedicated appliance** (planned): flash an image onto a small or
  low-end device (Raspberry Pi, mini PC), boot it and open it from another
  device. No Linux or Windows install needed.

Good fit: low-power or older hardware, one small server for a household, people
who want one thing to install, back up and update instead of a dozen
containers. Not a replacement for large, mature apps such as media servers or
home automation hubs.

## Quickstart

```sh
cargo build --release -p tpt-home-cli

# 1. Initialize a data directory (creates data/config.toml, tpt.db, admin user)
./target/release/tpt-home init

# 2. Run
./target/release/tpt-home run            # http://127.0.0.1:8080
```

Open http://127.0.0.1:8080, sign in with the admin credentials printed by
`init` (or visit `/setup` on a fresh instance), and you get the shell:
dashboard, settings, admin · users — plus the sidebar entries of every
compiled-in blueprint (Notes, Drop, Track).

### Daily operation

```sh
tpt-home migrate               # apply pending migrations and exit
tpt-home run --port 9000       # one-off overrides
tpt-home user add alice --admin
tpt-home user list
tpt-home backup                # data/backups/tpt-<ts>.db + uploads/
tpt-home restore data/backups/tpt-1700000000.db
tpt-home completions bash      # zsh | fish | powershell | elvish
```

Configuration lives in `data/config.toml`; every value can be overridden via
`TPT_HOME_*` environment variables or CLI flags (`--data-dir`, `--host`,
`--port`). Details: `CONTRIBUTING.md`, `SECURITY.md`, `docs/`.

## Official blueprints

| Blueprint | What you get |
|---|---|
| `tpt-home-bp-notes` | Markdown notes, tags, FTS5 full-text search, sanitized HTML preview |
| `tpt-home-bp-drop` | Streaming file uploads, share links with optional password + expiry, cron-style cleanup |
| `tpt-home-bp-track` | User-defined relational tables (typed fields, safe dynamic SQL), row CRUD + filters |

All three are compiled into the `tpt-home` binary and demonstrate the full
blueprint API surface — they double as reference implementations.

## Write your first blueprint in 5 minutes

```sh
tpt-home new my_app          # scaffolds blueprints/tpt-home-bp-my-app and links it in
cargo build                  # that's the whole activation step
```

Starter templates (pick one with `--template`, then edit):

| Template | What it gives you |
|---|---|
| `basic` (default) | One table, one endpoint pair, a placeholder page |
| `records` | List and form: add, view and delete records |
| `logbook` | Dated numeric entries with totals, averages and a summary |
| `file-inbox` | Upload, list, download and delete files |
| `scheduled-job` | A background job that writes a periodic snapshot |
| `dashboard` | Metric cards built from grouped queries |
| `scheduled-fetch` | Fetches an admin-set URL on a schedule and keeps the responses |
| `sensor-alerts` | Signed sensor events in; alerts for an open door during quiet hours and for silent sensors |

What a blueprint looks like — no routing boilerplate, no trait methods, no
manual registration:

```rust
use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_route, bp_ui_route};

#[blueprint(id = "my_app", name = "My App")]
pub struct MyAppBlueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS my_app_items (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL
    );
"#;

#[bp_route(get, "/api/my_app/items")]
async fn list_items(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let items: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, title FROM my_app_items").fetch_all(&db).await?;
    Ok(Json(items))
}

#[bp_route(post, "/api/my_app/admin-thing", role = "admin")]
async fn admin_thing(/* ... */) -> Result<impl IntoResponse, ApiError> { /* ... */ }

#[bp_ui_route("/my_app", name = "My App")]
async fn my_app_page(user: axum::Extension<AuthUser>) -> impl IntoResponse {
    // server-rendered page, automatically in the sidebar behind login
}
```

Background work uses `#[bp_job(every = "5m")]` on an `async fn(DbPool)`; it
runs one period after boot, then on every period.

Compile-time guarantees: `#[bp_route]` paths must start with `/api/`, ids and
paths are validated by the macro, and blueprint migrations are namespaced by
crate name (`tpt-home-bp-my_app:SCHEMA`). The server discovers everything at
startup through compile-time registration — a blueprint compiles in or it
doesn't exist.

> WASM community plugins (sandboxed, `.tptp` packages, admin-approved
> capabilities) are Phase 4 of the roadmap — see `todo.md`.

## Deployment

- **Linux**: `sudo ./scripts/install.sh` installs a hardened systemd unit
  (dedicated user, `ProtectSystem=strict`, memory cap).
- **Windows**: `.\scripts\install.ps1` registers a logon Scheduled Task.
- **TLS**: `docker-compose.yml` runs only the edge — Caddy (auto-HTTPS) and/or
  Cloudflared (Cloudflare Tunnel). The app itself always runs as the single
  binary on the host.
- **Reaching it from outside the house**: [docs/REMOTE_ACCESS.md](docs/REMOTE_ACCESS.md)
  covers Tailscale (private, for family) and Cloudflare Tunnel (a public
  address), with `tpt-home remote status` and `tpt-home remote setup` to check
  and set them up. Netbird, Headscale and Caddy with your own domain are walked
  through in [docs/REMOTE_ADVANCED.md](docs/REMOTE_ADVANCED.md).
- **Non-developer guides**: [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md)
  (set up, first sign-in, troubleshooting) and
  [docs/FAMILY_SIGN_IN.md](docs/FAMILY_SIGN_IN.md) (adding family members).
- **Writing plugins**: [docs/PLUGIN_CONTRIBUTING.md](docs/PLUGIN_CONTRIBUTING.md)
  covers the rules, signing and testing.

## Architecture in one glance

```
Caddy / Cloudflare Tunnel (TLS)
        │
tpt-home (single binary)
 ├─ tpt-home-api   Axum router · auth · admin · blueprint route injection
 ├─ tpt-home-ui    server-rendered shell (login, dashboard, admin)
 ├─ tpt-home-core  SQLite (WAL) · Argon2 + sessions · config · registry
 ├─ tpt-home-cli   init · migrate · run · new · user · backup
 └─ blueprints/*   notes · drop · track · yours
        │
data/tpt.db (SQLite, WAL)  ·  data/uploads/
```

## Status & roadmap

Phases 0–2, the official blueprints, the CLI and deployment scaffolding are
implemented (see `todo.md` for the detailed checklist). In progress: full
Leptos UI hydration, the WASM plugin system (`wasmi`), and release
engineering.

## License

Dual-licensed under `MIT OR Apache-2.0` — see `LICENSE-MIT` and
`LICENSE-APACHE`. Copyright © 2026 TPT Solutions.

Contributions are welcome: dependency policy (no Apache-2.0-only crates,
no copyleft) and workflow are described in `CONTRIBUTING.md`. Security
issues: **security@tpt-solutions.example** (see `SECURITY.md`) — please do
not open public issues for vulnerabilities.
