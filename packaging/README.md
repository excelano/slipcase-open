# Packaging

Concept 4 registers the association, concept 10 puts machine policy under
`/etc`, and concept 15 makes `cargo-deb` and the Excelano apt repository the
Linux channel. This directory is those three, and one platform's worth so far.
`PLAN.md` Phases 4 and 5 add `windows` and `macos` beside it.

## linux

The media type, the desktop entry, and the machine policy file. Install into a
prefix, which defaults to `~/.local`:

    cargo build --release
    ./packaging/linux/install.sh
    ./packaging/linux/install.sh --prefix /usr/local --policy /etc
    ./packaging/linux/uninstall.sh

The policy file is not installed unless `--policy` asks for it. Concept 10 makes
it the highest layer on this platform, and a copy under `~/.local` would be a
control an administrator did not put there and cannot see.

Check that it took:

    xdg-mime query filetype some.slpc                    # application/x.slipcase+zip
    xdg-mime query default application/x.slipcase+zip

An empty file answers `application/x-zerosize` whatever the glob says, so check
against a real container.

### The desktop entry is displayed, and `NoDisplay` would break it

Concept 4 asks that whichever of the two products is not the default stays one
click away rather than becoming unreachable. On Linux that mechanism is the Open
With list, which is built from the applications registered against the media
type — so this entry has to be in it.

`NoDisplay=true` was the plan and is wrong. Measured on this machine: an entry
carrying it answers `false` to `g_app_info_should_show()`, which is the predicate
GIO documents for whether an application belongs in a menu and which the app
choosers filter on. An entry hidden from those lists is an entry concept 4
cannot reach. The cost of leaving it displayed is that "Open payload" also
appears in the applications grid, where launching it with no argument prints the
usage and exits.

### No icon, and what it costs

The icon named for a media type is one file at one path, and two packages cannot
ship the same path — dpkg refuses the second install outright.
`slipcase-desktop` ships it, so this package does not, and its media type
declaration carries no `<icon>` element. Where only this package is installed a
container draws as a plain archive, which is true, since it is one.

The desktop entry falls back to the stock `document-open` for the same reason,
and the notification channel uses that name too. An icon of its own is wanted
and is not blocking. The proper fix when both products ship together is a
`slipcase-common` package owning the type and the icon, which is a change in two
other repositories.

The media type declaration is a second copy of what `slipcase-desktop` ships,
under a different filename. Either product has to work with the other absent,
and shared-mime-info takes the union of every package in the directory, so
installing both is not a conflict and not a redefinition.

## debian

    cargo build --release
    cargo deb --no-build

`Cargo.toml`'s `[package.metadata.deb]` is all of it; there is no
`build-deb.sh`, because this binary links `libc` and `libgcc` and nothing else,
so there is no gap between what the executable needs and what the package
declares. The sibling viewer has such a script and needs one, for the reason
its own packaging notes give.

Check the result with `dpkg-deb -c` and `dpkg-deb -I`. Two things are worth
looking at every time. `/etc/slipcase/open.toml` must appear in `conffiles`, or
dpkg will overwrite an administrator's policy on the next upgrade. And
`usr/bin/slipcase-open` must be there at all: the asset list names it as
`target/release/slipcase-open`, which cargo-deb rewrites to wherever the target
directory really is — a machine with `[build] target-dir` set in a Cargo
configuration file has no environment variable saying where that went.

The package carries no maintainer scripts. `shared-mime-info` and
`desktop-file-utils` own dpkg triggers on the two directories written into here,
so the mime and desktop caches are rebuilt without a `postinst` asking. That is
why both are in `Depends` as much as for anything they provide at run time.

## What the shipped policy file must not do

`packaging/linux/open.toml` documents every key concept 10 defines and sets
none of them, and that is a requirement rather than tidiness. It installs at
`/etc/slipcase/open.toml`, the highest layer on this platform, so an
uncommented line there is a policy nobody wrote being enforced on every machine
that installs the package.
`policy::files::the_policy_file_the_package_ships_says_nothing` is what holds it
to that, and it found a defect the first time it ran — a layer that was present
and empty counted as administered, so shipping the file would have told every
install that its settings were managed when nothing had been.
