# macOS

The application bundle, the launcher inside it, signing, notarization, and
the Homebrew cask. Three files here and everything else is generated:

    Info.plist.in    the bundle's property list, with @VERSION@ substituted
    launcher.swift   the bundle's executable, which receives the double-click
    build-app.sh     assembles, signs, notarizes, and zips "Slipcase Open.app"

Build it, then register it:

    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target $t
    done
    ./packaging/macos/build-app.sh --universal
    lsregister -f "dist/Slipcase Open.app"

`lsregister` is not on `PATH`. It lives at
`/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister`,
and `build-app.sh` prints the full line. `lsregister -u` unregisters, which
matters because `dist/` is ignored by git and a deleted bundle otherwise leaves
a claim behind it. Concept 4's check is then a container and a shell:

    open some.slpc
    "dist/Slipcase Open.app/Contents/MacOS/slipcase-open" sessions

## Not the Mac App Store, and why

Concept 15 made the Store conditional on one question: whether a sandboxed
editor can open a payload extracted inside another application's container.
`slipcase-desktop` answered it on 2026-08-25 — the handover survives — and the
same measurement showed what does not: `Destination::in_place` creates a
sibling of the container and renames it over the original, and the grant a
sandboxed process is given covers the file rather than its directory. The
viewer paid for that with a platform-specific save path.

This tool would pay for more than that, because it is resident and writes
back later. Four things follow from the sandbox for it, the first measured by
the viewer and the other three reasoned from the sandbox's documented shape
rather than measured here:

- `writeback.rs` forks per platform, against concept 12's one engine.
- The grant Launch Services gives for an opened document dies with the
  process, so recovery after a restart (concept 6.3) needs a security-scoped
  bookmark stored in every session record.
- A path in `argv` carries no grant, so the command-line floor concept 9 rests
  on cannot open a container at all.
- `UNUserNotificationCenter` delivers an action to the bundle's own process,
  relaunching it if it has gone, which is never the process holding the
  session.

So the bundle is signed with Developer ID, notarized, and installed by a cask
or by hand, and the engine is untouched: `writeback.rs`, `recover.rs`, and the
command line all do on macOS exactly what they do on Linux. What is given up
is Finder's *Search App Store* for an unopenable `.slpc`, and the viewer on
the Store already answers that search.

## Two executables in one bundle

macOS does not deliver a double-clicked document as an argument. It launches
the bundle with no arguments and sends an Apple Event, and something has to be
listening. The viewer listens from inside its window loop, in the one module
of that crate that writes `unsafe`. This tool has no window loop, so
`launcher.swift` listens instead: it is the bundle's main executable, it
receives the document, runs `slipcase-open open PATH` with the binary beside
it, and terminates. The Rust crate keeps `forbid(unsafe_code)` on this
platform and takes on no AppKit bindings.

**The launcher terminates on purpose.** Launch Services sends a second
double-click to the running application if there is one, and the Rust process
cannot receive it. With the launcher gone, every double-click starts a fresh
one, which hands over through the front door in `endpoint.rs` the way a
second `open` does on every platform. Measured 2026-09-07: two opens of one
container, one instance, one session, and `pgrep` showing the launcher gone
after each.

`LSUIElement` is what keeps that moment invisible: no Dock bounce and no menu
bar for the fraction of a second the launcher lives. Launched with nothing to
open — clicked in Applications — it puts up one alert saying what it is for,
because there is no standing list on this platform for it to raise.

`slipcase-open` sits in `Contents/MacOS` beside the launcher rather than
under `Helpers`, so that the path a person symlinks or types is the obvious
one. The cask's `binary` stanza links it into the Homebrew prefix.

## What was measured, 2026-09-07

Against the assembled bundle, unsigned, registered from `dist/`, through
`open`, which goes through Launch Services and produces the Apple Event Finder
does:

- A container opens: the launcher hands it over and exits, the instance
  stays, TextEdit shows the payload, `sessions` lists it.
- A save writes back: the payload edited in place, the container holding the
  edit within the watcher's next tick.
- A second open of the same container hands over: same instance, one
  session, no second launcher left running.
- `close` ends the session and the instance exits.
- A container carrying `com.apple.quarantine` yields a payload carrying the
  same value, which is `slpc::provenance` doing on macOS what the concept
  table says.
- A container in `~/Downloads` opens and writes back with no prompt, which is
  the platform's per-folder consent not being asked for. Whether that holds
  on a machine where nothing has been granted is not something this machine
  can answer, and a person opening a container there for the first time may be
  asked once.

Not measured: a notarized bundle carrying `com.apple.quarantine` opened by a
person, which is the Gatekeeper path a download takes and needs a download.

## Signing and notarization

    ./packaging/macos/build-app.sh --universal \
        --sign "Developer ID Application: Excelano LLC (9K6W5PMFYP)" \
        --notarize ZFK4K98Q7M ISSUER_ID

`--sign` signs both executables and then the bundle, with the hardened
runtime and a timestamp, and reads the runtime flag back rather than trusting
it: a signature without it assembles perfectly and is refused by the notary
service days later. There are no entitlements, because there is no sandbox and
nothing here needs one.

`--notarize` takes the App Store Connect API key at
`~/.appstoreconnect/private_keys/AuthKey_KEY_ID.p8` and the account's Issuer
ID, which is in App Store Connect under Users and Access, Integrations, and is
deliberately written nowhere in this repository. It submits, waits, staples
the ticket into the bundle, asks Gatekeeper for its verdict, and writes
`dist/slipcase-open-VERSION-macos.zip` holding the bundle and the manual page,
printing the SHA-256 the cask needs.

**Which certificate does what.** A **Developer ID Application** identity signs
what is distributed outside the Store and is the only path that involves
notarization; **Apple Development** signs a bundle that runs here and
nowhere else; **Apple Distribution** is the Store's and is not used by this
product. All three are on the machine this was built on.

## The cask

The tap is `excelano/tap` and the cask is `slipcase-open`:

    brew install --cask excelano/tap/slipcase-open

It installs `Slipcase Open.app` into `/Applications`, which registers the
association without `lsregister`, links `slipcase-open` into the Homebrew
prefix so the command line is on `PATH`, and installs the manual page. The
cask points at the zip attached to the GitHub release for the version, with
its SHA-256, so a release here is: build, notarize, attach the zip, then bump
the cask.

`brew uninstall --cask slipcase-open` removes the application and the links.
`--zap` also removes `~/Library/Application Support/slipcase-open`, which is
where sessions live — and a session there may hold an edit that has not been
written back, so the cask says so rather than removing it quietly.

## Policy

Concept 10 names configuration profiles for macOS and they are not built.
For now macOS takes the file shape Linux has: a root-owned
`/etc/slipcase/open.toml` for machine policy, and
`$XDG_CONFIG_HOME/slipcase-open/policy.toml` — `~/.config` by default — for
the person's own settings. Nothing installs either; an administrator writes
the first by hand or through whatever their management tool uses to place a
file, and `slipcase-open policy` prints both paths as this machine resolves
them. `packaging/linux/open.toml` documents every key and is the file to
start from.

## Presentation

There is no menu bar item and no notification here. Concept 9 keeps the
command line as the floor beneath both and that floor is what ships: a
double-clicked container opens, saves write back silently, and `sessions`
and `recover` say what is open and what was left behind. `PLAN.md` Phase 5
records what a first increment above the floor would be and why it was not
taken before this shipped.
