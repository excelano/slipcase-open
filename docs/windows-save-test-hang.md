# The Windows save-test hang

A place to pick this up from a Windows machine. It is not fixed, but as of
2026-09-07 it is no longer unlocated: it was reproduced on a Windows desk and
the stack was read. Start at "What the stack said". The two sections after it
are kept because they were the working theory, and the theory was wrong.

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

## What the stack said, 2026-09-07, and what it falsifies

Reproduced on a Windows desk on run 27 of 200, and read with a debugger. The
hang is in **test teardown**, not in the save path. Everything below this
heading is measured off the wedged process; the section after it is kept
because it was the working theory and it was wrong.

The wedged test was `an_edit_is_written_back_once_however_many_times_it_is_asked_for`
(`src/flow.rs:818`), not the test CI named. That test never calls `wait_and_pump`,
never waits on the channel and never drains it. Both facts point the same way:
what wedges is not particular to either test.

Two threads, both parked. `cdb -pv` on the live process, `~*k`:

    1  "flow::tests::an_edit_is_written_back_once_however_many_times..."
       ntdll!NtQueryDirectoryFile
       KERNELBASE!GetFileInformationByHandleEx
       std::sys::fs::windows::remove_dir_all
       tempfile::dir::impl$3::drop
       slipcase_open::flow::tests::an_edit_is_written_back_...  [src/flow.rs @ 834]

    2  "notify-rs windows loop"
       ntdll!NtNotifyChangeDirectoryFileEx
       KERNELBASE!ReadDirectoryChangesW
       notify::windows::start_read                  [notify-8.2.0 windows.rs @ 308]
       notify::windows::handle_event                [notify-8.2.0 windows.rs @ 371]
       ntdll!KiUserApcDispatch
       ntdll!NtWaitForSingleObject
       KERNELBASE!WaitForSingleObjectEx
       notify::windows::stop_watch                  [notify-8.2.0 windows.rs @ 261]
       notify::windows::ReadDirectoryChangesServer::run

`src/flow.rs:834` is the test's closing brace. The body had run to the end and
every assertion had passed; the thread is dropping the `TempDir`.

**Measured, not inferred:**

- The test thread is in `remove_dir_all`, enumerating the temp directory.
- The watcher thread is in `stop_watch`, whose `WaitForSingleObjectEx(sem,
  INFINITE, 1)` is alertable; an APC fired inside that wait, ran `handle_event`,
  and `handle_event` called `start_read`, which re-armed
  `ReadDirectoryChangesW` on the directory being removed. That call has not
  returned.
- No thread is running. CPU across the process was identical to the
  millisecond over 15 seconds, and unchanged again minutes later. Total 1.31s
  for a process that had been wedged for over two minutes.
- `notify`'s own source explains the wait: `handle_event` releases
  `complete_sem` on `ERROR_OPERATION_ABORTED`, on `ERROR_ACCESS_DENIED` with
  the directory gone, and on an unknown error — but on `ERROR_SUCCESS` it
  re-arms instead, and `stop_watch` waits `INFINITE` for a semaphore that
  re-arming never posts. `start_read` does post it when the re-arm *fails*
  (`ret == 0`), so a re-arm that returned would have unwedged this. It did not
  return.

**Inferred, and not yet measured:** why `NtNotifyChangeDirectoryFileEx` does
not return. The shape is a deadly embrace between the two threads over one
directory — the enumeration and the change-notify are both directory-control
work on the same object — but nothing here has measured the kernel's side of
it, and the fix does not depend on knowing.

**What this kills.** The zero CPU rules out the drain flooding, which was the
leading suspect: a spin cannot burn no time. The wedged test never entering
`wait_and_pump` rules out that localisation. Neither `Watch::drain` nor
`recv_timeout` nor `write_back` appears on any stack.

**Why the probes hid it.** They were placed around the save path, which is not
where the hang is; and any work between the last save and the drop moves the
one race that matters — the watch stopping against the directory being
removed.

