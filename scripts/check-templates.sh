#!/usr/bin/env bash
# Scaffold every starter template and check that each generated blueprint
# compiles and links into the CLI. Runs in a temporary copy of the working
# tree, so the repository itself is never modified.
#
# Usage: scripts/check-templates.sh
# Set CARGO_TARGET_DIR to reuse an existing build cache.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

(cd "$root" && tar --exclude=./target --exclude=./.git --exclude=./data -cf - .) | tar -xf - -C "$work"
cd "$work"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

templates="basic records logbook file-inbox scheduled-job dashboard scheduled-fetch"
for kind in $templates; do
  name="check_${kind//-/_}"
  echo "== $kind"
  cargo run -q -p tpt-home-cli -- new "$name" --template "$kind" >/dev/null
  cargo check -q -p "tpt-home-bp-$name"
done

# The CLI links every generated blueprint, so this proves the registration
# path works for all of them, not just that each crate compiles on its own.
cargo build -q -p tpt-home-cli
echo "all $(echo $templates | wc -w) templates build and link"
