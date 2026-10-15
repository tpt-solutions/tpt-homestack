# tpt-homestack — Project Todo

Owner: TPT Solutions · License: `MIT OR Apache-2.0` · Targets: Linux x86_64, Linux ARM64, Windows

Key decisions
- Official blueprints (notes, drop, track) are compiled in via `#[blueprint]` macros.
- Community blueprints are WASM plugins on `wasmi`, with a declarative manifest and host-rendered UI.
- Plugin authoring tiers (Phase 4b): JS scripts (Boa), WASM with a documented ABI, external processes via API tokens, and an MCP server for AI agents.
- Multi-user auth with roles.
- No Apache-only dependencies (wasmtime is excluded).

---

## Phase 0 — Foundations & Licensing
- [x] Initialize git repository and `.gitignore` (`data/`, `target/`)
- [x] Create Cargo workspace root (`Cargo.toml`, resolver 2, shared `[workspace.package]` metadata)
- [x] Set workspace license to `MIT OR Apache-2.0`, authors/owner "TPT Solutions"
- [x] Add `LICENSE-MIT` (copyright TPT Solutions)
- [x] Add `LICENSE-APACHE` (copyright TPT Solutions)
- [x] Add license section to README (standard dual-license + contribution wording)
- [x] Add SPDX header convention (`// SPDX-License-Identifier: MIT OR Apache-2.0`) to source files
- [x] Add `deny.toml` (cargo-deny): allow MIT, BSD, ISC, Unicode, Zlib, and "MIT OR Apache-2.0"-style duals; deny Apache-2.0-only, GPL, AGPL
- [x] Audit the initial dependency list against the license policy (Axum, Tokio, SQLx, Clap, Argon2, wasmi, etc. — all dual-licensed; CI `cargo deny` gates the full graph)
- [x] Document the "no Apache-only dependency" rule in CONTRIBUTING
- [x] Set up CI (GitHub Actions): fmt, clippy, test, `cargo deny check`, on Linux and Windows
- [x] Add `rust-toolchain.toml` and MSRV policy
- [x] Add CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, issue/PR templates
- [x] Create empty crate skeletons: `tpt-home-core`, `tpt-home-api`, `tpt-home-ui`, `tpt-home-macros`, `tpt-home-cli` (fully implemented, see later phases)
- [x] Create `blueprints/`, `scripts/` directories
- [ ] Reserve crate names on crates.io (`tpt-home-*`)
- [x] Register the `tpt-home-bp-*` naming convention in docs (CONTRIBUTING, README)

## Phase 1 — Core Engine (`tpt-home-core`, `tpt-home-api`)
- [x] Config system (file + env + CLI overrides, defaults, validation)
- [x] Logging/tracing setup (low overhead)
- [x] SQLite pool via SQLx (WAL mode, pragmas tuned for low RAM)
- [x] Core migration runner (core + per-blueprint migrations, version tracking)
- [x] Shared types and error handling (`prelude`)
- [x] File I/O layer for `data/uploads/` (safe paths, no traversal)
- [x] Users and roles schema (admin, user; extensible via `Role::allows`)
- [x] Argon2 password hashing
- [x] Session management (secure cookies, expiry, revocation; SHA-256-hashed tokens)
- [x] First-run admin bootstrap flow (`tpt-home init` + `/api/setup` + `/setup` page)
- [x] Auth endpoints: login, logout, me, change password
- [x] User management endpoints (admin only, with last-admin/self guard rails)
- [x] Axum router and server startup
- [x] Middleware: auth, CORS, rate limiting, request IDs, security headers, CSRF content-type check
- [x] Blueprint registry: dynamic route/migration/UI collection from active blueprints (via `inventory`)
- [x] Graceful shutdown and health endpoint
- [x] Unit and integration tests for auth, roles, migrations (53 tests workspace-wide)

## Phase 2 — Macros (`tpt-home-macros`)
- [x] `#[blueprint(id, name)]` macro: registration, metadata, trait implementation
- [x] `#[bp_migration]` macro: collect and namespace-check migrations
- [x] `#[bp_route(method, path)]` macro: route registration with the `State<DbPool>` extractor (plus `role = "..."` and `public` flags)
- [x] `#[bp_ui_route(path)]` macro: route + sidebar registration (server-rendered v1; Leptos swap-in is Phase 3)
- [x] Auto-registration mechanism (`inventory`, MIT OR Apache-2.0) with explicit link anchors in the binary
- [x] Error handling with `?` in routes (`ApiError`: SQLx, I/O, JSON, auth conversions)
- [x] Per-blueprint permission and role attributes (`role = "admin"`)
- [x] Compile-time checks (route prefix `/api/` const-assert; id/path validation) — cross-crate id uniqueness is runtime-checked for now
- [x] `trybuild` compile-fail tests with readable error messages — `crates/tpt-home-macros/tests/ui/` covers a route outside `/api/`, an unknown HTTP method, a bad `#[bp_job]` interval and a non-async job; the messages name each problem
- [x] Proof-of-concept blueprint exercising routes, DB, and UI end to end (notes/track/drop e2e tests over the real router)
- [x] Reconcile the spec example's `query_as!` usage with real SQLx syntax (runtime `query_as` + `FromRow`, documented in README)