**What it means beyond the suite, and this is the part to chase next.** The
collision is a watch being stopped while the directory it watches is removed.
That is not only what a test's `TempDir` does at the end of a scope; it is
what `close` does in the product, which removes the session directory and
drops the `Watch` that was watching it. Whether the shipping path can wedge
the same way has *not* been measured. It is the first thing to establish,
because it decides whether this is a flaky suite or a hang on close.

## Measured: `close` wedges too, and the harness that says so

The question the stack raised — whether the shipping `close` can meet the same
collision — is answered. It can. Measured 2026-09-07 on the Windows desk.

**The harness had to be built twice, and the first one was worthless.** A
single-threaded loop of the exact shape that wedges ran **40,000 rounds without
wedging once**, and so did 40,000 closes. That silence meant nothing: with no
positive control, a loop that cannot wedge anything produces the same output
whether or not the defect is reachable. Putting the threads back — the suite
runs its tests across them, the loop did not — reproduced it immediately.

Both diagnostics then ran identically: 20 runs each, 400 rounds spread across
8 threads, killed and counted past a 60-second timeout.

| shape | wedged | clean |
| --- | --- | --- |
| `teardown_alone_under_repetition` (drop, no close) | 9 / 20 | 11 |
| `close_alone_under_repetition`, 8 workers | 1 / 20 | 19 |
| `close_alone_under_repetition`, 8 workers, repeated | 1 / 20 | 19 |
| `close_alone_under_repetition`, 4 workers | 2 / 20 | 18 |
| `close_alone_under_repetition`, 1 worker | **0 / 20** | 20 |
| `close_with_a_neighbouring_watch`, 1 worker + a live watch | **0 / 20** | 20 |

**The counts in that table are not `close`'s wedge rate, and reading them as
one would be wrong.** The close diagnostic drops a `TempDir` every round as
well as calling `close`, so it can wedge either way. All four of its wedges
were captured with a stack, and the stacks do not agree with each other:

- one, the 8-worker run below, is `Opened::close` -> `Session::remove` — the
  product path;
- the other three — both 4-worker wedges and the 8-worker repeat — are
  `tempfile::dir::drop`, the teardown collision already written up.

So what is established about `close` is **that it can wedge, not how often**.
One stack proves the first. Nothing here measures the second, and 1-in-20 is
the diagnostic's rate rather than the defect's.

**What the worker count does establish.** A lone session never wedged: 0/20
single-threaded, 0/20 with a neighbouring watch alive, and 40,000 rounds of an
earlier single-threaded loop. Wedges appear only from four concurrent sessions
upward. Whatever the mechanism, it needs more than one session closing at once
— which is what a resident instance holding several sessions does.

A clean run of either takes about 3 seconds, against the 60-second timeout, so
a wedge is not a slow run.

**The close wedge is the product path, and the stack says so rather than the
count.** The close diagnostic also drops a `TempDir` every round, so the number
alone could not tell the two collisions apart. The stack can:

    std::sys::fs::windows::remove_dir_all
    std::fs::remove_dir_all
    slipcase_open::session::Session::remove   [src/session.rs @ 167]
    slipcase_open::flow::Opened::close        [src/flow.rs @ 435]

`Opened::close` itself, blocked in `remove_dir_all`, with a watcher thread in
the same state as the original capture — `stop_watch`, an APC, `start_read`,
and `ReadDirectoryChangesW` that has not returned.

**What is not established.** With eight workers and several watchers alive,
this capture cannot say whether the watcher colliding with the close is the
closing session's own or a neighbour's. In the first capture it must have been
its own: that process had exactly three threads — main, the test, one notify
loop — so only one watch existed. Here it cannot be told apart, and the
difference matters, because a collision between *different* directories is a
wider problem than a session tripping over itself.

