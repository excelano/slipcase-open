# Packaging

Concept 4 registers the association, concept 10 puts machine policy under
`/etc`, and concept 15 makes `cargo-deb` and the Excelano apt repository the
Linux channel. This directory is those three, and one platform's worth so far.
`PLAN.md` Phases 4 and 5 add `windows` and `macos` beside it.

## linux

The desktop entry and the machine policy file. The media type is
`slipcase-common`'s, so install that first. Into a prefix, which defaults to
`~/.local`:

    ../slipcase-common/install.sh
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

### The media type is not here

`slipcase-common` declares `application/x.slipcase+zip` and ships the icon a
container is drawn with, and both products depend on it. Two packages cannot
ship one path — dpkg refuses the second install — so the type and the icon
belong to neither product and are declared once.

That package's README carries the two measurements behind it: `sub-class-of
application/zip` carries no icon, so a type declaring none draws as a blank
generic document rather than as an archive; and the icon has to be named as the
generic icon as well as the icon, because GTK4 searches theme-major and Adwaita
answers `application-x-generic` before hicolor is reached.

`install.sh` therefore installs the desktop entry and not the type, and says so
when the machine does not have the type declared — asked of `share/mime/types`,
the file `update-mime-database` writes, rather than of the filenames in
`packages/`, since every product's declaration has a different name. An entry
naming a type nothing has declared is an entry no file manager will offer, and
the symptom looks like an association fight rather than a missing package.

The desktop entry still falls back to the stock `document-open` icon, and the
notification channel uses that name too. An application icon of its own is
wanted here and is a different thing from the file-type icon above, which is why
it did not come with it.

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
