# tpt-homestack — Project Todo

Owner: TPT Solutions · License: `MIT OR Apache-2.0` · Targets: Linux x86_64, Linux ARM64, Windows

Key decisions
- Official blueprints (notes, drop, track) are compiled in via `#[blueprint]` macros.
- Community blueprints are WASM plugins on `wasmi`, with a declarative manifest and host-rendered UI.
- Multi-user auth with roles.
- No Apache-only dependencies (wasmtime is excluded).

---

## Phase 0 — Foundations & Licensing
- [ ] Initialize git repository and `.gitignore` (`data/`, `target/`)
- [ ] Create Cargo workspace root (`Cargo.toml`, resolver 2, shared `[workspace.package]` metadata)
- [ ] Set workspace license to `MIT OR Apache-2.0`, authors/owner "TPT Solutions"
- [ ] Add `LICENSE-MIT` (copyright TPT Solutions)
- [ ] Add `LICENSE-APACHE` (copyright TPT Solutions)
- [ ] Add license section to README (standard dual-license + contribution wording)
- [ ] Add SPDX header convention (`// SPDX-License-Identifier: MIT OR Apache-2.0`) to source files
- [ ] Add `deny.toml` (cargo-deny): allow MIT, BSD, ISC, Unicode, Zlib, and "MIT OR Apache-2.0"-style duals; deny Apache-2.0-only, GPL, AGPL
- [ ] Audit the initial dependency list against the license policy (Axum, Tokio, SQLx, Leptos, Clap, Argon2, wasmi, etc.)
- [ ] Document the "no Apache-only dependency" rule in CONTRIBUTING
- [ ] Set up CI (GitHub Actions): fmt, clippy, test, `cargo deny check`, on Linux and Windows
- [ ] Add `rust-toolchain.toml` and MSRV policy
- [ ] Add CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, issue/PR templates
- [ ] Create empty crate skeletons: `tpt-home-core`, `tpt-home-api`, `tpt-home-ui`, `tpt-home-macros`, `tpt-home-cli`
- [ ] Create `blueprints/`, `scripts/` directories
- [ ] Reserve crate names on crates.io (`tpt-home-*`)
- [ ] Register the `tpt-home-bp-*` naming convention in docs

## Phase 1 — Core Engine (`tpt-home-core`, `tpt-home-api`)
- [ ] Config system (file + env + CLI overrides, defaults, validation)
- [ ] Logging/tracing setup (low overhead)
- [ ] SQLite pool via SQLx (WAL mode, pragmas tuned for low RAM)
- [ ] Core migration runner (core + per-blueprint migrations, version tracking)
- [ ] Shared types and error handling (`prelude`)
- [ ] File I/O layer for `data/uploads/` (safe paths, no traversal)
- [ ] Users and roles schema (admin, user; extensible permissions)
- [ ] Argon2 password hashing
- [ ] Session management (secure cookies, expiry, revocation)
- [ ] First-run admin bootstrap flow
- [ ] Auth endpoints: login, logout, me, change password
- [ ] User management endpoints (admin only)
- [ ] Axum router and server startup
- [ ] Middleware: auth, CORS, rate limiting, request IDs, security headers
- [ ] Blueprint registry: dynamic route/migration/UI collection from active blueprints
- [ ] Graceful shutdown and health endpoint
- [ ] Unit and integration tests for auth, roles, migrations

## Phase 2 — Macros (`tpt-home-macros`)
- [ ] `#[blueprint(id, name)]` macro: registration, metadata, trait implementation
- [ ] `#[bp_migration]` macro: collect and namespace-check migrations
- [ ] `#[bp_route(method, path)]` macro: route registration with the `State<DbPool>` extractor
- [ ] `#[bp_ui_route(path)]` macro: Leptos route and sidebar registration
- [ ] Auto-registration mechanism (e.g. `inventory`/`linkme`; check licenses)
- [ ] Error handling with `?` in routes (return-type conversion)
- [ ] Per-blueprint permission and role attributes
- [ ] Compile-time checks (id uniqueness, route prefix `/api/<id>/`, table prefix)
- [ ] `trybuild` compile-fail tests with readable error messages
- [ ] Proof-of-concept blueprint exercising routes, DB, and UI end to end
- [ ] Reconcile the spec example's `query_as!` usage with real SQLx syntax

## Phase 3 — UI Shell (`tpt-home-ui`, Leptos)
- [ ] Set up cargo-leptos (SSR + WASM hydration)
- [ ] App shell layout: sidebar, header, theming (light/dark)
- [ ] Login page and auth guard
- [ ] Empty dashboard
- [ ] Dynamic sidebar built from active blueprints and plugins
- [ ] Settings page (profile, password)
- [ ] Admin pages: users and roles, installed blueprints/plugins
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
- [ ] Schema and migrations (notes, tags)
- [ ] CRUD API
- [ ] Markdown rendering (sanitized)
- [ ] FTS5 full-text search with triggers
- [ ] Leptos UI: list, editor, preview, search
- [ ] Tests
### `tpt-home-bp-drop`
- [ ] Schema (drops, expiry, access tokens)
- [ ] Streaming upload/download (large files, constant memory)
- [ ] Share links with optional password and expiry
- [ ] Background cleanup task (cron-style) for expired files
- [ ] Upload size/quota limits
- [ ] Leptos UI
- [ ] Tests
### `tpt-home-bp-track`
- [ ] Relational data model with dynamic user-defined tables/fields
- [ ] Safe dynamic SQL (identifier validation, parameterization)
- [ ] CRUD API and filters
- [ ] Leptos UI (table views, forms)
- [ ] Tests

## Phase 6 — CLI & Developer Experience (`tpt-home-cli`)
- [ ] Clap-based `tpt-home` binary
- [ ] `tpt-home init` (create data dir, config, DB, admin user)
- [ ] `tpt-home migrate`
- [ ] `tpt-home run`
- [ ] `tpt-home new <name>` scaffold (Cargo.toml, lib.rs using macros, workspace registration)
- [ ] `tpt-home new --plugin <name>` scaffold for WASM plugins
- [ ] `tpt-home plugin build|install|list|enable|disable|remove`
- [ ] `tpt-home user add|list|remove|reset-password`
- [ ] `tpt-home backup` / `restore` (SQLite + uploads)
- [ ] Shell completions
- [ ] `scripts/install.sh` systemd installer (user, unit file, enable/start)
- [ ] Windows install script/service support
- [ ] CLI integration tests

## Phase 7 — Deployment & Platform Support
- [ ] `docker-compose.yml` (Caddy + Cloudflared only)
- [ ] `Caddyfile` with auto-HTTPS and reverse proxy
- [ ] Cloudflare Tunnel configuration docs
- [ ] Cross-compilation for Linux ARM64
- [ ] Windows build and run verification
- [ ] Release profile tuning (LTO, strip, opt-level, panic=abort)
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
- [ ] README (what, why, quickstart, license)
- [ ] "Write your first blueprint in 5 minutes"
- [ ] "Write your first plugin" guide
- [ ] Plugin manifest and host API reference
- [ ] Architecture overview
- [ ] Deployment guide (systemd, Docker/Caddy, Cloudflare Tunnel, Windows)
- [ ] Admin and user guide
- [ ] Rustdoc for all public crates
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