**A reading of the rate difference, offered as inference and not measurement.**
`close` removes the directory while its watch is alive but *not stopping* — the
watch is dropped afterwards. The wedge needs a watcher inside `stop_watch`,
re-arming as it is cancelled. So the closing session's own watch is not in the
dangerous state during its own `remove`, and the collision has to come from a
concurrent one, which is rarer than the teardown's own watch stopping directly
against its own directory. The worker-count runs support the first half of
that: a lone session never wedges, and wedges need four concurrent sessions or
more, which is what "the collision has to come from a concurrent one" predicts.
The second half — that this is why `close` wedges more rarely than a teardown —
is not supported by anything here, because the close diagnostic's counts are
not `close`'s rate.

**What follows.** A hang in `close` is not a flaky suite. Concept 6.2 puts the
close at the user's hand, and this is the hand not coming back. It wants a fix
in the engine rather than in the tests, and the fix has to be measured against
this harness rather than reasoned about: the numbers above are what a fix must
move.

## The isolated close: rare, and a genuine hang

A third diagnostic, `close_alone_leaving_the_directory_behind`, hands each
directory over with `TempDir::keep` instead of dropping it, so the only
`remove_dir_all` in the process is the one inside `Session::remove`. Nothing
needs attributing afterwards: a wedge is the product path by construction.

Three passes, each 40 runs of 400 rounds across 8 workers:

| pass | timeout | wedged | run times of the clean runs |
| --- | --- | --- | --- |
| 1 | 60s | 0 / 40 | 36 at 2.8–3.5s, then 5.0, 5.2, 8.9, 19.3, 38.1, 39.3, 51.7 |
| 2 | 300s | 0 / 40 | all 2.9–5.8s |
| 3 | 300s | **1 / 40** | all 2.8–4.4s |

**Pass 3 settles what pass 1 could not: `close` blocks indefinitely, not for a
while.** Run 14 sat past the full five minutes and was still parked when the
stack was taken:

    thread 2  slipcase_open::flow::Opened::close
              slipcase_open::session::Session::remove
              std::fs::remove_dir_all
              ntdll!NtQueryDirectoryFile

    thread 4  "notify-rs windows loop"
              notify::windows::stop_watch
              notify::windows::start_read
              ntdll!NtNotifyChangeDirectoryFileEx

Five minutes is a hang by any measure a user would apply, and this is the
shipping path with nothing else in the process that could account for it.

**Two readings written here earlier were wrong, and both were wrong in the
direction of the evidence to hand.** Pass 1's slow runs were read as the
collision stalling and recovering; passes 2 and 3 show clean runs clustered
tightly at three seconds with no middle ground, so those four slow runs were
something else and remain unexplained — antivirus over several thousand leaked
directories is the untested candidate. Then pass 2's silence was read as
`close` never having been shown to block permanently, which pass 3 contradicted
one run later. The lesson is the same both times: forty runs of a one-in-forty
event is one sample, and a rate that has been seen once has not been measured.

**What is established about `close`.** It reaches the collision and stays
there. Twice now with a stack: once through the mixed diagnostic, once here
where nothing else in the process performs a removal. The frequency is roughly
one run in forty at 8 concurrent sessions — 1 in 120 runs across all three
passes, or about one in 48,000 closes — but that is one event and two zeroes,
so treat it as an order of magnitude and not a rate.

**Against the teardown collision**, which reproduces at 9 in 20 and was watched
parked with zero CPU for more than ten minutes: `close` is far rarer and just
as stuck. The suite's flakiness is the teardown; the product's exposure is the
close, and it needs a fix rather than a note.

## A fix for half of it, proved by taking it away

`Watch::drop` now waits for the platform to finish stopping before it returns,
and `Opened::close` drops the watch before removing the session directory.

**Why ordering alone was never a candidate, and the measurements said so
before the code was written.** `notify`'s watcher drop posts `Action::Stop`,
wakes its server thread, and returns — the stop has *started*, not finished. So
the teardown diagnostic, which already dropped the watch before removing the
directory, wedged 9 times in 20 anyway: correct order, asynchronous stop, still
a race. That also accounts for the rates being the wrong way round —
`teardown` overlapped its own stop by construction, while `close` needed a
neighbour's stop in flight and so was rarer.

