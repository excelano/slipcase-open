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

**Phase 3 is what forces it, and that is where the wait ends.** The apt push is
the last item in that phase and needs a tagged release here, which needs this
crate off the path dependency, which needs 0.3.11 published. It is also what
finally exercises `slpc` as a published crate — concept §14's argument, which
nothing in the workspace can make on its own, because the CLI reaches the
library by path.

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

**Done except the apt push.** Concept 9's channel is a trait the engine narrates
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

What is left is the apt repository, and it is the end of a chain rather than a
task of its own. It needs a tagged release, which needs the switch off the
`slpc` path dependency, which needs `slpc` 0.3.11 on crates.io — Phase 0's held
release, and the thing that finally exercises `slpc` as a published crate.

The tool is complete and shippable on one platform at the end of this.

## Phase 4 — Windows

`ShellExecuteEx` and `IAttachmentExecute`, registry policy with the ADMX/ADML
pair, the named pipe with its SID ACL, toast with actions, the tray, the ProgID
and its secondary verb, MSIX and the Store, winget.

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
