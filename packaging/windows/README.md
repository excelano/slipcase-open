# Windows packaging

What is here, how a package is built, and the things that were measured rather
than assumed. `PLAN.md` Phase 4 is the history; this is what the next build
needs to know.

    AppxManifest.xml.in    the manifest, with the identity templated out
    build-msix.ps1         builds the binary, stages, packages, optionally signs
    check-imports.ps1      refuses a DLL that does not ship with Windows
    make-ico.ps1           renders the five tray icons from the one drawing
    identity.psd1          what Partner Center assigned. NOT COMMITTED
    identity.psd1.example  the template, which says what goes in it
    assets/                tile and file-type logos, every scale
    listing/               the Store logo, which is not in the package
    policy/                the ADMX and ADML pair, which is not in the package

## Building one

    cargo build --release
    powershell -File packaging\windows\check-imports.ps1
    powershell -File packaging\windows\build-msix.ps1 -SelfSign

Two packages come out of one staging tree. The unsigned one is what goes to the
Store, because the Store signs what it distributes; the throwaway-signed one is
for installing here. **They are never built twice.** A rebuild of identical
source produces a different file — the COFF timestamp, the debug directory
timestamps and the PDB GUID — so the artefact uploaded has to be the one that
was tested rather than a fresh build of the same commit.

`build-msix.ps1` builds before it packages, and that is not a convenience. It
did not, until 2026-09-03, and it shipped a package containing a binary eight
hours stale whose executable-payload refusal was simply absent. The check that
was supposed to catch it compared the staged binary against the installed one,
which is two copies of the same stale file agreeing with each other. **A claim
that a package contains a change is traced to the source, not to another copy of
the artefact**; the cheap decisive test is to grep the staged binary for a string
only the new code has.

Installing the signed copy needs one administrator action, which the script
prints and does not attempt: the throwaway certificate has to reach
`LocalMachine\TrustedPeople`. The per-user store is not consulted for this and
importing there leaves deployment failing `0x800B0109`.

## What was measured

### MSIX redirects `%LOCALAPPDATA%`, and the process is not told

**This is the most expensive thing in this directory.** A packaged process that
asks for `%LOCALAPPDATA%` is given
`…\Packages\<family>\LocalCache\Local` instead, and the read view is *merged*, so
it also sees whatever is in the real location and cannot tell the two apart. A
redirection layer can tombstone a file belonging to the layer beneath it and
cannot remove a directory there.

That produced a defect hunted for two days as something else: session
directories that survived their own removal, leaving an empty `payload/` and a
record, so the session list and the tray showed a corpse. It was permanent, not
transient, and no process held anything — the same executable, byte for byte,
with the same package identity, removes the directory when it runs from the
staging tree and never removes it when it runs from the install location.

`session::platform_base` asks Windows where the package keeps its data rather
than asking for a variable it will not be given. `LocalCacheFolder` rather than
`LocalFolder`, because the cache is the store Windows neither roams nor puts in
a device backup, which is what a copy of somebody's payload should be.

**The lesson generalises past this defect.** The harness that failed to
reproduce it for twenty rounds was driving the unpackaged binary. Anything
measured about this product has to go through the shipped artefact, because the
container is part of the product.

### `IAttachmentExecute` is not what reads Mark of the Web

Measured 2026-09-01 by asking `CheckPolicy` about ten files, marked and
unmarked, across five extensions, reading the raw `HRESULT` rather than the
`Result` the bindings collapse it into: the marked and unmarked answers are
identical in every case, and what moves them is `SetSource` and the extension.
That interface is for a client which has *received* an attachment and is
deciding whether to save and run it, so the zone comes from the source it is
told about rather than from the file.

What does consult the mark is `ShellExecuteEx`. So the launcher is that call
with the two flags that would switch the check off — `SEE_MASK_NOZONECHECKS` and
`SEE_MASK_FLAG_NO_UI` — deliberately absent. **A requirement that is a negative
is weaker than one that is a call**, so the mask is a named constant with a test
over it, in `platform::shell`.

### "Last installed wins" is not what Windows does

With the viewer and this product both installed as packages and no `UserChoice`
set, a double-click raises *How do you want to open this file?* with **Keep using
this app: Slipcase** above and **Slipcase Open** below it, badged New. Windows
keeps the incumbent and offers the newcomer; it does not switch.

What survives from concept §4 is the consequence — each product registers a
secondary verb, so the one which is not the default stays reachable. *Open
payload* is this product's, and without it a machine where the viewer won would
have no route to this one from Explorer at all.

A practical consequence for anything scripted here: a plain double-click on a
machine with both installed raises a picker, which cannot be automated. Naming
the verb reaches the same command, because the manifest gives it the path alone,
which is what the default activation passes too.

### The console subsystem cannot ship

The binary was `WINDOWS_CUI`, so a packaged double-click raised a full-size
console window titled with its path under `WindowsApps`, and it stayed for as
long as the session did rather than flashing. It is the windows subsystem now,
with the console attached back when a terminal started it — which is also what
keeps `slipcase-open open` at a prompt behaving like a command.

