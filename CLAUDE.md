# CLAUDE.md

Guidance for Claude Code working in `slipcase-open`. Short, because the
reasoning lives in `slipcase-open-concept.md` and the order of work lives in
`PLAN.md`. Read both before changing anything.

---

## What this is

A companion to the `slipcase` CLI and the `slipcase-desktop` viewer. Double-click
a `.slpc`, the payload opens in whatever application normally handles it, and
edits made there are written back into the container. No metadata UI, no
preview, no container browsing.

The engine is `src/lib.rs` and its modules, with no dependency on how the tool
presents itself. `src/main.rs` is the command line over it, which concept §9
keeps as the floor beneath the notifications and the tray. Every
security-relevant decision — validation, policy, the launch path, the write-back
— lives in the engine, once, for all three platforms.

## Before you commit

Run `./check.sh`. It is `fmt --check`, `clippy -D warnings`, and the suite five
times.

**The verdict comes from the tool, never from its output.** Twice now the gate
has looked green while something was wrong. A commit went out on 2026-08-30 with
a clippy warning outstanding because the count was printed and not read. Then on
2026-08-31 `cargo test | grep | head -1` reported the first test binary's result
and exited zero while an integration test in the second was failing — measured
by breaking one on purpose and watching `check.sh` print `ok` and exit 0. Both
times the fix was to take the status from the command. A gate that fails is a
gate; a line that looks reassuring is not.

**Five runs rather than one, and this is not caution.** The suite watches real
directories through the platform's notifier, and event timing moves with load.
Two tests passed alone and failed in a loaded run that day: one asserted an
exact repack count that the code never guaranteed, and one asserted a recovery
state that depended on whether a pump had landed yet. A watcher test that passed
on its first run has not been tested.

## Two rules this repository learned the hard way

**Assert what the code guarantees, not what it happened to do.** `pump` decides
from the bytes — whether the payload differs from what the container holds — and
not from how many events arrived, because the event count is a function of how
busy the machine is. Any test that counts events, or reads state that a pump may
or may not have reached yet, is measuring the machine.

**A causal claim in a comment or a commit message is a finding, and needs the
same evidence as one.** Three separate explanations written here on 2026-08-30
were wrong while the fixes they accompanied were right: a repack loop attributed
to the write-back retriggering itself when it came from an external reader, and
a swallowed-save defect that was never demonstrated. The rule that would have
caught all of them: before writing down *why* something broke, revert the fix
and watch a test fail. If it does not fail, say what was measured and stop
there. An unproven claim in a doc comment is worse than no comment, because the
next reader has no way to tell it apart from a proven one.

## Where things sit

`PLAN.md` has the phases and what is done. `slpc` comes from crates.io as of
0.3.11, which carries `payload_crc`; it was a path dependency on the sibling
checkout until that was published. `testsupport` is still a git dependency on
`slpc-rust`, being `publish = false`.
