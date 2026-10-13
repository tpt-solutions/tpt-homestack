# Versioning and releases

## Version scheme

The project uses [Semantic Versioning](https://semver.org/). The version lives
once, in `[workspace.package]` in the root `Cargo.toml`, and every crate
inherits it.

While the major version is `0`, the project is pre-stable:

- A **minor** bump (`0.1.0` → `0.2.0`) may contain breaking changes. Each one
  is listed under **Changed** or **Removed** in the changelog, with the
  migration step.
- A **patch** bump (`0.1.0` → `0.1.1`) contains only fixes and security
  changes, with no behaviour a user has to adapt to.

From `1.0.0` the usual rules apply: breaking changes only in a major bump.

## What counts as a breaking change

These are public contracts. Changing one is breaking, so it needs a changelog
entry and, for `0.x`, a minor bump:

- **Plugin manifest** (`plugin.toml`): existing keys, their meaning, and the
  allowed values of `runtime` and page `kind`.
- **Plugin WASM ABI.** ABI version 1 is frozen (`docs/PLUGIN_ABI.md`). A new
  host import is a new ABI version, not a change to version 1.
- **Plugin database rules:** the `<id>_*` table prefix and the scoped-SQL
  restriction.
- **HTTP API paths and JSON shapes** under `/api/`.
- **Config keys** in `config.toml` and the `TPT_HOME_*` environment variables.
- **Migrations.** They are forward-only. An applied migration is never edited,
  because the server refuses an edited one (`docs/PLUGINS.md`). Core and
  blueprint migrations follow the same rule.

## Release checklist

Work through this list in order. Stop at the first failure.

1. **Scope.** Every entry for the release is in `CHANGELOG.md` under
   `[Unreleased]`, and each breaking change names its migration step.
2. **Version.** Set `version` in `[workspace.package]` in `Cargo.toml` to the
   new version. Move the `[Unreleased]` entries under a new heading with the
   version and today's date. Leave an empty `[Unreleased]` above it.
3. **Tests.** Run:
   ```sh
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo test -p tpt-home-plugins --features js
   ```
   The JavaScript plugin tests are behind the `js` feature, so the plain
   workspace run does not include them.
4. **Dependencies.** `cargo deny check` and `cargo audit` both pass. Any new
   ignore has a reason, in `deny.toml` or `.cargo/audit.toml`.
5. **Security.** Re-read `docs/SECURITY_REVIEW.md`. If a security fix ships,
   its changelog entry is under **Security**.
6. **Smoke run.** On Windows and Linux, `tpt-home init`, `run`, log in, and
   open one blueprint page and one plugin page.
7. **Commit and tag.** Commit the version and changelog, then tag with the
   `v` prefix, for example `git tag v0.1.0`. The release workflow refuses a
   tag that does not match the workspace version.
8. **Push and verify.** Push the tag. Check that all three builds finish and
   that the release has one archive and one `.sha256` per target. Download one
   archive and check its checksum.
9. **Publish the notes.** Paste the version's changelog section into the
   GitHub release.

## Hotfixes

A security fix on a released version is made on `master`, then released as a
patch version. The changelog entry comes first. A hotfix does not include
unrelated changes.

## Crates.io

Crates are not published yet (`todo.md`, Phase 10). Before the first
publish, the crate names need reserving, and the release checklist needs a
publish step. Until then, the release is the GitHub binary archives.