### The execution alias is not optional

The binary under `WindowsApps` is ACL'd against being run directly, so without
`windows.appExecutionAlias` every verb the command line offers would be
unreachable from the installed product. Concept §9 keeps the command line a
shipped interface rather than a test harness, and this is what makes that true
of the packaged build.

While editing the manifest: **a double hyphen cannot appear in an XML comment**,
which is what `makeappx` refused the file for first.

### A message box on a thread dies with its process

The refusal is a `MessageBoxW` on a thread of its own, because the caller can be
the resident loop and a modal loop there would stop every open session's watcher
for as long as somebody left the box up. An invocation that refused and held
nothing then returned immediately and took the thread down with it.

Measured 2026-09-06 against the installed package, opening the same container
through the same shell verb twice:

| With | Result |
| --- | --- |
| nothing of this tool running | no box, no window, process gone inside 400 ms |
| an instance already running | box on screen inside 400 ms, every time |

Which is why it looked like the first double-click after a restart was the one
that failed: it was the only one where the process that refused was also the one
that would have had to stay. `Channel::stay_until_seen` is the fix — the box
still goes on a thread, and the process now joins it on its way out. The front
door is dropped first, so a refusal somebody leaves on screen does not hold a
bound pipe that nothing is accepting on; checked by opening an ordinary
container while the box was up.

This is the same shape as the zone warning below, and it is worth saying why one
of them is now measured and the other still is not: nothing automated can watch
a modal dialog's *contents*, but whether a window belonging to this process
exists at all is `MainWindowHandle`, and that is a fact a script can have.

### Two ways of asking "is the tray up", one of which lies

The tray's window is an ordinary top-level window of class `slipcase-open-tray`,
invisible (`WINDOW_STYLE(0)`, no parent). **`FindWindowW` returns 0 for it and
`EnumWindows` finds it**, measured 2026-09-06 on the same running process in the
same second. Several rounds of "no tray icon" here were that broken probe and
not a finding. Enumerate and match on the owning process id — the same
correction `screenshot.ps1` already carries for a different reason.

Two further notes for anything scripted against the tray. An instance killed
with `Stop-Process -Force` never runs the tray's `Drop`, so
`Shell_NotifyIcon(NIM_DELETE)` is skipped and the dead icon stays in the
notification area until something hovers over it — a sweep of tests leaves a row
of them, and they are debris rather than a defect. And `Shell.Application`'s
verb launched from a console host puts a console in the process's ancestry;
`wscript.exe` running a two-line `.vbs` is a launcher with none, which is what
Explorer looks like.

### `is_terminal` is not "a command line started this"

`std::io::stderr().is_terminal()` is false for a **pipe**, so a shell that
redirects — and this crate's own process tests, which spawn with `Stdio::piped`
— answers no to a question meant to distinguish a shell from a double-click.
Measured 2026-09-06 when the standing list began gating whether a process stays:
a redirected refusal was given a tray icon and stayed for ever, hanging two
integration tests.

What separates them is whether the process was handed standard handles at all.
Explorer gives a windows-subsystem process none, `attach_console` has already
joined the parent's console where there was one, and a redirect still counts as
a shell — which is the intent concept §9 states and `is_terminal` only
approximates. `Voice` keeps `is_terminal`, because *is somebody reading these
lines* is a different question that a redirect really does change.

## Policy is a separate download

`policy/` holds `SlipcaseOpen.admx` and `en-US\SlipcaseOpen.adml`. They are not
in the package and cannot be: an MSIX runs no code at install, so it cannot
place files in `PolicyDefinitions`. That is how Microsoft ships its own.

    %SystemRoot%\PolicyDefinitions\                        one machine
    \\<domain>\SYSVOL\<domain>\Policies\PolicyDefinitions\  a central store

`src/policy/registry.rs` reads the keys they write. The two agree by test rather
than by care: `the_admx_writes_the_keys_and_values_this_module_reads` reads the
ADMX and fails if a value name appears in one and not the other. That pair is
worth a test because nothing at run time consults an ADMX — a renamed value
produces a policy that appears applied in the Group Policy editor and governs
nothing, with no error anywhere.

**The `Policies` subtree keeps standard users out under `HKCU` as well as
`HKLM`**, which is measured: writing it from an ordinary account is refused. That
is the property concept §10 relies on, and it means the two policy layers cannot
be exercised without elevation — the user's own key is what the unelevated
checks use, and every layer goes through the same reader.

## Still to run by hand

Two things nothing automated can do, both needing a person at this desktop:

- **The zone warning.** A marked container's payload carries `ZoneId=3` onto the
  extracted copy, and `ShellExecuteEx` is called with the check left on, but
  that the warning is *displayed* has never been watched. Nothing automated can
  watch a modal dialog, and the suite never reaches the launcher on any
  platform.
- **The ADMX in the Group Policy editor.** The pair is well formed and every
  reference in it resolves, which is checked; that `gpedit.msc` renders it and
  writes the values expected needs the files in `PolicyDefinitions`, which needs
  administrator.
