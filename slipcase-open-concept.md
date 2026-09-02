# slipcase-open — concept

*Status: building. The design is settled and this document is it; `PLAN.md` has
the order and what is done. Amendments found while building are marked in place
rather than folded away, so the reason a paragraph changed stays with it. §17
lists what was deliberately left to implementation.*

---

## 1. What it is

A minimal companion to the Slipcase CLI and viewer: double-click a `.slpc`, the
payload opens in whatever application normally handles it, and edits made there
are written back into the container.

No metadata UI. No container browsing. No preview. The payload, its own
application, and a write-back path.

## 2. Why it's separate from Slipcase (the viewer)

`slipcase-desktop` is metadata-first and deliberately has no payload preview.
Its audience is people who care about the metadata — one container at a time,
or, if the inventory mode is ever built, a corpus at a time.

There is a second audience with no interest in metadata at all: someone who has
been sent a `.slpc` and wants the document inside it. For them the viewer is a
detour, and the honest alternative today is "extract with 7-Zip, open the file."
`slipcase-open` makes that path direct and, unlike 7-Zip, closes the loop by
putting edits back.

Splitting it out keeps the viewer's scope clean and keeps this tool small enough
to have no GUI framework at all. Notifications carrying actions cover what it
has to say, with a tray or menu bar item where the platform has one and the
command line where it does not (§9).

## 3. Non-goals

