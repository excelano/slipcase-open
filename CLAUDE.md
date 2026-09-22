# CLAUDE.md

A companion to the `slipcase` CLI and the `slipcase-desktop` viewer. Double-click a
`.slpc`, the content file opens in whatever application normally handles it, and edits made
there are written back into the container. No flyleaf UI, no preview, no container
browsing. The engine is `src/lib.rs` and its modules, with no dependency on how the tool
presents itself; `src/main.rs` is the command line over it, which concept §9 keeps as the
floor beneath the notifications and the tray. Every security-relevant decision —
validation, policy, the launch path, the write-back — lives in the engine, once, for all
three platforms. `slipcase-open-concept.md` is the reasoning and `PLAN.md` the order of
work.

## Before you commit

    ./check.sh        # fmt --check, clippy -D warnings, and the suite five times

**The verdict comes from the tool, never from its output.** Take the status from the
command: a pipeline through `grep` and `head` reports the first test binary's result and
exits zero while the second is failing. A gate that fails is a gate; a line that looks
reassuring is not.

**Five runs rather than one, and this is not caution.** The suite watches real directories
through the platform's notifier, and event timing moves with load. A watcher test that
passed on its first run has not been tested.

Releases: run `ship slipcase/slipcase-open`. There is no release document.

## Rules

**Assert what the code guarantees, not what it happened to do.** `pump` decides from the
bytes, whether the content file differs from what the container holds, and not from how many
events arrived, because the event count is a function of how busy the machine is. A test
that counts events, or reads state a pump may or may not have reached yet, is measuring
the machine.

**A causal claim in a comment or a commit message is a finding and needs the same evidence
as one.** Before writing down *why* something broke, revert the fix and watch a test fail;
if it does not fail, say what was measured and stop there. An unproven claim in a doc
comment is worse than no comment, because the next reader cannot tell it apart from a
proven one.

**The catalogue is declared in the engine, not in `main`.** `resident` composes every
report and every question and hands them to a `Channel` that only carries them, so concept
§9's boundary holds for language too. What is translated is what the tool says about your
files — a notification, a question, the button on one, a line of `sessions`, the clause
`recover::State` renders. What is not is what it says about itself: `--help` is clap's, and
the `policy` report prints paths in a fixed-width table built to the width of English, read
beside the documentation and pasted into bug reports. **`Choice::key` is never translated
and `Choice::label` always is**: the key is what the notification service is handed and
echoes back, and `from_key` matches on it, so a German key is a button whose press this
build cannot recognise. `po/update-po.sh` is the only way the catalogues move, and
`po/pseudo.sh` with `POTEXT_LANG=en-x-pseudo` is what finds a sentence that never went
through `t`.
