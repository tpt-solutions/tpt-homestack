# Changelog

All notable changes to tpt-homestack are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
follows the versioning policy in [docs/RELEASING.md](docs/RELEASING.md).

## [Unreleased]

No release has been tagged yet. The first tag will be `v0.1.0`.

### Added

- Plugin runtimes: JavaScript scripts (Boa, behind the `js` feature), external
  processes with scoped API tokens, and an MCP server (stdio and HTTP).
- Plugin capabilities: scheduled jobs (`[[jobs]]`), signed public endpoints
  (`[[hooks]]`), outbound HTTP for scripts (`http` plus `http_hosts`), and
  admin-configured notification channels (`notify`).
- Declarative `summary` pages, with totals and grouped cards (`grouped = true`,
  one row per group, up to 50 rows).
- Official blueprint scheduling: `#[bp_job]` with per-job timeouts and an
  admin status page.
- Webhook receiver with HMAC-SHA256 signatures and admin management.
- Track CSV import and export (RFC 4180, all-or-nothing import).
- Scaffolding templates: `basic`, `records`, `logbook`, `file-inbox`,
  `scheduled-job`, `dashboard`, `scheduled-fetch`, `sensor-alerts`.
- Plugin ports of `records`, `sensor-alerts` and `logbook`.
- `tpt-home migrate`, `tpt-home new` (with its error paths tested).
- `trust_proxy` setting for deployments behind a reverse proxy.
- Release workflow for Linux x86_64, Linux ARM64 and Windows (not yet run).
- Browser checks (`scripts/browser-check.mjs`) in CI: every page for console
  errors and CSP violations, six workflows, phone width, labels and touch
  sizes, and a Tab-key focus check. The CI job has not run on GitHub yet.
- Tests for the JavaScript script worker that run real worker processes:
  results, script errors, loop and deadline limits, the memory limit, and a
  missing or bad worker.

### Fixed

- The settings page's password change never worked. Its script was broken
  (doubled braces in a plain string, and a call to a helper the page did not
  define). Found by the new browser check.

### Changed

- Backups are written to one folder per backup, `<out>/<time>/`, with the
  database and its uploads inside. Earlier, a second backup overwrote the
  first backup's uploads.
- A plugin install that is refused for its manifest or package now returns
  400 with the reason, not 500.
- Internal server errors return a generic message to clients. The detail is
  logged only.
- Quiet hours and alert times use the server's local time zone.
- Deletes and plugin installs ask with the app's own dialog, not the browser's
  `confirm()`. Cancel and Escape keep the item.
- Text on the message, badge and sidebar colours meets WCAG AA (4.5:1) in both
  light and dark themes. The dark theme's error and success messages were
  hard to read.
- Keyboard users get a visible focus ring on every control.
- On phone widths, buttons, inputs and selects are at least 44 px tall. Wide
  tables scroll inside their card instead of widening the page.
- Settings, the plugin form and the Notes, Track and Drop pages build their
  controls with the shared components.

### Security

- Pages allow only scripts that carry a per-response nonce, and inline event
  handlers are refused. Pages use one delegated dispatcher instead. Inline
  styles are still allowed.
- JavaScript plugin calls run in a worker process with an operating-system
  memory limit (`max_memory_mb` plus 16 MB). A script that exhausts memory
  fails its own call; the server keeps running. Previously a script could
  build hundreds of megabytes in the server process. Each call now pays about
  16 ms of process start-up. The Linux memory path is not yet run in CI.
- The rate limiter's stale-client pruning runs once per window, not on every
  request, so a request no longer costs time in proportion to the number of
  clients seen. Found by a load test with rotating addresses.
- Login and setup are limited to 20 attempts per client per minute, separate
  from the general API limit.
- A username locks for 15 minutes after 10 failed logins, for every client.
- Login timing no longer reveals whether a username exists.
- Passwords are limited to 1024 characters.
- Internal error messages (SQL text, paths) are no longer sent to clients.
- Multipart uploads must send the `X-TPT-Request` header. Browsers cannot send
  it cross-site, so a form on another site cannot forge an upload. API clients
  using `curl -F` must add `-H "X-TPT-Request: 1"`.

See [docs/SECURITY_REVIEW.md](docs/SECURITY_REVIEW.md) for the open items. The
remote-access gate is still closed.

### Known gaps

- The CSP allows inline scripts; the Leptos UI and asset pipeline are not built.
- No Leptos UI, installers, appliance image or ARM64 hardware testing yet.
