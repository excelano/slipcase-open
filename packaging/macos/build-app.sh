#!/bin/sh
# Assemble `Slipcase Open.app`: the launcher that receives the document Finder
# sends, the `slipcase-open` binary it hands that document to, the property
# list that claims the type, and the icon. Signs it for Developer ID when asked,
# and notarizes and staples it when asked for that too.
#
# The bundle is the unit of everything on macOS. A bare executable has no
# bundle identifier, Launch Services files it as a nameless process, and
# nothing can be associated with it. `README.md` beside this file says why the
# bundle has two executables in it, and why it is not on the Mac App Store.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "${here}/../.." && pwd)
binary=""
outdir="${root}/dist"
# Not on PATH, and README.md says so.
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
universal=no
identity=""
key_id=""
issuer=""
# The floor Info.plist.in declares, checked against every executable below.
floor=11.0

usage() {
    cat <<'USAGE'
usage: build-app.sh [--binary PATH] [--outdir DIR] [--universal] [--sign ID]
                    [--notarize KEY_ID ISSUER]

  --binary PATH  the slipcase-open executable to bundle (default: the release
                 build for this machine's architecture)
  --outdir DIR   where to write "Slipcase Open.app" (default: ./dist)
  --universal    join the two per-architecture release builds with lipo, so
                 one bundle runs on Apple silicon and Intel:

                   for t in aarch64-apple-darwin x86_64-apple-darwin; do
                     MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target $t
                   done
                   ./packaging/macos/build-app.sh --universal
  --sign ID      sign both executables and the bundle with this identity, with
                 the hardened runtime notarization requires. `security
                 find-identity -v -p codesigning` lists what this machine
                 holds; a release is signed with "Developer ID Application".
  --notarize KEY_ID ISSUER
                 submit the signed bundle to Apple's notary service with the
                 App Store Connect API key at
                 ~/.appstoreconnect/private_keys/AuthKey_KEY_ID.p8, wait for
                 the verdict, staple it, and write the zip a cask downloads:
                 slipcase-open-VERSION-macos.zip, holding the bundle and the
                 manual page. ISSUER is the Issuer ID from App Store Connect,
                 Users and Access, Integrations. Implies --universal and
                 needs --sign.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --binary) binary="${2:?--binary needs a path}"; shift 2 ;;
        --outdir) outdir="${2:?--outdir needs a directory}"; shift 2 ;;
        --universal) universal=yes; shift ;;
        --sign) identity="${2:?--sign needs an identity}"; shift 2 ;;
        --notarize)
            key_id="${2:?--notarize needs a key id}"
            issuer="${3:?--notarize needs an issuer id}"
            shift 3
            ;;
        -h|--help) usage; exit 0 ;;
        *) echo "build-app.sh: unknown argument $1" >&2; usage >&2; exit 2 ;;
    esac
done

stage=""
cleanup() { [ -z "$stage" ] || rm -rf "$stage"; }
trap cleanup EXIT INT TERM
stage=$(mktemp -d)

# Everything --notarize needs is checked before anything is built, because a
# submission is a queue and finding out afterwards costs a wait.
key=""
if [ -n "$key_id" ]; then
    [ -n "$identity" ] || {
        echo "build-app.sh: --notarize needs --sign; an unsigned bundle is refused" >&2
        exit 2
    }
    universal=yes
    key="${HOME}/.appstoreconnect/private_keys/AuthKey_${key_id}.p8"
    [ -f "$key" ] || {
        echo "build-app.sh: no API key at ${key}" >&2
        exit 1
    }
fi