## Phase 3 — UI Shell (`tpt-home-ui`, Leptos)
- [ ] Set up cargo-leptos (SSR + WASM hydration) — DECIDED: not needed. The shell is server-rendered and no WASM bundle exists; the Leptos items below are closed on the same decision
- [x] App shell layout: sidebar, header, theming (light/dark; server-rendered, no JS framework)
- [x] Login page and auth guard
- [x] Empty dashboard (blueprint cards)
- [x] Dynamic sidebar built from active blueprints (plugins land with Phase 4)
- [x] Settings page (password change)
- [x] Admin pages: users and roles (installed-blueprints/plugins page deferred to Phase 4)
- [ ] Shared components library (forms, tables, lists, modals, toasts) — PARTIAL: `tpt-home-ui/src/components.rs` has buttons, fields, selects, forms, tables, lists, badges, messages, modals and toasts. Text is escaped, and handlers use data attributes (CSP-safe). The users page uses the modal (password reset) and toasts; the status page uses toasts; the browser check covers the modal. Not migrated yet: the welcome page, the plugin pages, the settings page and the blueprint pages, which still hand-write their markup. The `confirm()` on delete is still the browser dialog.
- [ ] Accessibility and responsive layout (mobile browsers) — PARTIAL: `scripts/browser-check.mjs` measures each page at 390 px and fails on sideways scrolling. Found and fixed the jobs table overflowing (640 px). NOT DONE: keyboard and screen-reader checks, contrast, and touch-target sizes
- [x] Embed WASM/assets into the single binary — DONE for the shell: CSS and scripts are inline and nothing is read from disk at runtime (no Leptos bundle exists)
- [ ] UI tests — PARTIAL: `scripts/browser-check.mjs` drives headless Chrome or Edge over the DevTools protocol. It checks eleven pages (including `/admin/plugins`, `/admin/remote`, `/admin/status`) for console errors and CSP violations, runs six real workflows, and checks phone width. Not in CI yet: it needs a running server and a browser

## Phase 4 — WASM Plugin System (community blueprints)
- [x] Add `wasmi` runtime to the host (verify MIT/Apache dual license in cargo-deny) — in use since Phase 4; `cargo deny` passes on the current graph
- [x] Define plugin ABI/WIT-style interface (versioned)
- [x] Define `plugin.toml` manifest (id, name, version, permissions, routes, pages, tables)
- [x] Host functions: DB access (scoped to plugin-prefixed tables), key-value store, HTTP response helpers, logging
- [x] Host functions: file access (scoped to a plugin directory) — only if permitted by the manifest
- [x] Permission model: plugins declare capabilities, admin approves on install — install takes an `approve` list; anything not approved is denied at runtime (fail closed); install reports requested vs approved
- [x] Sandbox limits: memory cap, fuel/instruction limit, timeouts
- [x] Plugin migrations (declared in the manifest, run by the host) — forward-only `[[migrations]]`, checksummed, applied in one transaction with the install; tested in tests/lifecycle.rs
- [x] Plugin API route dispatch (`/api/<plugin-id>/…` to the WASM handler)
- [x] Declarative UI: manifest-defined pages, forms, lists, tables, detail views rendered by the shell — table (with row links), list, detail (by ?id=) and form pages; escaped output; verified through a session on the ticker example. Not built: search, pagination beyond 200 rows, edit/delete actions in the UI
- [x] Plugin lifecycle: install, enable, disable, update, uninstall (with data retention option) — all covered by tests/lifecycle.rs; enable and disable take effect without a restart and keep data
- [x] Plugin package format (`.tptp` archive: wasm + manifest + assets) with checksum
- [x] Optional signature verification — Ed25519 over the exact package bytes: `plugin keygen`, `plugin sign`, `plugin install --signature`, and the admin install form. `trusted_plugin_keys` and `require_signed_plugins` in config.toml. Tests: signing unit tests and `tests/plugin_signing_cli.rs`
- [x] Plugin SDK crate (`tpt-home-plugin-sdk`) with a macro-style authoring experience for Rust plugins
- [x] Plugin template and example plugin (`tpt-home-bp-calendar`) — the example is `plugins/tpt-home-bp-calendar`; the template is `tpt-home new --plugin <name>` (see Phase 6)
- [x] Hot load/unload without a server restart — install, enable, disable and uninstall change the registry at once; verified live and in tests/lifecycle.rs
- [x] Plugin isolation and failure handling (a crashing plugin must not take down the host) — a trapping guest fails only its own call (sandbox test); the next call runs. JS plugins now run in a worker process with a memory cap (CLI build; tested on Windows, Linux not yet run). See `docs/PLUGIN_SANDBOX_REVIEW.md`
- [ ] Plugin tests and fuzzing of the host boundary — PARTIAL: `crates/tpt-home-plugins/tests/boundary.rs` runs deterministic mutated inputs (manifests, SQL scope, `.tptp` packages) and checks for panics and escapes. Coverage-guided fuzzing is built in `fuzz/` (five targets: manifest, package, catalog, update, webhook; about 7 million runs in total, no crashes). Runs are short; longer runs before releases are still to do
- [x] Debug builds: a plugin response of a few hundred bytes overflowed the tokio worker's 2 MiB stack in debug builds (wasmi frames are much larger). FIXED: debug builds run the workers with a 256 MiB stack (`crates/tpt-home-cli/src/main.rs`); release is unchanged. Reproduced and verified by hand with the calendar example; no automated regression test yet (the plugin tests run on large thread stacks, so they never hit it)

## Phase 4b — Easy Plugin Authoring (JS scripts, any-language WASM, external processes, MCP)
Four tiers sharing one manifest + capability model: JS scripts (Boa), WASM (Rust SDK + documented ABI), external processes (any language), MCP for AI agents.

### A. Foundation (prerequisite for everything below)
- [x] Link `tpt-home-plugins` into the CLI binary (Cargo dep + `BLUEPRINT_LINK_ANCHORS` entry) — `tpt-home run` serves no plugins today
- [x] Runtime abstraction: replace `LoadedPlugin.module: wasmi::Module` with an enum/trait (`Wasm` | `Script` | `External`); dispatch through it
- [x] Add `[plugin] runtime = "wasm" | "js" | "external"` to the manifest (default `wasm`, reject unknown values)
- [x] Extract shared `HostServices` (db/kv/storage/log + capability checks + size/row caps) from the wasmi closures; wasmi imports become thin wrappers
- [x] Enforce the stored `approved_permissions` at runtime (dispatch currently uses `manifest.permissions`)
- [x] Normalize denied-capability behavior (today: `-1` for reads, silent no-op for writes, trap on scope errors) before freezing ABI v1
- [x] Package format: allow `manifest.plugin.entry` and `src/**` script files (unpack currently only accepts `plugin.wasm`)
- [x] Roll back plugin dir + DB row when load fails after install — tested in tests/lifecycle.rs (forced insert failure after the package moved: an upgrade restores the old package, a fresh install leaves nothing)
- [x] Wall-clock timeout for plugin calls (checked at every host call; pure guest loops are bounded by fuel only, see docs/PLUGINS.md)

