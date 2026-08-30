#!/usr/bin/env bash
# Everything that has to pass before a commit. Fails loudly rather than
# printing a number somebody has to remember to read.
#
# The repeat count is not caution. The suite watches real directories through
# the platform's own notifier, and event timing changes with load — two tests
# passed alone and failed in a loaded run on 2026-08-30. A watcher suite that
# passed once has not been tested.
set -euo pipefail
RUNS="${1:-5}"

echo "== fmt =="
cargo fmt --check

echo "== clippy =="
cargo clippy --all-targets -- -D warnings

echo "== tests, $RUNS runs =="
for i in $(seq "$RUNS"); do
    printf 'run %s: ' "$i"
    cargo test --quiet 2>&1 | grep -E "^test result: (ok|FAILED)" | head -1
done