# Cargo is asked where its target directory is. `[build] target-dir` in a
# Cargo configuration file moves it and no environment variable then says so.
target_dir=$(cd "$root" && cargo metadata --format-version 1 --no-deps |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')

# The same line `preflight.sh` reads, with the same shape.
version=$(sed -n 's/^version *= *"\([0-9][^"]*\)".*/\1/p' "${root}/Cargo.toml" | head -1)
[ -n "$version" ] || {
    echo "build-app.sh: could not read a version out of Cargo.toml" >&2
    exit 1
}

# A released bundle runs on both architectures, and `cargo build --release`
# writes one. Asking for a target explicitly writes to `<triple>/release/`, so
# the two slices are built separately and joined here; nothing is compiled by
# this script.
if [ "$universal" = yes ]; then
    [ -z "$binary" ] || {
        echo "build-app.sh: --universal builds its own binary; drop --binary" >&2
        exit 2
    }
    slices=""
    for triple in x86_64-apple-darwin aarch64-apple-darwin; do
        slice="${target_dir}/${triple}/release/slipcase-open"
        [ -x "$slice" ] || {
            echo "build-app.sh: no executable at $slice — run 'MACOSX_DEPLOYMENT_TARGET=${floor} cargo build --release --target ${triple}' first" >&2
            exit 1
        }
        slices="${slices} ${slice}"
    done
    binary="${stage}/slipcase-open"
    # shellcheck disable=SC2086
    lipo -create ${slices} -output "$binary"
    swift_targets="arm64-apple-macos${floor} x86_64-apple-macos${floor}"
else
    [ -n "$binary" ] || binary="${target_dir}/release/slipcase-open"
    case "$(uname -m)" in
        arm64) swift_targets="arm64-apple-macos${floor}" ;;
        x86_64) swift_targets="x86_64-apple-macos${floor}" ;;
        *) echo "build-app.sh: unknown architecture $(uname -m)" >&2; exit 1 ;;
    esac
fi
[ -x "$binary" ] || {
    echo "build-app.sh: no executable at $binary — run 'cargo build --release' first" >&2
    exit 1
}

# The launcher, one slice per architecture the binary has, joined the same way.
# `swiftc` rather than a Swift package: it is one file with one job, and a
# package would be a second build system in the repository for it.
launcher_slices=""
for target in $swift_targets; do
    out="${stage}/launcher-${target%%-*}"
    swiftc -O -target "$target" "${here}/launcher.swift" -o "$out"
    launcher_slices="${launcher_slices} ${out}"
done
launcher="${stage}/Slipcase Open"
# shellcheck disable=SC2086
lipo -create ${launcher_slices} -output "$launcher"

app="${outdir}/Slipcase Open.app"
rm -rf "$app"
mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"

# The icon. There is no SVG in this repository: the Windows assets were
# rendered from the family's one drawing, and the 1024-pixel tile is the
# largest of them, so it is the source here and `sips` resamples it down to
# the ten sizes `iconutil` wants. Downsampling a 1024-pixel raster is a
# different thing from upsampling a 64-pixel one, which is the trap the
# viewer's script records.
source_png="${root}/packaging/windows/assets/slipcase.scale-400.png"
[ -f "$source_png" ] || { echo "build-app.sh: no icon source at $source_png" >&2; exit 1; }
iconset="${stage}/slipcase-open.iconset"
mkdir -p "$iconset"
for pair in 16:16x16 32:16x16@2x 32:32x32 64:32x32@2x \
            128:128x128 256:128x128@2x 256:256x256 512:256x256@2x \
            512:512x512 1024:512x512@2x
do
    size=${pair%%:*}
    out="${iconset}/icon_${pair#*:}.png"
    sips -z "$size" "$size" "$source_png" --out "$out" >/dev/null 2>&1
    got=$(sips -g pixelWidth "$out" | sed -n 's/.*pixelWidth: *//p')
    [ "$got" = "$size" ] || {
        echo "build-app.sh: asked sips for ${size}px and got ${got}px" >&2
        exit 1
    }
done
iconutil --convert icns "$iconset" --output "${app}/Contents/Resources/slipcase-open.icns"

sed -e "s/@VERSION@/${version}/g" "${here}/Info.plist.in" > "${app}/Contents/Info.plist"
# A malformed property list is not an error Finder reports; it is a bundle that
# quietly does not associate. Parsed here so the failure is loud.
plutil -lint "${app}/Contents/Info.plist" >/dev/null

install -m 0755 "$launcher" "${app}/Contents/MacOS/Slipcase Open"
install -m 0755 "$binary" "${app}/Contents/MacOS/slipcase-open"