### B. JS script runtime (Boa)
- [x] Add Boa dependency (pinned); verify license and tree with `cargo deny` — pinned `=0.21.1` behind the `js` feature; licenses/bans/sources pass; `paste` advisory (RUSTSEC-2024-0436, unmaintained proc-macro pulled only by Boa) is ignored in `deny.toml` with a reason; remove it when Boa drops `paste`. Release binary: 10.33 MiB without js, 18.31 MiB with.
- [x] Script contract: `handle(req) -> {status, content_type, body}`; only global is `tpt` (log, db, kv, storage) backed by `HostServices`
- [x] Lock down the engine: no `eval`/`Function`, imports, fs, net or timers
- [x] Map `[limits]` to the engine: memory cap, instruction budget, wall-clock deadline (approximate: loop budget from max_fuel, recursion 256; no heap cap, bounded by source size and host caps)
- [ ] Compile once at load and cache; fresh context per request — deviation: validated at load, re-parsed per call (Boa scripts are bound to their realm)
- [x] Ship `tpt.d.ts` typings for editor autocomplete (TypeScript is author-side transpile only)
- [x] Script sandbox tests (infinite loop, memory bomb, denied caps, scope violations, forbidden globals)
- [x] Conformance test: same fixture plugin as wasm and JS gives identical responses

### C. Documented WASM ABI for other languages
- [x] `docs/PLUGIN_ABI.md`: exports, packed return and `-1`, JSON shapes, 4 MiB caps, re-entrant `tpt_alloc`, fuel semantics, denied/trap behavior, no WASI
- [x] Minimal WAT example in the docs — `docs/examples/minimal.wat`, compiled by a sandbox test
- [x] Starter templates for other languages — AssemblyScript: `docs/examples/assemblyscript` (source, build config, committed `plugin.wasm`), tested by `crates/tpt-home-plugins/tests/assemblyscript.rs`. NOT DONE: TinyGo (no toolchain here)

### D. API tokens + external-process plugins
- [x] `api_tokens` table (hashed token, user, name, scopes, expiry, last used) + migration — migration `tpt-home-core:004_api_tokens`; hashed, scoped (`api` or `plugin:<id>`), expiring
- [x] Bearer-token middleware yielding the same `AuthUser` as the session resolver — `auth_resolver` accepts `Authorization: Bearer` on `/api/` only, within scopes (403 out of scope)
- [x] `tpt-home token create|list|revoke`
- [x] `runtime = "external"`: register by manifest + scoped token; same capabilities and table-prefix rules; host-rendered UI unchanged — loopback URL entry, forwarded with timeout; host services at /api/plugin-host/<id>/<op> under plugin:<id> tokens
- [x] Python example external plugin (~30 lines) — `docs/examples/external-python/` (53 lines incl. comments); verified end to end: forwarding, injection-safe writes, scope and permission denials, 502 when down
- [ ] Optional (stretch): admin-approved local process supervision from the manifest

### E. MCP server (`tpt-home-mcp`)
- [x] Hand-rolled JSON-RPC 2.0 (`initialize`, `tools/list`, `tools/call`); only add an MCP crate if `cargo deny` clears it — no MCP crate added
- [x] Streamable HTTP transport at `POST /api/mcp` (bearer auth, rate limit) — request/response JSON only (no SSE streaming); the server's global rate limit applies
- [x] Stdio transport: `tpt-home mcp --token ...` — logs now go to stderr so stdout carries only protocol lines
- [ ] Add `description` + `schema` to `RouteEntry` (macro args; default description from the doc comment) — NOT DONE: tools are derived from plugin manifests, so route metadata is not needed yet
- [x] Derive tools from enabled blueprint routes and plugin manifests — DONE: plugin tools; blueprint tools for notes (list, create, get, update, delete) and track (tables, rows, add, update, delete) via `McpToolEntry`, tested end to end in `crates/tpt-home-mcp/tests/blueprint_tools.rs`. Drop is not exposed (binary uploads)
- [x] Execute tool calls through the server's HTTP stack with the caller's credential — DONE, by loopback HTTP rather than an in-process router: the caller's own token or session is forwarded, so `/api` guards, role checks, token scopes and rate limits all apply. Needs the server running (stdio alone cannot)
- [x] `docs/AI_PLUGIN_GUIDE.md` so an AI agent can write sandboxed JS plugins
- [ ] Optional: admin-only `plugin_install` MCP tool with a human approval step — DECIDED: not built. Install stays in the admin UI, where the approval is a person's click
- [x] MCP tests: scripted JSON-RPC sessions over stdio and HTTP; non-admin token cannot call admin tools — DONE: protocol and authorization tests, a spawned `tpt-home mcp` session (`tests/mcp_stdio.rs`), and a live-router test through the `/api` guard (`crates/tpt-home-mcp/tests/live_router.rs`)

### F. Housekeeping
- [x] Update `docs/PLUGINS.md` for the new runtimes, tokens and MCP — added an "API tokens and MCP" section (token CLI, scopes, 401/403, both MCP transports)
- [x] `cargo deny check` for every new dependency — advisories, bans, licenses and sources all pass on the current graph

