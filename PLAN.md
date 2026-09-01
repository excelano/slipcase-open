# slipcase-open — build plan

*Companion to `slipcase-open-concept.md`, which is the design. This is the
order it gets built in and the decisions taken to start.*

---

## Decisions taken at the start

**Repository: `excelano/slipcase-open`.** Its own repository rather than a
fourth member of the `slpc-rust` workspace, for the reasons in concept §14. One
binary crate, with the engine as UI-free modules inside it and no middle crate
until something else needs one.

**Test fixtures come from `slpc-rust` rather than being written again.**
`testsupport` is `publish = false` and carries no version, so a git
dev-dependency on the repository is the way to reach it from outside the
workspace, and that preserves the reason it exists: two copies of the
mark-a-file helper disagreed about the Windows arm within an hour of being
written, and a third copy here would undo that on purpose. The conformance
corpus is in the `slipcase` specification repository under `conformance/`, with
`generate.py` and `manifest.toml`; `slpc-rust`'s `corpus` crate is the runner
that drives it and not a fixture library, so it is not what gets depended on.

**Build order departs from concept §12, which needs one line amended.** "Windows
first" was written about the security layer and reads now as a build order. The
engine and the Linux platform implementation come first: that is the machine the
work happens on, the presentation baseline is least demanding there — no
AppUserModelID, no bundle — and it is the shortest path to something that runs
end to end.

The usual objection is that this defers the riskiest work. It does not, because
the Windows unknowns are already retired elsewhere in the project: `slpc`
carries Mark of the Web through `provenance`, and `slipcase-desktop` has done
registry ProgID resolution and MSIX packaging. What is genuinely new here is the
session model, the watcher, write-back and recovery, and all of that is
portable. The platform trait is defined against Windows' requirements even while
only the Linux implementation exists, so nothing is redesigned when Windows
lands.

---

## Phase 0 — the `slpc` accessor

Concept §14 needs `Container::payload_crc()`: the CRC-32 the ZIP central
directory already records for the payload member, which lets recovery compare
the extracted payload against the container without keeping a record of its own
(§6.3).

It is one field on the private `Entry` struct, populated in `entries_of` from
the same pass that already reads the name, the size and the kind, and one
accessor mirroring `payload_size` — the same `Unsupported::Version` refusal, the
same shared borrow. The doc comment says it is the ZIP field rather than a
slipcase one and disclaims fixity, because SPEC §5 defines no fixity key and a
format library exposing a checksum invites the reading it declined to license.

**Committed, not released.** The changelog entry sits under `[Unreleased]` and
0.3.11 waits. Nothing published consumes the accessor;
`slipcase-open` is on a path dependency through Phase 2 by design; `slpc` and
`slipcase` version in lockstep, so a release is the full cycle in
`RELEASING.md` including the apt push that document flags as the step a release
loses; and the design work so far has turned up three separate places where the
answer was *`slpc` already does that, or should*, which makes a second release a
fortnight later the likely outcome of cutting one now.

**Released as 0.3.11 on 2026-08-31, and the switch is made.** The apt push was
the last item in Phase 3 and needed a tagged release here, which needed this
crate off the path dependency, which needed 0.3.11 published. `slpc` now
resolves from the registry with a checksum, and the suite is green against it —
which is concept §14's argument settled by measurement: the library has been
exercised as a published crate by a consumer outside its workspace, which
nothing in that workspace can do on its own, because the CLI reaches it by path.

`testsupport` stays a git dependency. It is `publish = false` and carries no
version, and this crate is not published either, so nothing about it changed.

## Phase 1 — the engine, headless

The bulk of the work, and all of it testable without a desktop.

**Done.** Container open and validation through `slpc`. Extension extraction
mirroring `slipcase-desktop`'s `Path::extension` rule so the two products never
disagree about what a payload's extension is (§5.2). Policy as a pure function
over the §10 precedence chain, behind a source trait whose first implementation
reads TOML files at paths it is given. The session directory and its TOML record (§6.4). Extract, launch behind
the platform trait, watch the session directory with `notify`, sibling detection
(§6.1). Write-back through `Destination::in_place`, validated through
`written()` before the rename (§7). Recovery by CRC comparison (§6.3).

