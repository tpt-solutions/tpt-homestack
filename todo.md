# tpt-homestack — Project Todo

Owner: TPT Solutions · License: `MIT OR Apache-2.0` · Targets: Linux x86_64, Linux ARM64, Windows

Key decisions
- Official blueprints (notes, drop, track) are compiled in via `#[blueprint]` macros.
- Community blueprints are WASM plugins on `wasmi`, with a declarative manifest and host-rendered UI.
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
- [ ] `trybuild` compile-fail tests with readable error messages
- [x] Proof-of-concept blueprint exercising routes, DB, and UI end to end (notes/track/drop e2e tests over the real router)
- [x] Reconcile the spec example's `query_as!` usage with real SQLx syntax (runtime `query_as` + `FromRow`, documented in README)

## Phase 3 — UI Shell (`tpt-home-ui`, Leptos)
- [ ] Set up cargo-leptos (SSR + WASM hydration)
- [x] App shell layout: sidebar, header, theming (light/dark; server-rendered, no JS framework)
- [x] Login page and auth guard
- [x] Empty dashboard (blueprint cards)
- [x] Dynamic sidebar built from active blueprints (plugins land with Phase 4)
- [x] Settings page (password change)
- [x] Admin pages: users and roles (installed-blueprints/plugins page deferred to Phase 4)
- [ ] Shared components library (forms, tables, lists, modals, toasts)
- [ ] Accessibility and responsive layout (mobile browsers)
- [ ] Embed WASM/assets into the single binary
- [ ] UI tests

## Phase 4 — WASM Plugin System (community blueprints)
- [ ] Add `wasmi` runtime to the host (verify MIT/Apache dual license in cargo-deny)
- [ ] Define plugin ABI/WIT-style interface (versioned)
- [ ] Define `plugin.toml` manifest (id, name, version, permissions, routes, pages, tables)
- [ ] Host functions: DB access (scoped to plugin-prefixed tables), key-value store, HTTP response helpers, logging
- [ ] Host functions: file access (scoped to a plugin directory) — only if permitted by the manifest
- [ ] Permission model: plugins declare capabilities, admin approves on install
- [ ] Sandbox limits: memory cap, fuel/instruction limit, timeouts
- [ ] Plugin migrations (declared in the manifest, run by the host)
- [ ] Plugin API route dispatch (`/api/<plugin-id>/…` to the WASM handler)
- [ ] Declarative UI: manifest-defined pages, forms, lists, tables, detail views rendered by the shell
- [ ] Plugin lifecycle: install, enable, disable, update, uninstall (with data retention option)
- [ ] Plugin package format (`.tptp` archive: wasm + manifest + assets) with checksum
- [ ] Optional signature verification
- [ ] Plugin SDK crate (`tpt-home-plugin-sdk`) with a macro-style authoring experience for Rust plugins
- [ ] Plugin template and example plugin (`tpt-home-bp-calendar`)
- [ ] Hot load/unload without a server restart
- [ ] Plugin isolation and failure handling (a crashing plugin must not take down the host)
- [ ] Plugin tests and fuzzing of the host boundary

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
- [ ] Leptos UI
- [ ] Tests
### `tpt-home-bp-track`
- [x] Relational data model with dynamic user-defined tables/fields
- [x] Safe dynamic SQL (integer-only identifiers `track_data_<id>`/`f_<id>`, parameterized values, kind validation)
- [x] CRUD API and filters
- [x] UI (table views, forms; server-rendered v1, Leptos hydration pending)
- [ ] Tests

## Phase 6 — CLI & Developer Experience (`tpt-home-cli`)
- [x] Clap-based `tpt-home` binary
- [x] `tpt-home init` (create data dir, config, DB, admin user)
- [x] `tpt-home migrate`
- [x] `tpt-home run`
- [x] `tpt-home new <name>` scaffold (Cargo.toml, lib.rs using macros, workspace registration; verified compiling)
- [ ] `tpt-home new --plugin <name>` scaffold for WASM plugins
- [ ] `tpt-home plugin build|install|list|enable|disable|remove`
- [x] `tpt-home user add|list|remove|reset-password`
- [x] `tpt-home backup` / `restore` (`VACUUM INTO` + uploads copy, atomic swap on restore)
- [x] Shell completions (bash, zsh, fish, powershell, elvish)
- [x] `scripts/install.sh` systemd installer (user, hardened unit file, enable/start)
- [x] Windows install script (`scripts/install.ps1`, logon Scheduled Task; script written, task registration not exercised here)
- [ ] CLI integration tests

## Phase 7 — Deployment & Platform Support
- [x] `docker-compose.yml` (Caddy + Cloudflared only)
- [x] `Caddyfile` with auto-HTTPS and reverse proxy
- [x] Cloudflare Tunnel configuration docs (docs/DEPLOYMENT.md)
- [ ] Cross-compilation for Linux ARM64
- [x] Windows build and run verification (release binary smoke-tested: init/run/login/notes/track/drop/share)
- [x] Release profile tuning (fat LTO, strip, codegen-units=1; panic kept unwinding for tower recovery)
- [ ] Single-binary build process (embedded WASM/assets)
- [ ] Upgrade/migration path between versions
- [ ] Optional Dockerfile for the app itself

## Phase 8 — Quality, Security & Performance
- [ ] Test coverage for core, macros, API, blueprints, plugins
- [ ] End-to-end tests (login, blueprint use, plugin install)
- [ ] Security review: auth, sessions, CSRF, XSS (markdown), path traversal, SQL injection, upload handling
- [ ] Plugin sandbox security review
- [ ] Dependency audit (`cargo audit`, `cargo deny`)
- [ ] Benchmark vs targets: idle RAM < 30 MB, startup < 0.5 s, binary < 25 MB, CPU idle ~0%
- [ ] Optimize memory allocations (allocator choice, pool sizes) — check the license of any allocator
- [ ] Verify the targets on ARM64 hardware
- [ ] Load and soak testing

## Phase 9 — Documentation
- [x] README (what, why, quickstart, license)
- [x] "Write your first blueprint in 5 minutes" (README section)
- [ ] "Write your first plugin" guide
- [ ] Plugin manifest and host API reference
- [x] Architecture overview (docs/ARCHITECTURE.md)
- [x] Deployment guide (systemd, Docker/Caddy, Cloudflare Tunnel, Windows) — docs/DEPLOYMENT.md
- [ ] Admin and user guide
- [x] Rustdoc for all public crates (doc comments on public items)
- [ ] Changelog and versioning policy

## Phase 10 — Release & Community
- [ ] Version scheme (semver) and release checklist
- [ ] CI release pipeline: binaries for Linux x86_64, Linux ARM64, Windows, with checksums
- [ ] Publish crates to crates.io (core, macros, plugin SDK, official blueprints)
- [ ] GitHub release and tags
- [ ] Plugin discovery: curated index/registry (repo-based)
- [ ] Blueprint/plugin contribution guidelines and templates
- [ ] Community channels (discussions)
- [ ] Post-release: collect feedback, triage, roadmap v2