## Phase 5 — Official Blueprints
### `tpt-home-bp-notes`
- [x] Schema and migrations (notes, tags)
- [x] CRUD API
- [x] Markdown rendering (sanitized, dependency-free subset)
- [x] FTS5 full-text search with triggers
- [x] UI: list, editor, preview, search (server-rendered v1; Leptos hydration pending)
- [x] Tests
### `tpt-home-bp-drop`
- [x] Schema (drops, expiry, access tokens)
- [x] Streaming upload/download (large files, constant memory; chunk loop + ReaderStream)
- [x] Share links with optional password and expiry
- [x] Background cleanup task (cron-style) for expired files (`StartupEntry`)
- [x] Upload size limits (`max_upload_mb`, enforced while streaming) — per-user quotas deferred
- [ ] Leptos UI — DECIDED: not built. The blueprint pages stay server-rendered, as the Phase 3 decision says
- [x] Tests: `tests/e2e.rs` (share links, passwords, multipart upload and download) and `tests/cleanup.rs` (the expiry sweep removes only expired rows and their files; a missing file does not block the row)
### `tpt-home-bp-track`
- [x] Relational data model with dynamic user-defined tables/fields
- [x] Safe dynamic SQL (integer-only identifiers `track_data_<id>`/`f_<id>`, parameterized values, kind validation)
- [x] CRUD API and filters
- [x] UI (table views, forms; server-rendered v1, Leptos hydration pending)
- [x] Tests: `tests/e2e.rs` (dynamic tables, CRUD, filters, kind validation) and `tests/csv.rs` (export→import is byte-identical, case-insensitive headers, one bad row saves nothing, non-UTF-8 refused)

## Phase 6 — CLI & Developer Experience (`tpt-home-cli`)
- [x] Clap-based `tpt-home` binary
- [x] `tpt-home init` (create data dir, config, DB, admin user)
- [x] `tpt-home migrate`
- [x] `tpt-home run`
- [x] `tpt-home new <name>` scaffold (Cargo.toml, lib.rs using macros, workspace registration; verified compiling)
- [x] `tpt-home new --plugin <name>` scaffold for WASM plugins — creates `plugins/<name>` (plugin.toml, SDK crate, a table with list, add and delete routes) and registers it in the workspace; the CLI is not changed. Tested in a scratch workspace (tests/migrate_and_new_cli.rs) and by building the generated plugin to wasm, packing and installing it
- [x] `tpt-home plugin build|install|list|enable|disable|remove` — `list` reads the plugins table; `install` takes `--approve` and `--enable` and matches the admin API; `remove` needs `--yes` unless `--keep-data`; verified by a live smoke run (the CLI writes the DB, so a running server picks up changes on restart)
- [x] `tpt-home user add|list|remove|reset-password`
- [x] `tpt-home backup` / `restore` (`VACUUM INTO` + uploads copy, atomic swap on restore). Each backup now goes in its own timestamped folder; the earlier flat layout let a newer backup overwrite an older backup's uploads. Covered by `each_backup_keeps_its_own_uploads` in `tests/admin_cli.rs`
- [x] Shell completions (bash, zsh, fish, powershell, elvish)
- [x] `scripts/install.sh` systemd installer (user, hardened unit file, enable/start)
- [x] Windows install script (`scripts/install.ps1`, logon Scheduled Task; script written, task registration not exercised here)
- [x] CLI integration tests — DONE (plugin signing, mcp stdio and the `new` scaffolds are covered too): `tests/plugin_cli.rs`, `tests/admin_cli.rs` and `tests/migrate_and_new_cli.rs` run the real binary for `plugin`, `user`, `token`, `backup`, `restore`, `migrate` (idempotent after init) and the refusal paths of `new` (bad id, outside a workspace). `run` is covered by `tests/e2e_server.rs` (it starts the real server). `new` success is covered for blueprints and plugins in a scratch repo (`tests/migrate_and_new_cli.rs`)

