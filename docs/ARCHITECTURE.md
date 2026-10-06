# Architecture

How tpt-homestack stays a single lean binary while remaining extensible.

## The shape

```
[ Internet ]
     │
     ▼
[ Caddy / Cloudflared ]            TLS edge only (docker-compose sidecar)
     │
┌──────────────────────────────────────────────────────────┐
│ tpt-home  (single binary = tpt-home-cli + everything)    │
│                                                          │
│  tpt-home-api   Axum router, middleware, core endpoints  │
│  tpt-home-ui    server-rendered shell (login/dashboard)  │
│  tpt-home-core  SQLite, auth/sessions, config, registry  │
│  blueprints     notes · drop · track · … (compiled in)   │
└──────────────────────────────────────────────────────────┘
     │
     ▼
data/tpt.db (SQLite WAL)   data/uploads/   data/config.toml
```

The CLI crate is the binary; it links `tpt-home-api` and every official
blueprint. A custom distribution = a new binary crate that depends on the
blueprints you want. There is no plugin loading for compiled blueprints —
**a blueprint compiles in or it doesn't exist** — which is what keeps the
runtime tiny and the failure modes boring.

## Registration: compile-time, via `inventory`

Blueprints never edit the server. Four attribute macros emit `inventory`
entries (linker-section statics) that the API walks at startup:

| Macro | Emits | Consumed by |
|---|---|---|
| `#[blueprint(id, name)]` | `BlueprintEntry` + `Blueprint` impl | dashboard, docs |
| `#[bp_migration]` | `MigrationEntry { key, sql }` | `migrate::run_all` |
| `#[bp_route(method, path, role?, public?)]` | `RouteEntry` / `PublicRouteEntry` | `registry::apply_routes` / `apply_public_routes` |
| `#[bp_ui_route(path, name)]` | `UiEntry` | sidebar + `apply_ui_routes` |
| (crate-level) | `StartupEntry` | background tasks (drop cleanup) |

The macros validate ids/paths at compile time (e.g. `#[bp_route]` paths must
start with `/api/` — enforced by a `const` assertion).

## Request flow

```
request
  → security headers          (core::middleware)
  → CSRF content-type check   (SameSite=Lax cookies are the primary defense)
  → rate limiter              (fixed window per IP; strict bucket for auth)
  → [public branch]           /healthz · /api/auth/login · /api/setup ·
                              /login · /setup · blueprint public routes
  → [protected branch]
       auth resolver          cookie → sessions table → AuthUser extension
         → core pages/API, blueprint routes (#[bp_route], optional role layer),
           blueprint UI pages (#[bp_ui_route])
```

Unauthenticated `/api/*` gets `401` JSON; unauthenticated pages get a
`302 → /login`.

## Data

- **One SQLite file** (`data/tpt.db`, WAL, `NORMAL` sync, 4-connection pool,
  ~2 MiB page cache) — sized for Raspberry-Pi-class RAM budgets.
- **Migrations**: every `#[bp_migration]` is applied once, tracked in
  `_tpt_schema_migrations` under `"<crate>:<name>"` keys; core schema always
  first. Forward-only.
- **Blueprint namespacing**: blueprint tables carry the blueprint id prefix
  (`notes_*`, `drop_files`, `track_*`); track's dynamic tables are
  `track_data_<id>` with `f_<field_id>` columns, so user input can never
  become a SQL identifier.
- **Uploads** live under `data/uploads/<blueprint>/` with randomized names;
  `core::files` sanitizes client filenames and rejects traversal.

## Auth model

- Argon2id password hashes (PHC strings), users with `admin`/`user` roles.
- Sessions: 256-bit random tokens; the DB stores only SHA-256 hashes. Cookie
  is `HttpOnly; SameSite=Lax` (+`Secure` when configured).
- First-run bootstrap: `tpt-home init` or `POST /api/setup` (only while the
  instance has zero users) create the initial admin.
- Guard rails in the admin API: you cannot delete/disable yourself or remove
  the last working admin.

## Deliberate deferrals

- **Leptos hydration (Phase 3)**: the shell is server-rendered HTML with
  small inline scripts; `tpt-home-ui` is the swap-in point.
- **WASM plugins (Phase 4)**: community blueprints will run on `wasmi` behind
  a manifest + capability model; the registry types above are shaped so
  plugin routes/UI can join the same dispatch.
