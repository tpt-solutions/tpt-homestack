#!/usr/bin/env bash
# Fails when the release server binary is larger than the size target.
#
# The target is 30 MiB (31,457,280 bytes). It is set in docs/BENCHMARKS.md, which
# records why. Run it from the repository root:
#
#   bash scripts/check-size.sh
#
# CI runs it on Linux; the figures in docs/BENCHMARKS.md are from Windows, so
# the two are not expected to match exactly.

set -euo pipefail

LIMIT_BYTES=$((30 * 1024 * 1024))

cargo build --release -p tpt-home-cli

bin=target/release/tpt-home
[ -f "$bin" ] || bin=target/release/tpt-home.exe
size=$(stat -c%s "$bin")

echo "release binary: $size bytes ($((size / 1048576)) MiB); limit $LIMIT_BYTES bytes (30 MiB)"
if [ "$size" -gt "$LIMIT_BYTES" ]; then
  echo "the binary is over the size target; see docs/BENCHMARKS.md before raising it"
  exit 1
fi