## Phase 5b — Automation and Data I/O
- [x] Scheduled-job hook: `#[bp_job(every = "5m")]` on `async fn(DbPool) -> Result<(), ApiError>`; `JobEntry` registry with one loop per job, each run in its own task so errors are logged and panics contained; first run one period after boot; interval checked at compile time (`s`/`m`/`h`/`d`, minimum 10s). Verified by loop tests (repeat, error, panic) and interval-parser tests, and live: a 10s job in a scratch blueprint wrote rows at 10, 20 and 30s
- [x] Job visibility: admin page `/admin/jobs` (sidebar "Admin · Jobs") and `GET /api/admin/jobs` (admin only) show each job's period, timeout, last start and finish, last outcome and error, next run, runs and failures. Status is kept in memory, so it resets on restart. Verified live: the Drop cleanup job's first run was recorded as `ok`, and viewers and anonymous callers were refused
- [x] Job timeout: `#[bp_job(timeout = "...")]`, default 5 minutes. A run over its cap is aborted and logged; runs never overlap because the next tick waits for the current run. Tested with a run that never returns; the job keeps running
- [x] Drop's expiry cleanup moved from `StartupEntry` to `#[bp_job(every = "1m")]`, so there is one scheduling mechanism. Behaviour change: the first sweep runs one minute after boot instead of immediately
- [x] `tpt-home new --template scheduled-job` (the job hook is now available)
- [x] Webhook signature check (`tpt_home_core::webhook`): HMAC-SHA256 over `"<timestamp>.<body>"`, constant-time comparison, timestamp window against replays. Verified against RFC 4231 test vectors 1 and 6 and against tampered bodies, wrong secrets, stale and future timestamps, and malformed headers
- [x] Webhook route and secret management: admin CRUD at `/api/webhooks` (secret shown once, `secret` never listed), public receiver `POST /api/hooks/{name}` verified with `webhook::verify` (window 5 min), bodies up to 64 KiB stored in `webhook_deliveries` (newest 200 kept). Verified live with 24 checks (signature, timestamps, unknown names, non-admin 403, duplicate 409, non-UTF-8, oversized, delete) and a 205-delivery retention check in the database
- [x] Webhook delivery to blueprint code — DONE: `#[bp_webhook("name")]` runs its handler once per accepted delivery, after the sender's 202, in its own task (timeout 60 s, panics contained). Test: `crates/tpt-home-plugins/tests/webhook_push.rs`. Blueprints can still read `webhook_deliveries` themselves
- [x] Webhook replay protection inside the window: decided not to build a nonce store. The 5-minute window limits replay to that window, and a captured request can be resent within it. Residual risk accepted for home sensors; revisit if webhooks carry commands (unlock, disarm)
- [x] Webhook rate limit: decided to keep the global limit (240 requests per 60 s per client). Home sensors send a few events a minute at most; revisit if a sender batches events. Documented as a known limit
- [x] Track CSV import and export: `GET /api/track/tables/{id}/export.csv` (RFC 4180, CRLF, header of field names) and `POST /api/track/tables/{id}/import` (`text/csv` body up to 1 MiB, 5000 rows max). Header maps columns case-insensitively, unknown or duplicate columns are refused, every cell is checked against its kind, and any bad row means nothing is saved (first 20 problems reported with line numbers). Verified with 19 live checks and a byte-identical export → import → export round trip. The content-type guard now also accepts `text/csv`; this is safe because HTML forms cannot send it and cross-origin requests need a preflight that is not granted
- [x] Seven starter templates for `tpt-home new --template`: `basic`, `records`, `logbook`, `file-inbox`, `scheduled-job`, `dashboard`, `scheduled-fetch`. Multi-file layout (`templates/<kind>/src/`) with per-template extra dependencies. Verified together in one scratch workspace: all 10 blueprints linked, every API route exercised, admin-only routes return 401 anonymous and 403 for a user, pages render 200, job-driven rows appear on a 10s schedule, file upload/download/delete round-trips byte for byte, no panics or job failures in the log
- [x] CI job `templates` (`scripts/check-templates.sh`) scaffolds every template into a temp copy of the tree, checks each generated crate and builds the CLI that links them all; runs under the CI `-D warnings` setting. Passes locally in about 2 minutes
- [x] Sensor-alerts starter template (`--template sensor-alerts`): senders post signed events to a feed webhook; a one-minute `#[bp_job]` reads each delivery once and raises a `quiet-open` alert for a door opened during quiet hours (server local time) and a `silent` alert for a sensor with no report for the set number of hours, once per silence. Admins create the webhook and set the rules on the page. Verified live with 22 checks (access rules, validation, both rules, no repeats, each delivery recorded once) and by generating and testing the crate. Not included: notifications (email, chat), cameras, Zigbee/Z-Wave, MQTT. Those need an outbound channel or external plugins, and this is not a certified alarm system
- [x] Outbound notifications for alerts — DONE: SMTP email (`/api/notify/email`, settings, test send, reserved `email` channel for `tpt.notify`; password kept in the database like other secrets; `tests/email.rs` against a fake SMTP server) and chat or HTTP channels (`/api/notify/channels`)
- [x] Template scaffold: `scheduled-fetch` pulls in reqwest (rustls). `cargo deny check` passes on the scratch workspace that includes the generated crate (advisories, bans, licenses, sources). The repo's own CI deny job covers only the repo graph, so generated crates still need a manual check

- [x] Times for users: quiet hours use the server's local time zone (chrono Local); the settings page shows the server's current local time and UTC offset; alert text no longer carries a clock time (each alert's time shows in the viewer's own zone); the scheduled-job snapshot table shows the local time instead of a UTC day. Verified live: the open-door alert only fires when the server's local hour matches, on a machine 12 hours ahead of UTC

