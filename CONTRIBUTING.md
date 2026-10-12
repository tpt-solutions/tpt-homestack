# Contributing to tpt-homestack

Thanks for helping build a lean, single-binary home server platform.

## How to contribute

Contributions are accepted as **issues only**. External pull requests will not
be accepted. Use the issue templates to report bugs, request features, or
propose changes, and describe the change in the issue instead of opening a PR.

## Getting started

```sh
git clone <repo> && cd tpt-homestack
cargo build              # debug build
cargo test               # unit + integration tests
cargo run -p tpt-home-cli -- run   # start the server on http://127.0.0.1:8080
```

## Licensing rules (enforced by `cargo deny`)

- This project is dual-licensed `MIT OR Apache-2.0` (copyright TPT Solutions).
  Every source file carries the SPDX header:
  `// SPDX-License-Identifier: MIT OR Apache-2.0`.
- **Allowed** dependency licenses: MIT, Apache-2.0 (and duals of the two),
  BSD-2/3-Clause, ISC, Unicode-3.0, Zlib, CC0-1.0, BSL-1.0.
- **Denied**: GPL, AGPL, LGPL, MPL, SSPL, EUPL.
- **Project rule — no Apache-2.0-only dependencies.** The license checker
  tolerates Apache-2.0 (many crates are `Apache-2.0 OR MIT` and pass), but
  proposals that introduce a dependency that is *only* available under
  Apache-2.0 (e.g. `wasmtime`) will be rejected. Prefer `MIT OR Apache-2.0`
  crates. Justify any exception in the issue.

## Code conventions

- Crate naming: everything user-facing in the ecosystem is prefixed
  `tpt-home-*`; community blueprints are `tpt-home-bp-<name>`.
- Blueprints are compiled-in modules registered via the `tpt-home-macros`
  attribute macros (`#[blueprint]`, `#[bp_migration]`, `#[bp_route]`,
  `#[bp_ui_route]`). Never hand-wire routes into `tpt-home-api`.
- Blueprint API paths must start with `/api/<id>/`; blueprint DB tables must
  be prefixed with the blueprint id.
- Keep the performance targets in mind: idle RAM < 30 MB, startup < 0.5 s,
  binary < 25 MB. Avoid heavyweight dependencies.

## Checks

Run these before any change is merged into `master`:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check        # if you touched dependencies
```

## Reporting issues

Use the issue templates. Security issues: see `SECURITY.md` — do not open
public issues for vulnerabilities.