- Editing or displaying metadata (that is the viewer's job).
- Rendering payload content (permanently out of scope across the project).
- Browsing containers, searching, or anything corpus-shaped.
- Acting as a security boundary (see §11).

## 4. How it is invoked

Double-click, the same as any other document. The tool registers as a handler
for `.slpc` and `application/x.slipcase+zip` through the platform's ordinary
mechanism: a ProgID on Windows, an exported UTI on macOS, a shared-mime-info
type and a desktop entry on Linux.

`slipcase-desktop` claims the same association, so a machine with both installed
has two products contending for it. That is rarer than it looks, because the
audiences barely overlap. Someone who wants the metadata has no use for the
payload path, and someone who was sent a document and wants to read it has no
use for the viewer. The answer is to not engineer around it: last installed
wins, which is what the platform does with every other duplicated association,
and the user can change it once in the platform's own default-application UI.

Two things follow. Each product registers a secondary verb alongside the
association it claims, "Open payload" here and "View metadata" in the viewer, so
that whichever is not the default is one click away rather than unreachable.

And neither product re-asserts the association at launch or asks to be made the
default. Registering at install time and accepting the user's override is
ordinary behaviour; a prompt on every start is how two tools sharing an
extension turn into a support problem.

**Amended while building it: on Linux there is no second thing to register.** A
desktop entry naming the media type is already in the Open With list, so the one
entry is both the association and the verb, and the work is to keep it visible
rather than to add anything. `NoDisplay=true` is what would take it out —
measured, an entry carrying it answers false to `g_app_info_should_show()`, the
predicate GIO documents for menu display — and hiding it there would leave the
non-default product unreachable, which is what this paragraph exists to prevent.
Windows and macOS keep the second registration, since a ProgID verb and a
bundle's document role are separate declarations there.

The command line stays a first-class entry point regardless of any of that. It
makes the tool testable and scriptable with no association registered at all,
and it is the path the viewer would use if it ever grows a button that hands a
container over.

## 5. Flow

1. Open the container via `slpc`. Validate per SPEC §3 and §6.
2. Take the extension from `payload.file` (§5.2) and check it against effective
   policy (§10). Refuse and explain if it is disallowed, or if there is no usable
   extension (§5.1).
3. Extract the payload into a session directory of its own: per-user,
   access-controlled, holding that one file and nothing else (§6.1, §6.4).
4. Apply the host platform's trust-zone marking, propagated from the container.
5. Launch via the platform's attachment-aware execution path.
6. Register a session and watch the containing directory for modification.
7. On modification, repack into the container.
8. On session close, do a final repack — or offer one, if no modification was
   ever seen — and remove the session directory, unless the application is
   still working in it (§6.2).

### 5.1 Policy keys on the extension, and content sniffing is not a control

The obvious design determines the payload's real type by content, checks that
against policy, and refuses when content and name disagree. It is wrong, and the
reason is worth writing down, because it looks like the careful option.

**Launch dispatches on the extension.** `ShellExecuteEx`, `open` and `xdg-open`
resolve the handler from the name, and none of them reads the bytes. The
extension alone determines what runs. So a payload whose bytes are a PE image
and whose name is `invoice.pdf` opens in a PDF reader, which fails on it. There
is no path by which the sniffed type reaches the loader, and a policy check
against the sniffed type is therefore a check on a value that has no bearing on
what executes.

Sniffing is also imprecise where it would matter most. OOXML and ODF are ZIP
archives, so a `.docx` and a bare `.zip` are the same bytes to a general
classifier without reading their internal structure, and a rule that refused
every disagreement would refuse most real office documents. Under the design
above, none of that needs solving, because nothing needs to tell those two
apart.

**Policy therefore allowlists extensions.** That is what the platforms dispatch
on, it is the vocabulary administrators already write AppLocker and attachment
manager rules in, and it needs no shipped table mapping names to types — a rule
`slipcase-desktop` states as DESIGN §3 and holds to by asking the platform
rather than carrying its own map.

**One narrow content check survives, and it is not a policy check.** The bytes
are read far enough to see whether the payload is an executable image — MZ/PE,
ELF, Mach-O — or a script with a shebang, under an extension that is none of
those. Nothing else is reported: a `.docx` sniffing as a ZIP is noise and goes
unmentioned.

That is a handful of magic bytes rather than a type table, it costs the first
few bytes of the payload, and it fires close to never in ordinary use. Because
it fires rarely and means something specific when it does, it earns an interrupt
rather than a badge somewhere quiet. What it means is that the container's
payload is an executable wearing a document's name, which is the shape of a
phishing attachment, and a person shown that sentence will usually stop.

It still does not refuse. The extension governs what runs, so a PDF reader
handed a PE image fails on it harmlessly, and refusing would be asserting a
control this path does not carry. The user is told and then decides.

**One case does need a refusal, and it is not a mismatch.** A payload with no
extension, or one the platform has no registration for, makes `ShellExecuteEx`
present the Open With dialog, which hands the choice of executable to the user
inside a flow they believe is "open the document." Require a known, allowlisted
extension and refuse when there is not one.

### 5.2 What counts as the extension, and how it is compared

`slipcase-desktop` already answers the first half, and the two products must not
disagree about what a payload's extension is. It takes the extension with
`Path::extension()`, which gives `gz` for `archive.tar.gz` and nothing at all
for `.bashrc`, and it has tests pinning both. Use the same rule.

**Comparison against policy folds ASCII case, and an extension that is not
ASCII alphanumeric is not allowlistable.** A payload carrying one falls into the
refusal above for a payload with no usable extension.

Folding has to match the way the platform resolves the handler, or policy and
the shell disagree and the disagreement is the defect. Windows registry keys are
case-insensitive; shared-mime-info lowercases the filename before matching a
glob unless that glob is marked case-sensitive. Both fold ASCII and neither does
anything more elaborate, so full Unicode case folding would have this tool
drawing distinctions the launch path does not — U+212A folding to `k` is the
worked example — and each such divergence is a place where policy permits one
thing and the shell opens another. Registered types are ASCII in practice, so
excluding the rest costs little and the refusal can be explained.

The deny list folds the same way, and reaches the same set. Nothing should be
refusable by a rule the allow list could not have expressed.

`payload.file` is attacker-controlled, and SPEC §2.3 constrains it only to being
a plain filename. The extension is what follows the last `.` in the decoded
name, so the right-to-left override that dresses a `.exe` as a `.pdf` survives
that rule intact. SPEC §3 already requires bidirectional formatting characters
to be rendered escaped rather than applied, and refusal messages are a display
path bound by it.

## 6. The hard part: knowing when to write back

Write-back detection is where this tool is genuinely difficult, and the failure
modes are not obvious.

**Process exit is not a reliable signal.** Launching a document frequently
returns immediately, because the file is handed to an already-running instance
of the target application. There is often no child process to wait on. Roughly
half of real-world applications behave this way.

**Applications save atomically.** Serious editors write to a temporary sibling
and rename over the target. A watcher registered on the file itself loses its
handle on the first save. The watch must be on the *directory*, matching by
name. (`notify` supports this on all three platforms, but only if set up
deliberately.)

**Save As is invisible.** If the user saves to a different location, no event
fires, and the container silently retains the original payload. There is no
detection for this. The only correct response is to not claim a write-back
happened.

### 6.1 One signal the directory watch gives for free

The payload is extracted into a directory of its own, holding that one file.
Everything else that subsequently appears there was created by the target
application, by construction. A lock file, an autosave, a backup, a save-in-
progress temporary: the tool does not need to know which, or what any of them
are called.

Siblings present means the application is working in there, which process exit
does not tell you. Siblings gone means it has cleaned up and has probably
finished. That is worth more than a table of `~$name.docx` and `.~lock.name#`
conventions, it generalises to applications nobody thought to enumerate, and it
keeps this tool free of the shipped table `slipcase-desktop` DESIGN §3 refuses
on the same grounds.

It stays a heuristic. Most read-oriented applications write no sibling at all,
so an empty directory means nothing, and an application that leaves a `.bak`
behind forever never produces the cleaned-up signal. Both degrade to silence,
which is the intended fallback. What the signal is good for is driving tray
state, and timing the close prompt to arrive when the user has put the document
down rather than at an arbitrary moment.

### 6.2 Consequence: an explicit session model

Because detection is unreliable, the tool should not pretend to be invisible.

- Opening a container starts a **session**, tracked and visible.
- The open sessions, and the number of write-backs each has had, are listed
  somewhere standing (§9).
- The user **closes the session explicitly**, which performs a final repack and
  cleans up, subject to the application having finished with the directory.
- A session surviving a crash is recoverable on next launch, because the
  payload and the session record are still on disk (§6.3).

This is more honest than a silent watcher, gives a place to surface "the payload
changed — write it back?" if confirmation is wanted, and gives the user
somewhere to look when they expect an edit to have landed.

**Closing a session that saw no modification event offers a write-back rather
than skipping one.** This is the only available answer to Save As. It costs one
dialog on a path the user is already interacting with, and the alternative is a
container that silently keeps the old payload after the user believes they
edited it. Asking is not detection and should not be described as though it
were: the prompt says the payload was not seen to change, and lets the user say
otherwise.

**Closing a session while the application still has the payload open does not
delete the directory.** The close is honoured: the final repack runs as it
would otherwise. But where siblings say the application is still working in
there (§6.1), the directory is handed to the recovery mechanism rather than
removed, and the next launch picks it up through §6.3 and asks. The editor's
next save then lands somewhere the tool will still look, at the cost of a
session directory living until the next launch instead of until the close.

Warn at close and offer to proceed anyway, but do not make the warning the only
protection. A user who chooses to go ahead should still not lose the save.

**Individual write-backs during a session are not confirmed by default.** A
repack is atomic (§7) and unremarkable, and a prompt on every save is friction
for the common case. Per-write confirmation is a setting for the archival
audience, off by default.

### 6.3 Recovery never writes back on its own

A session that survives a crash is recoverable because the payload and the
session record are still on disk. Recovery must not act on them. The tool was
not watching when the process died, so it cannot distinguish a complete save
from a half-written one, and putting a truncated save over the container is the
worst outcome this tool is capable of producing.

It can narrow what has to be asked without keeping a record of its own. The ZIP
central directory already stores a CRC-32 for the payload member, so recovery
computes the CRC of the extracted payload and compares. Equal means nothing was
lost: clean up and say nothing. Different means an edit never landed, and since
a complete one and a truncated one look alike, it becomes a recovery item naming
the container, the payload, and the payload's modification time, offering
write-back, discard, and reveal-the-folder. Nothing happens until the user
chooses, and the session directory survives until they do.

**Comparing against the container beats recording a digest.** A recorded value
is a second copy of a fact and can drift from it, and the moment it gets
consulted is after a crash, which is when a session record is least
trustworthy. The container's own stored CRC needs nothing maintaining it:
repacking recomputes it, so the comparison stays correct across every write-back
in a session as a side effect of the write-backs themselves.

`slpc` does not expose it today — `payload_size()` and `payload_mode()` are
there and the CRC is not — and adding the accessor reads a field the container
already carries, which is the format library's job and costs no dependency
(§14). Computing the extracted payload's value needs `crc32fast`, already compiled
as a transitive dependency of `zip`.

**It is change detection and never fixity.** The question is whether the file
changed, not whether it can be proved untampered: anybody able to write into the
user's own access-controlled session directory can do worse than forge a
checksum, so the framing that would argue for SHA-256 does not arise. The value
is never surfaced as an integrity claim and never written into container
metadata. SPEC §5 declined to define a fixity key and this must not become one
by the back door.

Size and modification time were the alternative and are worse. Editors that
preserve timestamps, and coarse mtime granularity, both produce false negatives,
and a false negative here silently discards an edit.

### 6.4 Where a session lives on disk

**Not in the system temporary directory, which is the obvious place and the
wrong one.** A reboot, a `tmpfiles` cleaner, or Storage Sense may delete
anything there. That would destroy an edit the user has made and the tool has
not yet written back, silently, in the window §6.3 exists to survive.

A session lives under the application's own per-user state directory, at 0700:
`%LOCALAPPDATA%` on Windows and deliberately not the roaming profile, since an
extracted payload cannot follow a user between machines; `~/Library/Application
Support` on macOS rather than `Caches`, which the system may purge at will;
`$XDG_STATE_HOME` on Linux, which is defined for state that must survive a
restart without being configuration or data, and not `XDG_RUNTIME_DIR`, which is
cleared at logout.

One directory per session, holding a small TOML record naming the container's
absolute path, the payload name, the session start, and the write-back count.
TOML because `toml_edit` is already in the tree under `slpc`, so the record costs
no dependency. Recovery is then a scan of one tree rather than a record pointing
at a directory that may not be there.

**The payload sits one level below the record, in a `payload/` directory of its
own, and it took writing this to see why.** SPEC §2.3 permits any plain
filename, `session.toml` among them, so a payload beside the record could
overwrite it — a container can be built to do that deliberately. And §6.1 reads
anything else appearing in the payload's directory as the target application's
work, which is a sound inference only where this tool put exactly one file
there. The record sitting alongside would make the tool its own first false
positive. One subdirectory answers both.

The record cannot settle one case and the code has to: the container moved or
was deleted while the session ran. Recovery says so and offers to save the
payload elsewhere rather than failing at the rename.

**The cost of moving out of the temporary directory is that the payload is now
somewhere backup and sync software looks.** A payload from a confidential
container sits in the user's state directory for the life of the session, where
`%LOCALAPPDATA%` and `~/Library/Application Support` are both routinely captured.
That is the right trade against losing edits, and it is a trade rather than a
free fix, so it belongs in the administrator documentation beside §11.

## 7. Write-back mechanics

Repack to a temporary container and swap it over the original. Never modify in
place: an interruption mid-write corrupts the container. Preserve unrecognised
members, as `repack` already does, per SPEC §3.

**`slpc` already implements the swap, and reimplementing it would be a
regression.** `Destination::in_place` resolves the path first, so a container
reached through a symbolic link is replaced rather than the link, and takes the
replacement's permissions from the file being replaced rather than from the
umask. `commit` then calls `provenance::carry` to move the platform's mark —
Mark of the Web on Windows, `com.apple.quarantine` on macOS — onto the
replacement *before* the rename, and fails the commit if the mark cannot be
carried, so a marked container is never replaced by an unmarked one. The
ordering of the permission set and the carry is deliberate and was measured: a
read-only marked container came back from `repack` with no mark, silently and
with exit zero, because the mode was applied first and the attribute write was
then denied.

This matters because the naive version looks correct and fails quietly. A plain
`std::fs::rename` is `MoveFileEx`, which carries over neither the target's ACLs
nor its alternate data streams, and Mark of the Web is an alternate data stream.
The repack-and-rename that anyone would write first strips the container's trust
zone on the first save, with no error and no visible symptom.

**The residual gap is Windows ACLs.** `ReplaceFileW` would carry those across
where `carry` does not, but taking it would mean leaving `slpc`'s commit path
for a narrower benefit: the replacement is created in the container's own
directory and inherits the same ACEs, so only an explicit non-inherited ACE on
the container is lost. Document it alongside the ownership and hard-link
caveats `in_place` already names. A container reachable under two names is
rewritten under only the one it was opened by, which is the standing cost of
replacing a file rather than writing into it.

**Validate the repacked container before it replaces anything.** `slipcase
repack` reads its own output back through `Destination::written()` before the
rename, which is the difference between replacing the only copy of a container
on faith and doing it on evidence. Write-back has more reason to do this than
`repack` does, because it runs unattended and repeatedly.

**Write-back does not touch the metadata member.** SPEC §5 defines no checksum
or fixity key, and §2.2 assigns no meaning to any key beyond `slipcase_version`
and `payload.file`, so there is nothing in a conformant container that a changed
payload can falsify. A producer may have recorded its own size or digest under a
private key permitted by §2.5, but since the specification gives those keys no
meaning, this tool cannot know which key, what it covers, or how it is encoded.
Guessing is worse than leaving it: a wrong digest is a false claim, where a stale
one is at least a claim whose provenance is the producer's. The metadata member
is preserved byte for byte, and the administrator documentation says so.

## 8. Process model

**The tool is a resident single instance, and that is forced rather than
chosen.** §6 starts from the observation that launching a document returns
immediately, so something has to outlive the launch to hold the watch. And a
session list in the plural means a second invocation hands its container to the
first rather than starting a rival with a session list of its own.

So there is a front door: a named pipe on Windows, a Unix domain socket on macOS
and Linux, and on macOS the `openFile` event that hands a document to a running
bundle without passing it as an argument, which `slipcase-desktop` already deals
with in `opened_document.rs`. Every entry point in §4 is a client of it. Where
no instance is running, the invocation starts one and hands over; where one is,
it hands over and exits.

**A container that already has a live session is not opened twice.** Two
sessions on one container would both repack it, the second write-back would
overwrite the first, and one person's edit would be gone with nothing said. The
resident instance keeps a table of live sessions, and an invocation naming a
container already in it starts nothing: it re-launches that session's payload
and brings the application forward, which is what a second double-click on an
open document does everywhere else. No refusal, no second session directory, no
divergent copies.

**The table is keyed on file identity and on the path, because neither holds
alone.** Device and inode on Unix, volume serial and file index on Windows.
Canonicalising a path resolves symbolic links and does nothing about the
container reachable under two hard links that §7 already warns about, and two
names for one inode are two paths and one file.

**Amended, found while building it: the identity does not survive a
write-back.** §7 replaces the container by renaming a new file over it, so a
container acquires a new inode every time a session saves. An identity recorded
when the session opened stops matching the file at that path after the first
save, and the next invocation of the same container finds no entry and opens the
second session all of this exists to prevent. The path is stable across
replacement and blind to hard links; the identity is the opposite. So a lookup
matches on either, and the identity is re-read after each write-back so that the
hard-link half does not quietly lapse for the rest of a session.

The case where the two keys disagree comes out right rather than by accident.
After a write-back through one of two hard links, §7 says the other name still
points at the original holding the old contents — so by then they really are
different files, neither key matches, and opening the other one is a new session,
which is correct.

Where a filesystem returns no stable identity, which some network mounts do not,
fall back to the canonicalised path and accept the narrower guarantee. A zero
inode is that case rather than an answer: some mounts report one for every file,
and a key that collapsed every container on such a mount into one entry would
refuse to open the second container somebody asked for, believing it already had
it.

**A pending recovery item is resolved before a new session opens on the same
container.** A session left behind by a crash is not in the live table, so
nothing refuses it, but opening a fresh session first would extract the
container's current payload and leave the recovered edit with nowhere to go. So
the recovery question comes first — write it back, discard it, or open fresh and
discard it — and the new session follows the answer.

**The front door is a control surface and has to be treated as one.** §10 says
any value supplied over IPC is a policy bypass, and the requests themselves are
the same problem: a local process that can reach the endpoint could hand the
engine a container, close a session before its final repack, or discard a
recovery item. Restricting the endpoint to the invoking user through the
platform's own mechanism — a pipe ACL naming their SID on Windows, a socket
under a directory only they can traverse elsewhere — is a requirement and not a
hardening measure.

**It exits when nothing is left to watch, and it does not autostart.** The
process lives while an open session or a directory handed to recovery under §6.2
exists, and stops when neither does. Staying resident does nothing for the crash
case, where the process is dead by definition and recovery-on-next-launch is
already the answer. What it helps is the other path into recovery, where the user
closed a session while the editor still had the document open: a live process
keeps watching that directory and notices the save when it happens, rather than
leaving the question until somebody next opens a container. Bound it — once the
siblings are gone and the payload has settled, prompt once, act on the answer,
and exit; if nobody answers, the record on disk carries the question to the next
launch.

**Amended, measured on GNOME Shell 48.7: the notification does not carry it.**
This sentence used to say that §9's persistent notification held the question
alongside the record. It does not, on this platform: a notification is removed
when the process that sent it goes, so the buttons stop working at the moment
the instance exits. Nothing here breaks, because the record on disk was always
the durable half and `sessions` reads it — but two things follow. The bound on
the linger is the whole window in which a question can be answered rather than a
courtesy on top of a durable one, so it is a real interface decision and not a
housekeeping constant. And the message this leaves in place of a withdrawn
question is itself sent from the dying process, so on Linux it goes too; what
somebody actually comes back to is `slipcase-open sessions`.

Autostart on login is not worth three conventions and three sets of expectations
about what may run unbidden, for a tool invoked occasionally whose fallback
already works.

**The engine has no UI dependency.** It is the modules holding everything in §5
through §7: validate, evaluate policy, extract, launch, watch, repack, recover.
Every security-relevant decision lives there, once, on all three platforms, and
it is drivable headlessly, which is how it is tested.

## 9. Presentation

This was written imagining a tray icon, which is a Windows habit. It translates
to a menu bar item on macOS and to nothing dependable on Linux: GNOME has not
had a system tray since 3.26, and GNOME Shell does not implement
StatusNotifierItem without an extension. Designing around one would make the
platform with the weakest security parity also the platform that loses the
session model.

**Notifications carrying actions are the portable primitive, and they are what
the design needs.** Buttons in the notification, and a notification that
persists somewhere the user can return to — with one qualification measured on
GNOME and recorded in §8: through `org.freedesktop.Notifications` it persists
only while the process that sent it lives. All three platforms have both:
`org.freedesktop.Notifications` has carried an actions parameter for as long as
it has existed, and GNOME Shell renders them as buttons and keeps the
notification in its message list; Windows toasts carry actions and persist in
Action Center; macOS notifications carry actions and persist in Notification
Center. That covers the close prompt (§6.2), the recovery item (§6.3) and the
executable warning (§5.1), and it gives §6.2 its "somewhere to look when they
expect an edit to have landed" without a tray anywhere.

So the layering inverts. Notifications with actions are the baseline and are
present everywhere. **The tray, or the menu bar item, is an enhancement** on the
two platforms that have one, worth having because it shows standing state rather
than events: how many sessions are open and how many write-backs each has had,
which a notification is poor at. **The command line is the floor**, always
available, and it is how the session list stays reachable on a desktop that
offers neither.

The cost is packaging work on two platforms rather than one. A Windows toast
with actions from an unpackaged binary needs an AppUserModelID and a Start Menu
shortcut, and macOS needs a real bundle for `UNUserNotificationCenter`. Both are
already required by the association work in §4, so neither is new.

**Windows creates its own user-scope shortcut on first run, and falls back to
the tray when that fails.** This applies to an unpackaged install only: an MSIX
carries an AppUserModelID as part of its identity, so the Store channel in §15
removes the problem rather than solving it. An archive that was unpacked rather than installed
has no shortcut and an AppUserModelID nothing registered, and the toast is what
degrades. The fallback costs nothing there, because Windows is the platform that
reliably has both a tray and a message box: such an install loses toast fidelity
and keeps the whole interaction. On Windows the toast is the nicety and the tray
is the mechanism, which is the inverse of Linux, and that asymmetry is the
better argument for this section's layering than the symmetry it opens with.

Creating the shortcut needs no elevation and also makes an unpacked binary
findable in Start, which that user probably wants. It does sit close to a rule
§4 sets, and the distinction is worth stating rather than glossing: an
association changes how the user's files behave across the system, where a
shortcut changes only whether this application is findable and whether its own
notifications work. Different weight, different rule. Make it visible and
removable rather than silent.

Native dialogs stay available where they exist and nothing depends on them.
Windows and macOS each offer a message box without a toolkit; Linux guarantees
no dialog binary, `zenity` and `kdialog` being often present and never certain.
A step that needs a modal on Linux is a step that has a hole on Linux.

## 10. Policy and administration

The set of payload extensions the tool will open must be configurable by the
user and lockable by an administrator. Extensions rather than media types, for
the reasons in §5.1.

**Precedence:** machine policy → user policy → user configuration → built-in
default.

On Windows, machine and user policy live under the `Policies` subtree
(`HKLM\SOFTWARE\Policies\Excelano\Slipcase`), which is access-controlled against
standard users and cleaned up by Group Policy on unapply. Application settings
must never be read from the normal application key when policy is in effect.
Ship an ADMX/ADML pair; this also enables Intune deployment via Policy CSP
ADMX ingestion.

macOS uses configuration profiles, with `CFPreferencesAppValueIsForced` to
detect the managed case. Linux uses a root-owned `/etc/slipcase/` taking
precedence over `~/.config`.

**Settings needed:**

| Setting | Notes |
|---|---|
| Allowed extensions | The permitted set, compared per §5.2 |
| User may extend | Whether the user's own additions are honoured at all |
| Replace vs. append | Must be explicit. An administrator who sets a list expecting it to be exhaustive, and instead gets it unioned with the defaults, has a silent hole |
| Deny list | Always wins, regardless of every other setting |
| Confirm each write-back | Off by default; on for archival use (§6.2) |
| How much is said | A threshold on §9's weights, not a list of switches. Added while building Phase 3; see below |

There is no setting for the extensionless case. A payload the platform has no
registration for is refused whatever the lists say (§5.1), because the dialog it
would otherwise raise offers the user every executable on the machine.

**Added while building it: how much the tool says belongs here too, and it is a
threshold rather than a list of switches.** Most people will want fewer
notifications than the design produces, and the honest axis is not how loud a
message is but whether anybody asked for it. A write-back happens on its own; a
confirmation of a button somebody just pressed does not, however routine it
looks, and a setting that silenced the second would mean pressing a button and
getting nothing back. So §9's weights carry that distinction and the setting is
one word over them.

Two things follow. **A question cannot be quietened**, because it goes through
the channel's *ask* rather than its *report* and the threshold cannot reach it —
structural rather than a rule to remember, since silencing one would strand a
payload with nothing to say so until the next launch. And **this key needs none
of the launch-path discipline below**: it gates no decision, so it is resolved
once when the instance starts and held.

Putting it in this chain rather than in a settings file of its own is what makes
it work on three platforms without designing anything: it inherits the registry,
the configuration profile, and the precedence, and an administrator can hold a
fleet quiet through the mechanism already specified. The platforms' own
per-application notification settings are the blunt version and are not this
tool's to duplicate — each of the three can switch it off outright. What this
key is for is the middle ground none of them can express.

**Enforcement happens in the launch path**, immediately before execution and
after the extension has been taken from the decoded `payload.file` and folded
(§5.2).
Disabling a control in a settings dialog is cosmetic. Any value cached at
startup, read from a user-writable config file, or supplied over IPC is a
bypass.

**`slpc` is not in the default allowed set, and nesting needs no special case
beyond a depth limit.** SPEC §2.3 permits a container whose payload is itself a
container and gives it no meaning. Opening one composes correctly without
anything coordinating it: the inner session repacks the inner container, that
container is the outer session's payload, the outer watcher sees it change and
repacks the outer, and each session knows only about its own container. So there
is no correctness reason to intervene.

The reason to keep it off by default is judgement rather than correctness. A
nested container is usually somebody having packed one by mistake, or an
archival wrapper, and neither wants an automatic recursive open; the viewer is
the better answer for looking at one. It stays allowlistable for the archival
user who nests deliberately, and the refusal says the payload is itself a
container and names both options.

Depth needs no plumbing over the IPC front door. The engine can see that a
container path it has been handed lies inside one of its own session
directories (§6.4), so depth is structural rather than something to pass along.

The UI should indicate when settings are administratively managed, both to set
user expectations and to reduce support load from "the app randomly refuses to
open files."

**Managed means a layer set something, not that one is present.** The Linux
package installs `/etc/slipcase/open.toml` documenting every key above and
setting none of them, which is how an administrator finds out what the keys are.
A rule counting the file's existence would tell every machine that installed the
package that its settings were administered when nothing had been, which is this
paragraph's own support load arriving by the front door. An empty allow list is
not the same thing: a layer permitting nothing says a great deal.

## 11. Honest limits

**The allowlist is not a security boundary.** The `slipcase` CLI is publicly
distributed and the container is a plain zip; any user can extract the payload
with standard tools and run it. What this control provides is a guardrail
against user error and social engineering in the *convenient* path — which is
where the realistic risk sits — not a barrier against a determined user. The
actual boundary is application control (AppLocker/WDAC and equivalents).

This must be stated plainly in administrator documentation. An organisation that
believes the allowlist is load-bearing may skip the controls that actually are,
which would leave them worse off than before the tool existed.

§5.1's content check is not a second control and must not be described as one.
It reports that a payload is not what its name claims; it refuses nothing, and a
payload that is what it claims to be and still hostile passes it without
comment. The one refusal in that section, for a payload with no usable
extension, closes a dialog rather than inspecting anything.

## 12. Platform coverage

The engine is portable in full. `notify` covers file watching on all three
platforms, and the container swap is portable too, because
`slpc::Destination::in_place` carries the platform-specific part behind one call
(§7). What differs is the security layer and the presentation.

| | Windows | macOS | Linux |
|---|---|---|---|
| Trust-zone propagation | Mark of the Web via `IAttachmentExecute` | `com.apple.quarantine` xattr | **Not available** |
| Zone carried on write-back | `slpc::provenance` (ADS) | `slpc::provenance` (xattr) | Noted, not enforced |
| Launch | `ShellExecuteEx` / `IAttachmentExecute` | `open` | `xdg-open` |
| Notifications with actions | Toast; needs an AppUserModelID | `UNUserNotificationCenter`; needs a bundle | `org.freedesktop.Notifications` |
| Standing session list | Tray icon | Menu bar item | Command line |
| Native dialog | Message box | `NSAlert` | **Nothing guaranteed** |
| IPC front door | Named pipe, ACL by SID | Unix socket, plus `openFile` | Unix socket |
| Association | ProgID | Exported UTI | shared-mime-info + desktop entry |
| Secondary verb | `shell\open-payload` under the ProgID | Services entry | Additional desktop entry, `NoDisplay` |
| Managed policy | Registry `Policies` + ADMX | Configuration profiles | Root-owned `/etc` |

Structure the differences as a small trait — `launch()`, `propagate_zone()`,
`effective_policy()`, `present()` — with three implementations, rather than
treating cross-platform as a yes/no decision.

**Windows first means Windows is the reference, not that it is built first.**
The trait is defined against what Windows requires — an attachment-aware launch,
a zone to propagate, a policy subtree with an ACL — because a trait shaped
around the platform with the weakest security story would have to be widened
later, and widening a security interface after three implementations exist is
how the arms drift. Which platform is implemented first is a separate question
and belongs in `PLAN.md`, where the answer is whichever machine the work is
happening on.

macOS is roughly a day's work once Windows is done, beyond the bundle. Linux
ships with the allowlist and a documented statement that zone propagation does
not exist there.

**These are three risk shapes rather than a ranking, and calling Linux the
degraded one flattens the picture.** Windows has the strongest zone story and the
weakest notification story, needing an AppUserModelID and a Start Menu shortcut
before a toast will carry a button. macOS is close to Windows on zones and needs
a bundle for anything at all. Linux has no zone propagation, which is a real
loss and the one worth stating plainly to administrators, and in exchange it has
the least fussy notification story of the three and the only sandbox story: a
Flatpak's target applications are already confined, and the document portal
exists to hand one file to one application with controlled access and take
changes back, which is this tool's job description. If `slipcase-open` ever
ships as a Flatpak, that is a better mechanism than a session directory rather
than a worse one.

Where Linux is genuinely thinner is the standing session list, and §9 answers
that with the command line rather than by pretending a tray is there.

**Why not Windows-only:** information management shops are mixed environments,
and tooling that only works on Windows makes Slipcase read as a
Microsoft-ecosystem format. That undercuts the neutral-primitive positioning.
Graceful degradation preserves it.

## 13. Implementation language

Rust. The deciding factor is that `slpc` already exists in Rust: a C#
implementation would mean either reimplementing container parsing (two
implementations of the same format, drifting apart) or FFI back to the Rust
core, which was already ruled out as a project-wide pattern. The validation
work in SPEC.md's security section needs to sit next to the parser, not across a
language boundary.

Supporting reasons: a single binary matches existing distribution on two
platforms of three, macOS wanting a bundle around it for the reasons in §9; the
Windows APIs needed are COM *consumption*, which `windows-rs` handles without
friction; and the engine is one body of code beneath three thin platform
implementations rather than three programs sharing a name (§12).

C# would win if this grew a real native UI (WinUI 3) or needed packaged
identity for Explorer context-menu integration. Neither applies at the size
described here, and §9's answer is deliberately the one that does not need a
toolkit on any platform.

## 14. Where the code lives

Its own repository, holding one binary crate that takes `slpc` as a published
dependency from crates.io with the `fs` and `provenance` features on. Both are
needed and neither is default: `fs` is the atomic file placement §7 relies on,
and `provenance` is the mark carried onto the replacement. `fs` implies
`provenance` already, so naming both is redundant in the manifest; name them
both regardless, because this tool uses the second directly and not as a side
effect of the first.

**The engine does not belong inside `slpc`, and the format is the reason before
the dependencies are.** `slpc` is the reference implementation of SPEC.md, so
anything in it reads as part of what a conformant implementation does. SPEC §5
is careful about what the format takes no position on, and how a payload reaches
an application sits so far outside that it never needed excluding. Put launch
policy, session state and a file watcher in there and the format acquires
opinions about opening payloads, which misleads the outside implementer the
crate exists to serve.

The dependency argument points the same way and is easier to check. `slpc` gates
`fs` and `provenance` off by default so that a caller who only reads containers
does not acquire `tempfile` or `xattr`, and both of those features are still
about the container: placing a container file, carrying a container's mark. The
engine would add `notify`, `crc32fast`, a registry reader, `objc2` for
`CFPreferences`, `windows-rs` for `ShellExecuteEx`, and an IPC endpoint. None of
it is about the container, and the line between the two is already drawn where
those two features sit.

**One addition to `slpc` is needed**: an accessor for the payload member's
CRC-32, which the ZIP central directory already stores and the `zip` crate
already surfaces on read. It is a field of the container, so reading it out is
the format library's job, it costs no dependency, and it lets recovery stop
keeping a record of its own (§6.3).

The alternative was the engine reading the central directory itself, and it is
worse than a version bump on any reading. It duplicates the parsing `slpc`
exists to keep in one place, and it does that parsing in the crate with no
fuzzing harness behind it, which is the drift this project is organised against.
The cost is a minor bump and a public accessor supported from then on, over a
field `zip` hands back for free.

Document it as the ZIP field it is rather than a Slipcase one, and disclaim
fixity in the doc comment. A format library exposing a checksum invites the
reading SPEC §5 declined to license, and §6.3 is careful about the same thing
from the other side.

**No middle crate.** The engine is UI-free modules inside the binary crate,
which is enough to test it headlessly, and a workspace split now would buy
nothing and add a version to keep in step. The only plausible second consumer is
the viewer's Open button, and giving the viewer write-back would undo the
separation §2 exists to make. Promote the engine to a crate when something else
needs it and not before.

The `slpc-rust` workspace publishes a library and a CLI, and its shape reflects
that: an MSRV floor inherited from `zip` rather than chosen, a `cargo-dist`
release configuration covering two artifacts, and a lockfile that a fourth
member would populate with `windows-rs`, `notify` and a COM dependency tree
irrelevant to everyone consuming `slpc`. `slipcase-desktop` is already a
separate repository, which settles the precedent for anything with a UI surface.

The argument that runs the other way is worth keeping in view. Nothing currently
exercises `slpc` as a published crate — the in-workspace CLI reaches it by path,
so the crates.io package could be broken in ways the workspace cannot see. A
second consumer that acquires it the way an outside implementer would is a check
the project does not otherwise have.

## 15. Distribution

Both channels, and they are not in tension. The stores serve the person who was
sent a container and has nothing that opens it; winget, Homebrew and apt serve
the person who already knows what this is. That is the audience split §2 is
built on, which is a fair sign the answer is the right shape.

`slipcase-desktop` already took the Microsoft Store and the Mac App Store, and
its packaging notes give the reason: Finder offers *Search App Store* by document
type, so a store listing is how somebody staring at an unopenable `.slpc` finds
the thing that opens it. That argument is stronger here than it is for the
viewer, because §2's audience is that person.

**Windows works either way, and the Store is the better fit.** The viewer's
handoff records that an MSIX process reads an alternate data stream, so Mark of
the Web survives packaging, and MSIX file type registration is proven in that
repository. Packaged identity also supplies an AppUserModelID, which removes the
first-run shortcut §9 otherwise needs. winget is not excluded: its `msstore`
source serves the Store package, and it will serve a plain installer instead.

One consequence for §10: an MSIX runs no code at install, so the package cannot
place ADMX/ADML files into `PolicyDefinitions`. Ship those as a separate
administrator download, which is how Microsoft ships its own.

**macOS needs one thing checked before the channel is chosen.** Under the App
Sandbox the payload sits in the application's own container, and a sandboxed
target editor — which is every Mac App Store editor — may not be able to open a
path inside another application's container. That failure mode is bad because it
is invisible: the payload opens in Word and fails in something from the Store,
with nothing to tell the user why.

The subject to check is already to hand. `slipcase-desktop` DESIGN §3 has an
Open button that extracts a payload and launches whatever the platform
registered, and that application ships to the Mac App Store. If Open works from
the sandboxed build against a sandboxed editor, the objection dissolves and the
Store is right for both products. If it does not, that is worth knowing for the
viewer as well, and this tool takes Developer ID with notarization through a
Homebrew cask.

**Linux is settled.** `cargo-deb` and the Excelano apt repository are the
established path in `slpc-rust`, and a `.deb` carries the binary, the desktop
entry, the shared-mime-info XML, and the root-owned `/etc/slipcase` §10 wants.
Flatpak stays an option worth taking for §12's document portal rather than a
distribution requirement.

## 16. Relationship to the parked Explorer work

The Explorer property handler — surfacing container metadata as Windows
properties so that Explorer's built-in faceted search works over `.slpc` files —
is a separate parked item. It shares nothing with this tool but the `slpc`
dependency.

If both are eventually built, the property handler is clearly an excelano
product rather than part of the reference implementation: it has no portable
analogue and would be the largest block of platform-specific code in the
project. `slipcase-open`, by contrast, degrades gracefully and can reasonably
live in the reference implementation.

## 17. Deferred to implementation

Not design questions. They are recorded so they are not rediscovered, and each
is settled by writing the code rather than by more of this document.

- A payload size above which extraction into the state directory (§6.4) warrants
  a warning, given that a multi-gigabyte payload otherwise sits in
  `%LOCALAPPDATA%` for the life of the session.
- What a policy change mid-session does. §10 puts enforcement in the launch path
  deliberately, so a session on a newly denied extension continues; terminating
  it would discard work in progress.
- Removing the first-run shortcut on Windows where §15 has not made it moot —
  an archive install has no uninstaller, and a shortcut pointing at a deleted
  binary is the stale-ProgID defect `slipcase-desktop` documents, by another
  route.
- Whether the state directory's backup and sync exposure (§6.4) is said to the
  user at open time or only in the administrator documentation.
- Whether Linux moves to `org.gtk.Notifications` with `DBusActivatable=true`.
  §8's amendment records that a notification through
  `org.freedesktop.Notifications` dies with the process that sent it, which
  leaves Linux the odd platform out: a Windows toast reactivates its application
  through the COM activator, and `UNUserNotificationCenter` relaunches a bundle
  the same way, so on both of those a question genuinely does outlive the
  instance. The GNOME mechanism with the same property is `org.gtk.Notifications`
  paired with a D-Bus activatable desktop entry, and taking it would restore the
  parity §9 assumes. What it costs is a second way into the process, which §8
  calls a control surface and which would have to be treated as one; and it is
  GNOME's rather than the freedesktop specification's, so the other desktops
  would keep the current path and the current limit. Worth deciding before
  Phase 4 builds on the assumption, and not before this one ships.