### F. Gaps found when checking whether templates can become plugins
Checked `logbook`, `records` and `sensor-alerts` against the plugin runtimes (wasm/js/external). Plugins have only `db`, `kv`, `storage` and `log` today.
- [x] Scheduled jobs for plugins: `[[jobs]]` in the manifest (`name`, `every`, `path`) and one host scheduler (`plugins/src/jobs.rs`, started by a startup entry). Sequential runs, bounded by `max_millis`, re-armed on re-enable. Verified by tests/plugin_jobs.rs. Gaps: results are logged only, not on the admin jobs page; runs are not yet audited in the database
- [x] Current time and timezone for plugins: `tpt.now()` in the JS tier returns Unix seconds plus the server's local date, hour, minute and UTC offset; always allowed. NOT in the WASM tier: adding a host import changes the frozen ABI v1, so that is a separate decision
- [x] Public (unauthenticated) plugin endpoints with host-verified HMAC signatures: `[[hooks]]` in the manifest, `POST /api/plugin-hooks/<id>/<name>` verified with `tpt_home_core::webhook::verify` (5-minute window, 64 KiB cap, same 401 for every refusal), handler called at `/hooks/<name>` with the verified body. Admin rotates the secret at `/api/plugins/<id>/hooks/<name>/secret`; uninstall deletes secrets. Verified through the real router in tests/plugin_hooks.rs. Gaps: the handler is tested directly, and the router only checks the anonymous refusal of the admin route, not an admin's success through the role guard; no replay store, same as core; secrets are stored in plain text because HMAC needs them
- [x] Authenticated user identity passed to the plugin handler (`Request.user`: `id`, `username`, `role`) for HTTP and MCP calls, in the JS and WASM tiers (the request JSON gains an optional `user` key, so ABI v1 guests see no change). NOT forwarded to external plugins yet: the forward would need a trusted header; add it with the external tier's next change
- [x] Outbound HTTP capability: `http` approval plus `http_hosts` declared in `[permissions]` (host or host:port, exact match after normalising). JS tier only: `tpt.http.get/post` return `{status, body}` or null, 1 MiB body cap, redirects not followed, request line and headers cannot be injected. HTTPS: `https` feature (ureq 3 with rustls and the OS trust store), on in the default CLI build. New crates: rustls 0.23 with the ring provider, rustls-platform-verifier (OS trust store) and webpki-roots (CDLA-Permissive-2.0, allowed); `cargo deny` licences, sources and bans pass. The default release CLI is 22.2 MiB against 20.5 MiB without it, so HTTPS costs about 1.7 MiB. Not tested: a real TLS handshake to a trusted server (the tests only check that TLS to a plain device fails cleanly). WASM tier not added (ABI frozen). Plain HTTP verified against a local TCP device in tests/script_http.rs; not run against a real LAN device
- [x] Notifications capability: admin-configured channels (`/api/notify/channels`, URL kept as plain text and never listed, outcome recorded per channel), plugins name them in `notify_channels` and the admin approves `notify`; `tpt.notify(channel, text)` posts `{"text": ...}` as JSON over http or https. Verified by tests/notify.rs against a local device; not yet tested against a real chat service. Email and push are not built
- [x] Declarative page kinds for summaries: `kind = "summary"` with up to 12 `metrics` (label + one scoped SELECT), shown as cards, checked at install (tests/summary_page.rs). Grouped metrics: `grouped = true` on a metric makes each SQL row a (group, value) line in a small table, capped at 50 rows (tests/logbook_plugin.rs). Still not built: charts (the `dashboard` template's)
- [x] Port `records` as a plugin first: `plugins/records` (JS), same rules as the template, verified by `tests/records_plugin.rs` through install and dispatch. Deviation: delete is a form page that posts an id, since the plugin UI has no row actions yet
- [x] Port `sensor-alerts` as a plugin: `plugins/sensor-alerts` (JS) uses the signed feed hook, a one-minute check job, notifications, `tpt.now()` quiet hours, kv settings and summary and list pages. Verified by tests/sensor_alerts.rs: quiet-hours and silence alerts each sent exactly once, admin-only settings, bad events ignored. Differences from the template: events arrive on the hook instead of being polled, and the feed secret is created with the plugin's admin route, not the core webhook page. Not yet: the template's per-sensor event history page
- [x] Port `logbook` as a plugin: `plugins/logbook` (JS) with the same validation as the template, a summary page with totals and grouped cards (total by month, entries by label), list, add and delete. Verified by tests/logbook_plugin.rs

## Phase 7 — Deployment & Platform Support
- [x] `docker-compose.yml` (Caddy + Cloudflared only)
- [x] `Caddyfile` with auto-HTTPS and reverse proxy
- [x] Cloudflare Tunnel configuration docs (docs/DEPLOYMENT.md)
- [ ] Cross-compilation for Linux ARM64 — BLOCKED here: `cargo check --target aarch64-unknown-linux-gnu` fails in `ring`'s build script (no `aarch64-linux-gnu-gcc` installed)
- [x] Windows build and run verification (release binary smoke-tested: init/run/login/notes/track/drop/share)
- [x] Release profile tuning (fat LTO, strip, codegen-units=1; panic kept unwinding for tower recovery)
- [x] Single-binary build process (embedded WASM/assets) — the shell's assets are inline, so the release binary is the only file needed
- [x] Upgrade/migration path between versions — DONE: newer databases are refused; a checked copy is saved before a migration; `migrate --dry-run` lists pending migrations. Verified with a real older binary (commit 1e0f1ac) against the current one; `scripts/upgrade-check.ps1` repeats it before a release
- [ ] Optional Dockerfile for the app itself — WRITTEN, NOT TESTED: `Dockerfile` and `.dockerignore`; no Docker here to build or run it
- [ ] One-line install script that downloads the right release binary, verifies its checksum, and sets up the service (Linux/Windows); depends on the Phase 10 release pipeline

## Phase 8 — Quality, Security & Performance
- [ ] Test coverage for core, macros, API, blueprints, plugins — PARTIAL: measured with cargo-llvm-cov: 83.4% of lines across the crates (weakest: the JS script worker process at 4%, the plugin SDK inside wasm at 23%, plugin dispatch at 41%, the webhook routes at 51%). New tests this round cover the plugin host route, the admin route smoke tests and the webhook administration; the script worker is still untested
- [x] End-to-end tests (login, blueprint use, plugin install) — `crates/tpt-home-cli/tests/e2e_server.rs` starts the real `tpt-home run` on a loopback port and drives it over TCP: login and refusals, notes (create, list, render), track (table, row, CSV export), building and installing the records plugin through the multipart endpoint, and the multipart header refusal
- [x] Security review: auth, sessions, CSRF, XSS (markdown), path traversal, SQL injection, upload handling — internal code review in `docs/SECURITY_REVIEW.md`. Seven fixes shipped, including a separate login rate limit (20/min), per-account lockout (10 failures, 15 min), a required `X-TPT-Request` header on multipart, no internal error detail sent to clients, no username-revealing login timing, and a 1024-character password cap. An outside review is still needed
- [ ] Gate: this security review must pass before remote access (Tailscale or tunnel) is promoted to ordinary users in docs or the wizard — OPEN, and it cannot be closed in this repo: an independent review is required. Remaining residuals in `docs/SECURITY_REVIEW.md`: inline styles still allowed; plain-text hook and notification secrets; per-process rate limits and lockout; no coverage-guided fuzzing; the JS worker's Linux memory path not run; the browser checks are not in CI
- [x] Plugin sandbox security review — `docs/PLUGIN_SANDBOX_REVIEW.md`: each control mapped to its test. The JS memory gap found in it is fixed in the CLI build: JS calls run in a worker process under an OS memory limit (`script_worker.rs`). OPEN: confirm the Linux `RLIMIT_DATA` path in CI (it type-checks, it has not run)
- [x] Dependency audit (`cargo audit`, `cargo deny`): `cargo deny check` passes (advisories, bans, licenses, sources). `cargo audit` reports `rsa` RUSTSEC-2023-0071, which is only in Cargo.lock via `sqlx-mysql` and not compiled into any target (no fix exists); ignored with that reason in `.cargo/audit.toml`. The `paste` warning is accepted in `deny.toml`. Re-check on each dependency change
- [x] Benchmark vs targets: idle RAM < 30 MB, startup < 0.5 s, binary < 30 MiB (was 25 MB), CPU idle ~0% — measured on x86_64 Windows with `scripts/bench.ps1`: 22.3 MB, 247 ms, 11.3 MB, 0%. See `docs/BENCHMARKS.md`. NOT measured: ARM64 hardware, RAM under load, soak
- [x] Optimize memory allocations — DONE: mimalloc as the global allocator; 704 → 946 requests per second, p99 98 → 67 ms, about +1 MB idle and +8 MB after a burst (see `docs/BENCHMARKS.md`)
- [ ] Verify the targets on ARM64 hardware
- [ ] Load and soak testing — PARTIAL: `scripts/load.ps1` ran a 30-minute soak earlier (2.9 million reads, all 200, server memory level at about 26 MB after the first two minutes, p99 23 ms), and a 20,000-request comparison this round (see `docs/BENCHMARKS.md`). NOT DONE: soaks of hours, concurrent writes, plugin and blueprint mixes. Rare stalls of 1–8 s remain unexplained

## Phase 9 — Documentation
- [x] README (what, why, quickstart, license)
- [x] "Write your first blueprint in 5 minutes" (README section)
- [x] "Write your first plugin" guide
- [x] Plugin manifest and host API reference
- [x] Architecture overview (docs/ARCHITECTURE.md)
- [x] Deployment guide (systemd, Docker/Caddy, Cloudflare Tunnel, Windows) — docs/DEPLOYMENT.md
- [x] Admin and user guide — `docs/GUIDE.md`: part 1 for people using the apps, part 2 for whoever runs the server (install, settings, users, tokens, apps, jobs, backups, sign-in protection, troubleshooting). Plain language; screenshots not included
- [x] Rustdoc for all public crates (doc comments on public items)
- [x] Changelog and versioning policy — `CHANGELOG.md` (Keep a Changelog, `[Unreleased]` holds the work since 0.1.0) and `docs/RELEASING.md`

## Phase 10 — Release & Community
- [x] Version scheme (semver) and release checklist — `docs/RELEASING.md` (pre-1.0 rules, the breaking-change list, a nine-step checklist, hotfixes). The checklist's tests, fmt, clippy and cargo deny/audit steps were run and pass here
- [ ] CI release pipeline: binaries for Linux x86_64, Linux ARM64, Windows, with checksums — PARTIAL: written, not run. `.github/workflows/release.yml` (tag `v*`, tag must match workspace version, native runners, SHA256SUMS, GitHub release). NOT YET RUN: needs a first tag push to verify; ARM64 relies on the `ubuntu-22.04-arm` runner
- [ ] Publish crates to crates.io (core, macros, plugin SDK, official blueprints)
- [ ] GitHub release and tags
- [ ] Plugin discovery: curated index/registry (repo-based); the in-app catalog (Phase 11) reads this index — PARTIAL: the reader is built (`catalog_url`, verified install); the format is in `docs/PLUGIN_INDEX.md`. Publishing an index is out of scope for now
- [x] Blueprint/plugin contribution guidelines and templates — `docs/PLUGIN_CONTRIBUTING.md`; templates from `tpt-home new`
- [ ] Community channels (discussions)
- [ ] Post-release: collect feedback, triage, roadmap v2

## Phase 11 — Ordinary-user experience (no terminal, no Rust)
Phases 0-10 serve people comfortable with a shell and a compiler. This phase is for someone who just bought a PC and wants apps running. Depends on release binaries (Phase 10) and plugins that install without compiling (Phase 4/4b).
- [ ] BLOCKED on tools here (not started): Native installers: Windows `.msi`/setup exe (installs, registers service, adds Start-menu shortcut that opens the UI); Linux `.deb` and `.rpm`; no Rust toolchain, no manual `init`
- [ ] Browser-based first-run wizard: create admin, name the server, pick starter apps, choose remote access — PARTIAL: /welcome has the name step, family accounts (with a phone-link QR card), a remote-access step that links to `/admin/remote`, and a starter-app picker that installs selected catalog apps with their plain-language approvals. Restore from the browser stays a CLI decision
- [x] In-app app catalog: browse the index, install with checksum and signature checks and the admin's approvals (`catalog_url`; Admin · Plugins page; `tests/catalog.rs`). Enable, disable and remove are on the same page. Not built: a curated public index (publishing)
- [x] Capability approval written in plain language (DONE: the Admin · Plugins page shows each ticked ability as a sentence and lists anything requested but not allowed) ("this app can read your notes") instead of raw permission names
- [x] Reachable by name on the LAN (mDNS, e.g. `homestack.local`) and a printed or QR link for phones — DONE on the server side: the server announces `<lan_name>.local` over mDNS when it listens beyond loopback (`crates/tpt-home-api/src/lan.rs`; a test browses the announcement back on this machine), and the Settings page shows the address with a QR code. NOT verified: name resolution from real phones on a home network
- [ ] Remote access scope: private access for the owner and family first, not public sharing. Tailscale (default) and Cloudflare Tunnel are offered on the admin Remote access page (`/admin/remote`), which shows live checks and the steps. Netbird and Headscale are docs-only (`docs/REMOTE_ADVANCED.md`). PARTIAL: setup changes stay in `tpt-home remote setup` by decision
- [ ] "Connect from anywhere" wizard (first-run step and a settings page): step 1 pick a provider, step 2 live checks and plain-language errors, step 3 a per-member how-to with a QR code. PARTIAL: `/admin/remote` does steps 1 and 2, including a probe of the tailnet `/healthz` address when Tailscale has one; the welcome step has the QR code. Not built: a probe for the Cloudflare hostname (the server does not know it)
- [ ] Tailscale wizard path: detect the official client, show the tailnet name, offer `tailscale serve`, set `secure_cookies`, keep `host = 127.0.0.1`. PARTIAL: detection, the steps and the reachability probe are on `/admin/remote`; `tpt-home remote setup --provider tailscale` makes the change after confirmation. In-page setup stays a CLI decision
- [x] Decide how Tailscale is integrated: guide the user to install the official client (simplest, nothing bundled) vs. embedding tsnet or similar; check licenses against `deny.toml`; record the decision — DECIDED: guide the user to install the official client. Nothing is bundled, so no new licence needs checking against deny.toml.
- [ ] On the appliance image (Phase 12), include the Tailscale client and a UI step to sign in and join the tailnet, so no shell is needed
- [ ] Cloudflare wizard path: domain, tunnel, Access, then `trust_proxy` and `secure_cookies`. PARTIAL: `/admin/remote` shows the client and steps; `tpt-home remote setup --provider cloudflare` lists the dashboard steps and changes the config after confirmation. Creating the tunnel and Access are left to the person
- [x] Advanced remote access docs: Caddy with a domain, Netbird (fully open source, free hosted tier), Headscale (self-hosted, needs a VPS) with step-by-step instructions — DONE: `docs/REMOTE_ADVANCED.md` has the steps for Caddy with a domain, Netbird and Headscale. Not tried against live Netbird or Headscale services.
- [x] `docs/REMOTE_ACCESS.md`: plain-language guide with Tailscale and Cloudflare first, then the advanced options and honest trade-offs (hosted coordination server, free-tier limits, CGNAT, domain cost); link from `docs/DEPLOYMENT.md` and `README.md` — DONE: docs/REMOTE_ACCESS.md, linked from README and DEPLOYMENT.md.
- [x] `tpt-home remote status|setup` CLI backing the wizard (detects Tailscale, Netbird and cloudflared, prints guided steps, updates `config.toml` after `confirm()`); needs a small config-write helper; tested with fake `tailscale`/`cloudflared` scripts on PATH in `crates/tpt-home-cli/tests/remote_cli.rs` — DONE: crates/tpt-home-cli/src/remote.rs; tests/remote_cli.rs runs it with fake tailscale and cloudflared scripts (set TPT_HOME_TAILSCALE_BIN and TPT_HOME_CLOUDFLARED_BIN). Known limit: config.toml comments are dropped when setup rewrites the file.
- [ ] Family access: family members use existing `Role::User` accounts; the wizard creates them and each one's sign-in how-to is in `docs/FAMILY_SIGN_IN.md`. PARTIAL: DECIDED not to build the per-user remote-access flag: the tunnels do not identify the user at the server, so the flag could not be enforced reliably
- [x] `trust_proxy` config (default false): `client_key` only trusts `X-Forwarded-For` when set, so the rate limiter can't be evaded by a spoofed header. Tested by unit tests and `forwarded_header_ignored_without_trust_proxy` / `forwarded_header_keys_the_limiter_with_trust_proxy` in `crates/tpt-home-api/tests/integration.rs`. Setup: `trust_proxy = true` in `config.toml` (or `TPT_HOME_TRUST_PROXY=true`) for the Caddy/cloudflared setups, as `docs/DEPLOYMENT.md` says. The compose file cannot set it because the binary runs on the host
- [ ] Safe defaults when remote access is on: HTTPS, rate limiting, forced admin password change, and a visible "exposed to the internet" warning. PARTIAL: DONE: admin-set passwords must be changed before use (`safety.rs`); the exposed notice on every page; `require_https = true` redirects or refuses plain HTTP (opt-in, so home networks without certificates keep working; `tests/https_required.rs`). Not on by default
- [ ] In-app backup and restore — PARTIAL: done: take a backup now, list backups, download a backup database, upload a database file as a new backup (checked first), and scheduled backups with retention (`backup_every_hours`, `backup_keep`; tests in `crates/tpt-home-api/tests/backup_api.rs`, live check with curl). Not done: restore from the browser (needs the server stopped, so it stays `tpt-home restore`), and uploads are not in the download (database only)
- [ ] In-app update: check for a new release, apply it, roll back on failure. PARTIAL: check-only is built (`update_url`, Admin · Status card, `/api/admin/update`, validated manifest with HTTPS and SHA-256; `tests/update_check.rs`). DECIDED: applying and rolling back wait for the release pipeline
- [x] Health and status in the UI — DONE (disk space included): `/admin/status` and `/api/admin/health` show version, uptime, blueprints, database size, last backup, backup settings and failing jobs. Disk free and total come from the `fs4` crate (`/api/admin/health`, `disk_free_bytes`)
- [x] Non-developer docs — DONE in text (`docs/GETTING_STARTED.md`, `docs/FAMILY_SIGN_IN.md`) with screenshots (`docs/screenshots/`, taken by `scripts/screenshots.mjs`). Not built: per-app user guides
- [ ] Usability test with 2-3 non-developers doing the full path from install to first app

## Phase 12 — Appliance image (dedicated device, no OS setup)
Goal: flash an image onto a small or low-end device, boot it, and open `homestack.local` from another device. No Linux or Windows install, no terminal. A kernel is still needed, so "replacing the OS" means a minimal purpose-built Linux that runs only homestack. Shares the first-run wizard, in-app update and in-app backup from Phase 11.
- [ ] BLOCKED on a Linux build host and hardware (not started). Choose the base: Buildroot, Alpine or Yocto (size, ARM64 + x86_64 support, license, update story) and record the decision
- [ ] Minimal image build pipeline in CI: x86_64 (USB/ISO for mini PCs and old desktops) and ARM64 (Raspberry Pi SD image), with checksums
- [ ] Image runs only homestack: read-only root, writable data partition, hardened service, no SSH or shell by default (opt-in for admins)
- [ ] First boot: DHCP, `homestack.local` via mDNS, headless first-run wizard in the browser; no monitor or keyboard needed
- [ ] Atomic A/B updates with rollback (image or binary), driven from the UI
- [ ] Data partition grows to fill the disk; backup/restore compatible with the non-appliance install
- [ ] Hardware support list and tested devices (Pi 4/5, common mini PCs); verify idle RAM and boot time on the smallest target
- [ ] Docs: flashing guide for non-developers, hardware buying guide
- [ ] Decide whether to support optional Docker/containers on the image (default: no; keep the image minimal)
- [ ] Prebuilt-binary constraint: the image has no Rust toolchain, so apps must come from the plugin catalog (Phase 11), not compiled-in blueprints
- [x] Convert selected starter templates (e.g. `logbook`, `records`, `sensor-alerts`) into installable plugins so the first catalog entries come from them (templates stay as the developer path) — done in Phase 5b (section F); the catalog itself (Phase 11) is not built
