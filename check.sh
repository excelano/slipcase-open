#!/bin/sh
# The gate. Run it before every commit.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
#
# Five runs rather than one, and that is not caution. The suite watches real
# directories through the platform's notifier, and event timing moves with load.
# A watcher test that passed on its first run has not been tested.
set -eu

RUNS="${1:-5}"

echo "== fmt =="
cargo fmt --check

echo "== clippy =="
cargo clippy --all-targets -- -D warnings

echo "== tests, $RUNS runs =="
i=1
while [ "$i" -le "$RUNS" ]; do
    # The status is taken from cargo rather than read out of its output, and
    # this is the second time that distinction has cost something. A commit went
    # out with a clippy warning because the count was printed and not read; then
    # `cargo test | grep | head -1` reported the first test binary's result and
    # exited zero while an integration test in the second was failing. A gate
    # that fails is a gate; a line that looks reassuring is not.
    if ! said=$(cargo test --quiet 2>&1); then
        printf '%s\n' "$said"
        echo "run $i: FAILED"
        exit 1
    fi
    printf 'run %s:\n' "$i"
    # Every binary's verdict, less the empty ones: the doc tests and the
    # binary's own have nothing in them and say so four times a run.
    printf '%s\n' "$said" | sed -n 's/^test result: /  /p' | grep -v '^  ok\. 0 passed' 
    i=$((i + 1))
done