Driven entirely by CLI verbs — open, sessions, recover — with no notifications,
no tray and no IPC. The CLI is the test harness before it is an interface, and
concept §9 keeps it as the floor afterwards.

**`close` is not among them, and listing it here was an error.** In this phase a
session lives inside the foreground process that opened it, so there is nothing
for an out-of-band `close` to talk to; it would have to find another process's
session directory and act on it blind. The verb becomes meaningful in Phase 2,
where a resident instance holds the sessions and the front door reaches it.

One thing changed shape while it was built. Reading a layer can fail, and the
trait says so rather than answering *says nothing*: an administrator's deny list
that will not parse is the case §10 cares about most, and flattening that into
silence would permit whatever the file was written to refuse, quietly, for as
long as the typo survived. A layer policy has already suppressed is not read at
all, so a broken file that was going to be ignored cannot fail a decision it
would have played no part in.

## Phase 2 — the process model

Single instance and the IPC front door (§8). The session table keyed on file
identity rather than path, deduplication of a container already open,
recovery-before-new-session ordering, and the exit rules. Unix socket first,
with the trait shaped so a named pipe drops in. The `close` verb, which has
something to talk to from here on.

**The instance runs in the foreground.** Detaching means `fork` and this crate
forbids `unsafe`; nothing in concept §8 asks for a background process, and from
Phase 3 the tool starts from a desktop entry rather than a shell, where there is
no terminal to hold. An invocation that loses the race to bind hands over to
whoever won it rather than failing.

**Done except the linger, which Phase 3 finished.** Concept 8 says a closed
session whose editor is still working should keep the process alive, so a live
watcher notices the save and prompts once. There was nothing to prompt through
until §9's notifications, and observing a save this tool may not act on (§6.3)
is worth nothing on its own — so this phase left the instance exiting when the
table emptied, and *the recovery question comes first* as a refusal naming two
commands. Both became what concept 8 asks for once there was somewhere to ask.

**The recovery sweep lands here rather than in Phase 1, and it was blocked rather
than forgotten.** Concept §6.3 says a recovered payload matching its container
means nothing was lost: clean up and say nothing. Nothing implements that half,
because in Phase 1 it cannot be done safely — a session that is open and not yet
edited reads as unchanged, and no process can tell a live session from a dead
one, so a sweep run from a second terminal would delete a directory out from
under a running editor. The session table is what supplies that distinction, so
the sweep is written against it.

## Phase 3 — Linux front end and packaging

`org.freedesktop.Notifications` with actions, the CLI session list, the desktop
entry, the root-owned `/etc/slipcase` policy source, `cargo-deb`, and the
Excelano apt repository. The shared-mime-info type moved out during the phase:
`slipcase-common` declares it and ships the icon, and both products depend on
that rather than each carrying a copy dpkg would refuse.

**Done, and released as 0.1.0 on 2026-08-31.** Concept 9's channel is a trait the engine narrates
and asks through, with D-Bus behind it and the terminal beneath that, and the
two things it unblocks are in: concept 8's linger, where a closed session the
application has not finished with keeps its watch until the last save lands, and
the recovery question, which is a question with buttons rather than a refusal
naming two commands. `packaging/README.md` says what was measured.

**`NoDisplay` on the desktop entry was wrong and the line above no longer asks
for it.** Concept §4 is amended with the finding: on Linux the Open With list is
built from applications registered against the media type, so the single entry
is both the association and the secondary verb, and `NoDisplay=true` takes it
out of the one list §4 needs it in. There is no second entry to ship.

The chain that stood in front of the apt push ran in order: `slpc` 0.3.11 to
crates.io, this crate off the path dependency, `slipcase-common` 1.0.0 for the
media type, `slipcase-desktop` 0.1.4 onto it, and then this.

The tool is complete and shipped on one platform. Phase 4 is next.

## Before Phase 4 — what CI found

`.github/workflows/linux.yml` runs the gate, the cross-target checks, and the
package on every push. Writing it turned up four things nobody had seen, which
is the argument for having had it sooner.

