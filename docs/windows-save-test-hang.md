# The Windows save-test hang

A place to pick this up from a Windows machine. It is not resolved. What is
known is written down so the next session does not start from the two CI runs
that first showed it.

## The symptom

On `windows-latest`, `cargo test --quiet` sometimes never returns. The suite
reaches the last of its tests and then one of them blocks. libtest prints:

    flow::tests::a_save_reaches_the_container_without_anybody_closing_the_session
    has been running for over 60 seconds

and nothing after it. The job sits until GitHub's own timeout or a cancel. The
`Test, three times` step in `.github/workflows/windows.yml` now kills a run that
wedges past three minutes and retries a bounded number of times, so a wedge no
longer costs a six-hour runner; a wedge shows as a `::warning::` in that step.

Measured 2026-09-07 on branch `macos`: two consecutive CI runs wedged here, on
two different fresh runner VMs, at the same test. Build and clippy were green
both times; only the test step hung.

## Where it is, and where it is not

The test saves the way a real editor does — writes a temporary sibling and
renames it over the payload — then loops under a ten-second deadline calling
`Opened::wait_and_pump(250ms)` until the change is seen. The deadline is checked
only between calls, so it cannot bound a single call that never returns. The
hang is therefore inside one `wait_and_pump`.

Reading the code in that path (`src/flow.rs`, `src/writeback.rs`,
`src/recover.rs`):

- `wait_and_pump` calls `Watch::next_change(within)`, which is
  `mpsc::Receiver::recv_timeout(within)` — bounded by construction — then
  `pump_including`.
- `pump_including` calls `Watch::drain()`, which is `Receiver::try_iter()`. Its
  `for` loop is the one construct in the path with no time bound: it runs while
  the channel has a message ready, so a watcher that floods the channel would
  spin it forever.
- `save_if_changed` calls `recover::state` (reads the payload) and
  `writeback::write_back` (reads the payload, replaces the container). Both are
  bounded file operations; `writeback.rs` has no retry or spin loop, only the
  bounded loops inside its own tests.

So the two suspects are the receive not returning and the drain flooding, both
in the `notify` v8 `ReadDirectoryChangesW` backend, not in this crate's save
logic. The container written by `write_back` lives beside the source container,
not in the watched payload directory, so the write-back does not feed the watch.

## Why it has not been pinned down

It is a Heisenbug. Env-gated `[diag]` probes were added around each phase of the
test and around the two calls inside `wait_and_pump`, then the exact failing
command was run fifty times across two diagnostic workflows. It did not hang
once. The probes — a handful of microseconds of work, or a file write, between
the rename and the pump — move the timing enough to hide the race. That is
itself the finding: a deterministic loop in our code would reproduce regardless
of instrumentation; a timing race in the watcher layer does not.

The probes and the diagnostic workflow were reverted in the same branch. This
file is what they left behind.

## How to chase it on a Windows machine

The point of a real machine is a debugger that can dump the wedged thread's
stack live, which is the one instrument the probes cannot be, because they
suppress the bug.

1. **Reproduce first, with no instrumentation.** In the checkout, loop the exact
   command until one wedges. PowerShell:

   ```powershell
   for ($i = 1; $i -le 200; $i++) {
     Write-Host "run $i"
     $p = Start-Process cargo -ArgumentList @('test','--quiet') -NoNewWindow -PassThru
     if (-not $p.WaitForExit(120000)) {
       Write-Host "run $i WEDGED — leave it running and attach a debugger to the test process"
       break
     }
     if ($p.ExitCode -ne 0) { Write-Host "run $i failed (not a wedge)"; break }
   }
   ```

   The wedged process is the test binary under
   `target\debug\deps\slipcase_open-*.exe`, still alive because it was not
   killed.

2. **Attach and dump stacks.** With Visual Studio or WinDbg, attach to that
   `slipcase_open-*.exe` and dump all thread stacks (`~*k` in WinDbg). The test
   runs on one thread and the `notify` watcher on another. What to read:
   - Is the test thread parked in `recv_timeout` past its 250ms? That points at
     the channel or the condvar, not the drain.
   - Is it spinning in `try_iter`/`drain`? That is the flood: the watcher thread
     is enqueueing without end. Then look at the watcher thread and at what
     `ReadDirectoryChangesW` is reporting.
   - Is it inside `write_back`, i.e. a file operation (`ReplaceFileW`, a read of
     the payload) blocked? That would move the suspect to the filesystem, and
     antivirus on the machine is the first thing to rule out.

3. **If a debugger is not to hand, catch the phase with file probes.** Re-apply
   the patch below, set `SLPC_DIAG_FILE` to a path, and run the loop above with
   a per-run timeout; on a wedge, read the file — its last line is the phase.
   The probes may hide the bug (they did in CI), so try several times, and
   prefer the debugger.

## The probe patch, to re-apply

All in `src/flow.rs`. A free function near the top of the module:

```rust
#[doc(hidden)]
pub fn diag(msg: &str) {
    if let Some(path) = std::env::var_os("SLPC_DIAG_FILE") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[diag] {msg}");
        }
    } else if std::env::var_os("SLPC_DIAG").is_some() {
        eprintln!("[diag] {msg}");
    }
}
```

In `wait_and_pump`, a line before `next_change`, one printing its result, and
one after `pump_including`. In `pump_including`, a line before the drain, one
after it printing the count and `payload_changed`, and lines bracketing
`save_if_changed`. In the test, `super::diag` calls before and after `open`,
after the rename, and inside the loop bracketing each `wait_and_pump` with an
iteration counter. Put `diag` above `open`'s doc comment, not between the doc
comment and `open`, or clippy's `missing_errors_doc` fires on `open`.

## Suspects to read, in order

1. `notify` v8's `ReadDirectoryChangesW` backend, for a path where the watcher
   thread either stops delivering (receive stalls) or delivers without end
   (drain floods). This is the leading suspect.
2. `Watch::drain` in `src/watch.rs` bounding its `try_iter` loop — a defensive
   fix that would cap a flood regardless of the backend, worth doing only once a
   flood is the measured cause.
3. The filesystem under `write_back` on this machine, ruled in or out by whether
   the wedged stack is in a file call.

Do not change `wait_and_pump`, `drain`, or the watcher on a guess. `CLAUDE.md`:
revert the fix and watch a test fail before writing down why it broke. Here that
means reproduce, attach, and read the stack first.