# Every executable has to agree with the floor the property list declares, and
# Cargo's default does not: without `MACOSX_DEPLOYMENT_TARGET` the x86_64 slice
# says 10.12. Finder would refuse to launch the bundle below the floor while
# the binary claimed to run there. Two load-command shapes are read so this
# cannot pass by finding neither.
declared=$(plutil -extract LSMinimumSystemVersion raw "${app}/Contents/Info.plist")
[ "$declared" = "$floor" ] || {
    echo "build-app.sh: Info.plist.in declares ${declared} and this script checks for ${floor}" >&2
    exit 1
}
for exe in "${app}/Contents/MacOS/Slipcase Open" "${app}/Contents/MacOS/slipcase-open"; do
    for arch in $(lipo -archs "$exe"); do
        got=$(otool -arch "$arch" -l "$exe" |
            awk '/LC_BUILD_VERSION|LC_VERSION_MIN_MACOSX/ {want=1; next}
                 want && ($1 == "minos" || $1 == "version") {print $2; exit}')
        [ "$got" = "$floor" ] || {
            echo "build-app.sh: the ${arch} slice of $(basename "$exe") was built for ${got:-nothing} and Info.plist declares ${floor} — rebuild with MACOSX_DEPLOYMENT_TARGET=${floor}" >&2
            exit 1
        }
    done
done

# Last, so that nothing this script writes lands inside the bundle after it has
# been sealed. Inner first: the launcher is the bundle's main executable and is
# signed with it, but `slipcase-open` beside it is a second program, and a seal
# over the bundle covers it only once it carries a signature of its own.
#
# `--options runtime` is the hardened runtime, which the notary service
# refuses a submission without. `--timestamp` reaches Apple's timestamp server
# and is what lets the signature outlive the certificate.
if [ -n "$identity" ]; then
    codesign --force --timestamp --options runtime --sign "$identity" \
        "${app}/Contents/MacOS/slipcase-open"
    codesign --force --timestamp --options runtime --sign "$identity" "$app"
    codesign --verify --deep --strict "$app" || {
        echo "build-app.sh: the signed bundle does not verify" >&2
        exit 1
    }
    # Read back rather than trusted: a signature without the hardened runtime
    # assembles perfectly and is refused days later by the notary service.
    for exe in "${app}/Contents/MacOS/Slipcase Open" "${app}/Contents/MacOS/slipcase-open"; do
        codesign -d -vv "$exe" 2>&1 | grep -q 'flags=.*runtime' || {
            echo "build-app.sh: $(basename "$exe") was signed without the hardened runtime" >&2
            exit 1
        }
    done
    echo "signed ${app} with ${identity}"
fi

if [ -n "$key_id" ]; then
    # `ditto` rather than `zip`: it is what Apple documents for a bundle, and it
    # keeps the extended attributes and symlinks a signature covers.
    submission="${stage}/submission.zip"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$submission"
    echo "submitting to the notary service"
    xcrun notarytool submit "$submission" \
        --key "$key" --key-id "$key_id" --issuer "$issuer" --wait \
        | tee "${stage}/notary.log"
    grep -q 'status: Accepted' "${stage}/notary.log" || {
        id=$(sed -n 's/^ *id: //p' "${stage}/notary.log" | head -1)
        echo "build-app.sh: not accepted; read the log with:" >&2
        echo "  xcrun notarytool log ${id} --key ${key} --key-id ${key_id} --issuer ${issuer}" >&2
        exit 1
    }
    # The ticket goes into the bundle, so a Mac that is offline when the bundle
    # is first opened can still verify it.
    xcrun stapler staple "$app" >/dev/null
    xcrun stapler validate "$app" >/dev/null || {
        echo "build-app.sh: the stapled ticket does not validate" >&2
        exit 1
    }
    # Gatekeeper's own verdict on what will be shipped, which is the only one
    # that matters to a person opening a download.
    verdict=$(spctl -a -vv "$app" 2>&1)
    printf '%s\n' "$verdict" | grep -q 'source=Notarized Developer ID' || {
        echo "build-app.sh: Gatekeeper does not accept the bundle:" >&2
        printf '%s\n' "$verdict" | sed 's/^/  /' >&2
        exit 1
    }

    # What the cask downloads: the bundle and the manual page, side by side.
    shipping="${stage}/shipping"
    mkdir -p "$shipping"
    ditto "$app" "${shipping}/Slipcase Open.app"
    cp "${root}/packaging/man/slipcase-open.1" "$shipping"
    zip="${outdir}/slipcase-open-${version}-macos.zip"
    rm -f "$zip"
    ditto -c -k --sequesterRsrc "$shipping" "$zip"
    echo "notarized and stapled ${app}"
    echo "wrote ${zip}"
    echo "  sha256 $(shasum -a 256 "$zip" | cut -d' ' -f1)"
    exit 0
fi

echo "built ${app} from ${binary}"
echo
echo "register it and check that it took:"
echo "  ${lsregister} -f \"${app}\""
echo "  open SOME.slpc"
echo "  \"${app}/Contents/MacOS/slipcase-open\" sessions"
