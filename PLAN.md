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

Lands in `slpc-rust` as a patch release. `slipcase-open` develops against a path
dependency and switches to the published version before it ships.

## Phase 1 — the engine, headless

The bulk of the work, and all of it testable without a desktop.

Container open and validation through `slpc`. Extension extraction mirroring
`slipcase-desktop`'s `Path::extension` rule so the two products never disagree
about what a payload's extension is (§5.2). Policy as a pure function over the
§10 precedence chain, behind a source trait whose first implementation reads a
file. The session directory and its TOML record (§6.4). Extract, launch behind
the platform trait, watch the session directory with `notify`, sibling detection
(§6.1). Write-back through `Destination::in_place`, validated through
`written()` before the rename (§7). Recovery by CRC comparison (§6.3).

Driven entirely by CLI verbs — open, sessions, close, recover — with no
notifications, no tray and no IPC. The CLI is the test harness before it is an
interface, and concept §9 keeps it as the floor afterwards.

## Phase 2 — the process model

Single instance and the IPC front door (§8). The session table keyed on file
identity rather than path, deduplication of a container already open,
recovery-before-new-session ordering, and the exit rules. Unix socket first,
with the trait shaped so a named pipe drops in.

## Phase 3 — Linux front end and packaging

`org.freedesktop.Notifications` with actions, the CLI session list, the desktop
entry and shared-mime-info type plus the `NoDisplay` secondary entry (§4), the
root-owned `/etc/slipcase` policy source, `cargo-deb`, and the Excelano apt
repository.

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
