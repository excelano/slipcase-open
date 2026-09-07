# slipcase-open

Double-click a `.slpc`, and the payload opens in whatever application normally
handles it. Edit it there, save, and the edit is written back into the
container. A `.slpc` is a Slipcase container: a ZIP archive holding a payload
together with a TOML document describing it, specified at
<https://slipcaseformat.org>.

No metadata window, no preview, no container browsing. Those are what
[`slipcase-desktop`](https://github.com/excelano/slipcase-desktop) is for, and
the two products claim the same file association deliberately: whichever was
installed last wins, and the other stays one click away in the Open With menu.

## What it does

Opening a container starts a **session**. The payload is extracted into the
user's own state directory, the platform's trust-zone mark is carried onto the
copy, the directory is watched, and the document is handed to the desktop. Every
save that reaches that directory is repacked into the container atomically.

A session lasts as long as the tool is running, because detecting that an
application has finished with a file is not reliable on any of the three
platforms — roughly half of applications hand the file to an instance that is
already running, so there is no process exit to watch, and an editor frequently
does not hold the file open at all. Quitting ends every session, writing back
anything outstanding.

A session that survives a crash is put back where its container has not moved,
on the next launch, and nobody is asked about it. Only a genuine conflict — both
the payload and the container changed since they last agreed — is a question,
because that is the one case the tool cannot settle without knowing which side
somebody meant to keep.

## Using it

    slipcase-open open report.slpc      # start a session and hold it
    slipcase-open sessions              # what is open, and what was left behind
    slipcase-open close 6a94-0          # final repack, then clean up
    slipcase-open recover 6a94-0 --write-back
    slipcase-open recover 6a94-0 --discard
    slipcase-open policy                # which files settings come from here

The first `open` becomes the resident instance; every later invocation hands its
container to that one and exits. On a desktop with a notification service, the
instance reports and asks through notifications carrying buttons. The command
line above is always there underneath.

## Policy

The set of payload extensions that may be opened is configurable by the user and
lockable by an administrator, in this order: machine policy, then user policy,
then user configuration, then the built-in default.

On **Linux** that is a root-owned `/etc/slipcase/open.toml` taking precedence
over `$XDG_CONFIG_HOME/slipcase-open/policy.toml`. The shipped policy file
documents every key and sets none of them.

On **Windows** it is the registry: `SOFTWARE\Policies\Excelano\Slipcase\Open`
under `HKLM` and then `HKCU`, over the user's own
`HKCU\SOFTWARE\Excelano\Slipcase\Open`. The `Policies` subtree is the one
Windows keeps standard users out of and Group Policy clears when a policy is
withdrawn, which is what makes it the administrator's rather than anybody's.
`packaging/windows/policy/` has an ADMX and ADML pair that puts every setting in
the Group Policy editor by name; it is a separate administrator download,
because a package runs no code at install and so cannot place files in
`PolicyDefinitions`.

The setting names and their spellings are the same on both platforms.

`slipcase-open policy` prints both paths as this machine resolves them, what
they add up to, and where the sessions are kept. Every one of those paths comes
out of the environment, so where a file lives and where it lives by default are
two questions and only the running program answers the first.

The same file sets how much the tool says. `notify = "important"` is the
default and keeps warnings, failures, questions, and the answer to anything you
did, while dropping what happens on its own — the first write-back of a session
is announced and the rest are quiet. `notify = "everything"` restores the
per-save notification. Nothing there can silence a question, and your desktop's
own per-application notification settings remain the way to switch the tool off
outright.

The allowlist is a guardrail against user error and social engineering in the
convenient path, and not a security boundary. A container is a plain ZIP and any
user can extract the payload with standard tools. The boundary is application
control — AppLocker, WDAC, and their equivalents.

## Installing

**Linux**, from the Excelano apt repository, or by hand. `slipcase-common`
declares the media type and ships the icon a container is drawn with; it is a
hard dependency, because without the type declared nothing associates a `.slpc`
with this tool.

    ../slipcase-common/install.sh
    cargo build --release
    ./packaging/linux/install.sh                    # into ~/.local
    ./packaging/linux/install.sh --prefix /usr/local --policy /etc

**Windows**, as an MSIX package. `packaging/windows/README.md` has the build and
the decisions behind it; the association, the *Open payload* verb on a
container's context menu, and the command line all come from the package. There
is no window: an icon by the clock takes its colour from whether your work is
where it should be, and its menu names the file.

**macOS** is not built yet. `PLAN.md` has the order and
`slipcase-open-concept.md` has the design.

## Building

    cargo build
    ./check.sh

The suite runs five times, because it watches real directories through the
platform's notifier and event timing moves with load. A watcher test that passed
on its first run has not been tested.

## Licence

MIT. See `LICENSE`.
