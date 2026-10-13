# Security review

Status: **internal code review, 2026-10-13. Not an independent penetration test.**
The gate in `todo.md` (Phase 8) stays open. Remote access (Tailscale or a
tunnel) must not be promoted to ordinary users until the open items below are
closed and an outside review has been done.

## Scope and method

Read the authentication, session, CSRF, rate-limit, error, upload, file-path,
SQL and HTML-rendering code, and the tests that cover them. Checked the
plugin host boundary through its existing sandbox tests. Not covered: a
running penetration test, fuzzing (not built), ARM64 hardware, and the
browser-side behaviour of every page.

## Fixed in this review

| # | Area | Finding | Fix |
|---|------|---------|-----|
| 1 | Login | `/api/auth/login` shared the 240-per-minute bucket with the whole API, so a password-guessing run was 12 times faster than it needed to be. | Login and setup have their own bucket: 20 attempts per client per minute (`LOGIN_MAX_DEFAULT`). Tested in `crates/tpt-home-api/tests/integration.rs` and `middleware.rs`. |
| 2 | Errors | Internal errors returned their message to the client, so SQL text, file paths and similar detail reached the browser. | Internal errors return `internal server error`; the detail is logged. Tested in `crates/tpt-home-core/src/error.rs`. |
| 3 | Login timing | For an unknown username, login skipped the Argon2 check (it verified against an unparseable string and returned at once). A wrong username answered faster than a wrong password, which revealed which accounts exist. | An unknown user now runs a full Argon2 verification against a fixed dummy hash (`verify_dummy`). The cost is the same by construction; it was not timing-measured. |
| 4 | Passwords | No length limit, so Argon2 could be run on very large inputs. | Passwords over 1024 characters are refused when set; verification rejects inputs over 4096 bytes before running Argon2. Tested in `password.rs`. |
| 5 | CSRF | Multipart uploads passed the content-type check, so a cross-site form could send one, relying only on `SameSite=Lax`. | Multipart requests must send `X-TPT-Request` (any value). A form cannot set a custom header, so the cross-site route is closed even if the cookie setting is weakened. The drop and file-inbox pages and the `curl` examples send it. Tested in `tpt-home-api/tests/integration.rs` and the drop e2e test. |
| 6 | Accounts | Login was limited per client only, so guessing one account from many addresses was not slowed. | Ten failures in a row lock a username for 15 minutes, for every client, including names with no account. A success clears the count. The lock is checked before the password, so a locked name costs no hashing. In memory, so it resets on restart. Tested in `auth/lockout.rs` and the integration tests. |
| 8 | Script policy | The CSP allowed inline scripts and handlers (`script-src 'unsafe-inline'`), so a stored XSS that slipped past escaping would run. | Each HTML response gets a fresh nonce; its `<script>` tags carry it and the policy allows only those. Inline event handlers were replaced by one delegated dispatcher (`data-click`, `data-submit`, `data-change`), so pages no longer need inline handlers. Checked in a real browser by `scripts/browser-check.mjs` (five workflows, seven pages, no CSP violations) and by `html_pages_allow_only_nonced_scripts`. |
| 9 | Settings | The password change form never worked: its script was a format-string literal with doubled braces, and it called a helper the page did not define. The browser check found it. | Rewritten as plain JavaScript; the server's message is shown as text. Covered by `settings_script_is_plain_javascript` and the settings browser workflow. |
| 7 | Plugin host | Hostile input to the manifest parser, the SQL scope check and the package reader had no robustness tests. | `crates/tpt-home-plugins/tests/boundary.rs` runs 4,000 mutated manifests, 6,000 random SQL statements, 300 damaged packages and 300 random byte blobs through them with a fixed seed. Nothing panics, nothing is written outside the destination, and accepted SQL only uses scoped tables. This is deterministic testing, not fuzzing. |

## Checked, no issue found

- **SQL injection.** Plugin metrics must be a single scoped SELECT on the
  plugin's own tables, checked at install. Plugin JS quotes values by hand and
  cannot use bound parameters, so the tests store quote-injection strings
  literally. Track's dynamic SQL uses integer identifiers and bound values
  only.
- **Path traversal.** Upload names are a generated id plus a sanitised name;
  paths are built with `safe_join` and checked against the canonical upload
  directory before any read or delete.