The stop is unobservable through the ordinary API: `Watcher::new` does
`let (meta_tx, _) = unbounded()` and drops the receiver, so `SingleWatchComplete`
goes nowhere. `Watch::on` therefore builds the Windows watcher through
`ReadDirectoryChangesWatcher::create` with a channel it keeps. `Drop` reads
that channel until the completion arrives, bounded by `STOP_WAIT` so that a
stop which never finishes is a delay rather than a second hang.

**The counterfactual, which is the part that makes this a finding.** One value
changed, everything else byte-identical, same 20 runs of 400 rounds across 8
workers:

| build | teardown gauge |
| --- | --- |
| before the fix | 9 / 20 |
| `STOP_WAIT` = 5s | **0 / 20** |
| same build, `STOP_WAIT` = 0s | **16 / 20** |
| `STOP_WAIT` = 5s, restored | **0 / 20** |

Clean runs stayed at 3.1–3.5s throughout, so the wait is not being paid when
the stop is healthy; it costs nothing in the ordinary case.

Note what the 16/20 is *not*: it is not the 9/20 baseline recovered. Zeroing
the wait restores the wait's absence but not the original construction — the
watcher is still built by `create` with a live meta channel where `notify`
discards it, which changes what the server thread does with its meta events.
The honest comparison is 0/20 against 16/20 within one build. The 9/20 belongs
to a different build and should not be set beside them.

## What it does not fix: the collision crosses directories

`close` is not fixed. After the fix, the isolated close diagnostic wedged 2 in
40 — against 1 in 120 runs before it, which is too few events either way to
call a regression, and it is certainly not an improvement.

Both post-fix wedges were captured, and they say why:

    thread 2  Opened::close -> Session::remove -> remove_dir_all
    thread 3  "notify-rs windows loop"  stop_watch -> start_read

The removing thread had **already waited for its own watch to stop** — the fix
did exactly what it was written to do. The thread parked in `stop_watch`
belongs to a different worker, on a different directory.

**So suspect 2 is answered: the collision crosses directories.** It is not a
session tripping over its own teardown. Once any watch anywhere in the process
is stuck stopping, its server thread stays parked in the kernel — `STOP_WAIT`
bounds *our* wait, not that thread — and every concurrent `remove_dir_all` in
the process blocks behind it, whatever directory it is removing.

That is why no ordering inside a single session can fix `close`, and why the
fix above reaches the teardown collision and stops there.

**What the fix is still worth.** The teardown collision is the suite's
flakiness and the reason `.github/workflows/windows.yml` carries wedge-retry
logic at all. Nine runs in twenty to zero, proved by removal, is that problem
solved.

**Two candidates for the rest, neither yet measured.**

1. Serialising stops against removals process-wide. A stop only gets stuck
   when an enumeration is running against it, so if no removal may run while
   any stop is in flight, neither can wedge. It stays inside this engine, and
   healthy stops take microseconds, so the contention should be invisible —
   "should" being what the harness is for.
2. Fixing it upstream: `handle_event` re-arming a read while its watch is being
   cancelled is `notify`'s defect, not this crate's. That fixes it for
   everybody and needs vendoring or a round trip through the project.

## The product's own shape does not reach this at all

Every diagnostic above closes sessions from several threads at once. The
product does not. `Resident`'s accepting thread only forwards streams down a
channel; `handle`, `turn`, `stand_down` and every `close` run in the single
main loop. Two sessions are never closed concurrently, and the eight concurrent
closers these diagnostics use correspond to nothing the product does.

`a_stand_down_shaped_close` is the shape it does have: eight sessions open
together, each holding a live watch, then closed one after another on one
thread. That is `stand_down`, which is what quitting with several sessions
open runs.

| build | product shape, 40 runs x 400 closes |
| --- | --- |
| `STOP_WAIT` = 5s | 0 / 40 |
| `STOP_WAIT` = 0s | 0 / 40 |

