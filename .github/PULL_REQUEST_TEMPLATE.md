# Pull Request

## Summary

What does this change and why?

## Checklist

- [ ] `cargo fmt --all` applied
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New/changed dependencies respect the license policy
      (no Apache-2.0-only crates; see CONTRIBUTING.md) and `cargo deny check` passes
- [ ] New source files carry the SPDX header
      (`// SPDX-License-Identifier: MIT OR Apache-2.0`)
- [ ] Blueprint routes use `/api/<id>/` prefixes; DB tables use the blueprint id prefix
- [ ] Docs updated (README / guides) when behavior or interfaces change

## Performance impact

Does this affect the lean-runtime targets (idle RAM < 30 MB, startup < 0.5 s,
binary < 25 MB)? If yes, include measurements before/after.