- **HTML injection (XSS).** Server-rendered pages escape interpolated values.
  The markdown renderer escapes HTML before applying markdown and only emits
  safe link schemes. The core `innerHTML` templates escape the values they
  interpolate. Plugin pages are rendered by the host with escaping, and the
  tests cover it.
- **Uploads.** Downloads are sent as `attachment` with `nosniff` and the CSP
  set, so an uploaded HTML file is not rendered in the app's origin. Size is
  enforced while streaming.
- **Sessions.** Cookies are `HttpOnly`, `SameSite=Lax`, and `Secure` when
  configured. Session tokens are stored as SHA-256 hashes. Changing a password
  revokes existing sessions. Setup works once only.
- **CSRF.** Mutating `/api/` requests must declare JSON, multipart or CSV
  bodies. Those cannot be sent cross-site without a preflight. Multipart
  also needs the `X-TPT-Request` header (fix 5), which a form cannot set.
- **Dependencies.** `cargo deny check` passes. `cargo audit` has one
  advisory (`rsa`), which is not compiled into any target; see
  `.cargo/audit.toml`.

## Open: residual risks

These must be closed, or explicitly accepted, before the gate opens.

1. **Inline styles are still allowed; inline scripts are not.** Scripts now
   need a per-response nonce, and inline event handlers are refused (fix 8).
   Styles keep `'unsafe-inline'` because pages use `style=` attributes. An
   injected style cannot run code, so the risk is lower, but it is still open.
2. **Cookie setting still matters.** Multipart uploads now also need the
   `X-TPT-Request` header (fix 5), so a cross-site form cannot forge them. The
   session cookie is still `SameSite=Lax` and should stay that way; it is the
   first line of defence for every other route.
3. **Plugin hook secrets are stored in plain text.** HMAC verification needs
   the secret, so it cannot be hashed. Anyone who can read the database file
   can read them.
4. **Replay within the window.** Webhook and plugin-hook signatures can be
   replayed for up to 5 minutes. Accepted in `todo.md` for home sensors;
   revisit if webhooks carry commands.
5. **Rate limits are per process and in memory.** After a restart they reset.
   Behind a proxy, `trust_proxy` must be set, or every visitor shares one
   bucket. Setting it wrongly lets a client spoof its address; see
   `docs/DEPLOYMENT.md`. The table of tracked clients is pruned once per
   window. Its size is the number of distinct addresses seen in about two
   windows, so an attacker with many addresses can grow it. A soak with 62,000
   rotating addresses added about 9 MB in its first 76 seconds. Whether it
   levels off over a longer run is what the soak in `docs/BENCHMARKS.md` checks.
   A cap on the table is not built.
6. **Lockout is in memory and per name.** A guesser who spreads attempts over
   many names is limited only by the per-client bucket. Restarting the server
   clears the counts. Fix: persist the counts, or use a longer window.
7. **The host boundary is tested deterministically, not fuzzed.** Coverage
   is the fixed inputs in `boundary.rs`. Coverage-guided fuzzing of the
   manifest, SQL and package parsers is still to do.
8. **Notification channel URLs are plain text.** They are never listed by the
   API, but they are readable from the database file, like hook secrets.
9. **JavaScript memory: contained in the CLI build, not yet verified on
   Linux.** A script could build 256 MB in 28 loop steps and exhaust the
   server's memory. JavaScript calls now run in a worker process under an
   operating-system memory limit, so the call fails and the server keeps
   running (test `a_script_that_exhausts_memory_fails_alone`). On Windows this
   is tested. On Linux the `RLIMIT_DATA` path type-checks but has not run.
   Library embedders that skip the worker still run scripts uncapped. See
   [PLUGIN_SANDBOX_REVIEW.md](PLUGIN_SANDBOX_REVIEW.md).
10. **No independent review yet.** This document is a code review by the
   project. An outside review is needed before the gate opens.

## Re-running the checks

```sh
cargo test --workspace
cargo test -p tpt-home-plugins --features js
cargo deny check
cargo audit

# With a server running (tpt-home run --port 8080 and an admin account):
node scripts/browser-check.mjs --base http://127.0.0.1:8080 --password '...'
```

`scripts/browser-check.mjs` drives headless Chrome or Edge over the DevTools
protocol. It loads each page, reports console errors and CSP violations, and
runs the notes, settings, track, drop and users workflows by clicking the real
controls. It fails on any violation or failed step.