**Both arms clean, and the second is the one worth having run.** A quiet
harness proves nothing unless the same harness wedges when the fix is removed,
and this one does not. So this measurement cannot validate the fix on this
path — what it says instead is that the path was never reachable, with the fix
or without it.

For contrast, at the same exposure and in the same build, two *concurrent*
closers wedged 1 in 40 and eight wedged 2 in 40.

**How strong this is, stated carefully.** 0/40 against 1/40 is thin: those two
numbers are close to indistinguishable, and nothing here rules out a rate too
low for forty runs to see. The stronger argument is structural rather than
statistical. The collision needs a `stop_watch` in flight at the same moment as
a `remove_dir_all`. On one thread that waits for each stop before returning,
that cannot arise at all. Without the wait it can — one close's stop could
still be running when the next close's removal begins — and 16,000 closes did
not catch it, which says that window is very small rather than that it is shut.

**What this settles.**

- The exposure that can be demonstrated needs concurrent closers. Residency and
  the tray do not create any: they hold several sessions in one process, and
  close them one at a time.
- So dropping the tray would not have fixed this. Nor would the simpler
  one-session-per-process model on its own — two concurrent closers is enough
  to wedge, and an open session beside a lingering one is two watches.
- The fix's demonstrated worth is the teardown collision, which is the suite's
  flakiness: 9/20 to 0/20, and 16/20 when the wait alone is removed. On the
  product path it is insurance against a window nothing has been able to open.
- The defect itself stays where it lives: `handle_event` re-arming a read
  during cancellation, in `notify`. Four lines, in somebody else's crate.

## Where it was thought to be — superseded by the stack above

Kept as written on 2026-09-07 before the stack was read. The localisation in
this section is wrong: the hang is not inside `wait_and_pump`.

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

Rewritten again once the fix landed and the counterfactual ran. Two of the
original three are retired and one is answered.

1. **Answered, and it is the root cause.** `notify` v8's `handle_event`
   re-arms `ReadDirectoryChangesW` on `ERROR_SUCCESS` instead of posting
   `complete_sem`, while `stop_watch` waits `INFINITE` for that semaphore. The
   re-armed read does not return while an enumeration is walking a directory,
   and every capture in this file has a thread parked exactly there.
2. **Answered: the collision crosses directories.** A thread that has already
   waited for its own watch to stop still blocks in `remove_dir_all` behind a
   *different* worker's `stop_watch`. So nothing a single session does to its
   own ordering can be sufficient, which is why the fix reaches the teardown
   case and not `close`.
3. **Done for the teardown case, open for `close`.** Waiting for the stop
   before removing takes the teardown gauge from 9/20 to 0/20 and back to
   16/20 when the wait alone is removed. `close` is unchanged at 2/40, for the
   reason in 2.

Retired long ago and worth not revisiting: the drain flooding (no thread has
ever been burning CPU in any capture) and `write_back` or the filesystem under
it (neither has appeared on any stack).

**The diagnostics** are `flow::tests::close_alone_under_repetition`,
`teardown_alone_under_repetition`, `close_with_a_neighbouring_watch` and
`close_alone_leaving_the_directory_behind`, all `#[ignore]`d and sized by
`SLPC_CLOSE_ROUNDS` and `SLPC_CLOSE_THREADS`. Two things to know before
trusting any of them:

- **A single-threaded stress loop here proves nothing.** 40,000 rounds of the
  shape that wedges produced nothing at one worker; put the threads back and it
  wedges inside one 400-round run. Any harness added later starts concurrent
  and demonstrates it can catch the bug before its silence is believed.
- **`close_alone_under_repetition` cannot measure a `close` rate**, because it
  drops a `TempDir` every round and can wedge in that instead — three of its
  four captured wedges did. `close_alone_leaving_the_directory_behind` exists
  for that reason: it leaks its directories so the only removal in the process
  is the one inside `close`.
