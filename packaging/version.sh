#!/bin/sh
# The version, in whichever spelling the caller needs.
#
# `Cargo.toml` holds the number and nothing else should. Three
# artefacts want it in three shapes and two of them cannot be the plain string,
# so one parser here rather than a `sed` in every build script. The reasoning
# is slipcase-desktop's and so is the shape.
#
#   ./packaging/version.sh              0.1.0     Cargo's own, and Debian's
#   ./packaging/version.sh --appx       0.1.0.0   four parts, the Store requires
#                                                 the fourth to be 0
#   ./packaging/version.sh --short      0.1.0     CFBundleShortVersionString,
#                                                 which is what a person sees
#   ./packaging/version.sh --build      1234      CFBundleVersion, which must
#                                                 increase on every upload
#
# `--build` is the only value here that is not a function of the release
# version: two uploads of 0.1.0 need different build numbers, because App Store
# Connect refuses a second upload carrying a build number it has seen,
# including a rejected one resubmitted unchanged. The first-parent commit count
# is monotonic without anybody having to remember, and it points back at a
# commit afterwards.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "${here}/.." && pwd)

usage() {
    cat <<'USAGE'
usage: version.sh [--appx | --short | --build]

  (no argument)  the version as Cargo.toml holds it        0.1.0
  --appx         four parts, fourth always 0               0.1.0.0
  --short        CFBundleShortVersionString                0.1.0
  --build        CFBundleVersion, monotonic                the commit count
USAGE
}

# The first `version =` at the start of a line, which is `[package]`'s;
# `rust-version` does not start a line with the word.
version=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "${root}/Cargo.toml" | head -1)
[ -n "$version" ] || {
    echo "version.sh: no version in ${root}/Cargo.toml" >&2
    exit 1
}

# Refused rather than mangled. A pre-release like `0.1.0-rc1` is a perfectly
# good Cargo version and there is no four-part number to make of it, so the
# numeric spellings say so instead of silently shipping `0.1.0.0` for
# something that is not 0.1.0.
numeric() {
    case "$1" in
        *[!0-9.]* | *..* | .* | *.)
            echo "version.sh: ${1} is not three numeric parts, so there is no ${2} spelling of it" >&2
            exit 1
            ;;
    esac
    [ "$(echo "$1" | tr -cd . | wc -c)" -eq 2 ] || {
        echo "version.sh: ${1} is not three parts, so there is no ${2} spelling of it" >&2
        exit 1
    }
}

case "${1:-}" in
    "")        printf '%s\n' "$version" ;;
    --short)   printf '%s\n' "$version" ;;
    --appx)
        numeric "$version" "AppxManifest"
        printf '%s.0\n' "$version"
        ;;
    --build)
        # `--first-parent` so a merge does not add every commit of the branch
        # it brought in, which would make the number jump by an amount nobody
        # can account for.
        count=$(cd "$root" && git rev-list --count --first-parent HEAD 2>/dev/null) || {
            echo "version.sh: --build needs a git checkout; this is not one" >&2
            exit 1
        }
        printf '%s\n' "$count"
        ;;
    -h|--help) usage ;;
    *)
        echo "version.sh: unknown argument $1" >&2
        usage >&2
        exit 2
        ;;
esac