**The Windows arm of `identity.rs` never compiled.** It used
`MetadataExt::volume_serial_number` and `file_index`, which have been behind the
unstable `windows_by_handle` feature since 2019, and this crate builds on
stable. It answers `None` now, which concept §8 already provides for — the
lookup falls back to the canonicalised path and accepts the narrower guarantee,
losing the hard-link arm visibly rather than by an approximation. **Phase 4 has
to settle it**: `GetFileInformationByHandle` through a crate carrying the unsafe
(`same-file` is the obvious one) or `windows-sys` with `forbid` lifted in that
module alone.

**The binary cannot be built for Windows at all**, because the front door and
the resident loop are `cfg(unix)` until concept §8's named pipe is written. CI
checks the library for Windows and everything for macOS, which is the truth
rather than an oversight — macOS is a Unix and compiles whole.

**The shipped package had no Debian changelog**, which policy makes an error.
0.1.0 and 0.1.1 went out without one, so nobody installing from apt could see
what changed. cargo-deb needs `changelog = "debian/changelog"` named explicitly;
it looks at neither `./changelog` nor `debian/changelog` on its own, which is
measured.

**And no manual page**, which is a lintian warning and which this repository now
fails on. Concept §9 keeps the command line a shipped interface rather than a
test harness, so it earned one.

## Phase 4 — Windows

`ShellExecuteEx` and `IAttachmentExecute`, registry policy with the ADMX/ADML
pair, the named pipe with its SID ACL, toast with actions, the tray, the ProgID
and its secondary verb, MSIX and the Store, winget.

**Under way.** File identity, the named pipe, a gate that runs on a Windows
machine, and the launcher are in. The registry policy source, the toast, the
tray, the ProgID and the packaging are not.

**`IAttachmentExecute` is not what reads Mark of the Web, and the line above is
wrong about it — as is concept §12, which needs the same amendment.** Measured
on 2026-09-01 by asking `CheckPolicy` about ten files, marked and unmarked,
across five extensions, and reading the raw `HRESULT` rather than the `Result`
the bindings collapse it into: the marked and unmarked answers are identical in
every case, and what moves them is `SetSource` and the extension. That interface
is for a client which has *received* an attachment and is deciding whether to
save and run it, so the zone comes from the source it is told about rather than
from the file. This tool arrives after that point, with the payload already on
disk and already marked by `slpc::provenance`.

What does consult the mark is `ShellExecuteEx` itself, so the launcher is that
call with the two flags that would switch the check off — `SEE_MASK_NOZONECHECKS`
and `SEE_MASK_FLAG_NO_UI` — deliberately absent. A requirement that is a
negative is weaker than one that is a call, so the mask is a named constant with
a test over it. `platform::shell` holds the table and the reasoning.

**The binary could not be built for Windows at all until the pipe landed**, which
is why the order here was identity, pipe, CI, launcher rather than the order the
line above lists. `endpoint::bind` and `resident::run` were `cfg(unix)`, so
nothing downstream of them could be tried.

**The gate had never run on this platform, and it found four clippy errors older
than any of this work.** CI checked Windows with `cargo check` and linted only
the host. `windows.yml` runs the whole gate now, and `linux.yml` cross-checks
`--all-targets` rather than the library alone.

**Two things are proven end to end on Windows and one is not.** A marked
container opens, the payload is extracted carrying `ZoneId=3` and its `HostUrl`,
the registered application is launched, an edit to the payload is written back,
and the repacked container is still marked. What is *not* proven is that the
zone warning is displayed for a payload that earns one: nothing automated can
watch a modal dialog, and the suite never reaches the launcher on any platform.
That one needs a person at a desktop.

## Phase 5 — macOS

The sandbox question in §15 is resolved first, because it may move where a
session lives on that platform (§6.4). Then the bundle, the exported UTI,
`openFile`, `NSStatusItem`, `UNUserNotificationCenter`, configuration profile
policy, and whichever channel that check chose.

---

## Running alongside

**The macOS sandbox check.** Whether a sandboxed editor can open a path inside
another application's container. `slipcase-desktop` already ships an
extract-and-launch button to the Mac App Store, so the subject is to hand.
Wanted early because it decides both §6.4 on macOS and the §15 channel, but it
gates nothing before Phase 5.

**Deferred to implementation.** Concept §17 holds four items that are settled by
writing the code rather than by more design: a payload size warranting a
warning, what a mid-session policy change does, removing the first-run shortcut
where §15 has not made it moot, and whether the state directory's backup
exposure is said to the user or only to administrators.
