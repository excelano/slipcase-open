#!/bin/sh
# Install the desktop integration concept 4 and 10 describe: the desktop entry
# and the machine policy file. Optionally the binary alongside them.
#
# The media type is not here. `slipcase-common` declares it once for every
# slipcase product, because two packages cannot ship one path; install that
# first, or the entry below has no type to be associated with.
#
# For a person installing by hand and for testing the association without
# building a package. The Excelano apt repository ships the same files, and the
# two must agree about where things go.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix="${HOME}/.local"
policy=""
binary=""
found_binary=""

usage() {
    cat <<'USAGE'
usage: install.sh [--prefix DIR] [--policy DIR] [--binary PATH] [--no-binary]

  --prefix DIR   where to install (default: ~/.local; use /usr/local for all users)
  --policy DIR   where the machine policy goes (default: none; /etc for a real one)
  --binary PATH  the executable to install into PREFIX/bin
  --no-binary    install the desktop integration only

With neither --binary nor --no-binary, a built executable is looked for under
CARGO_TARGET_DIR and ./target, release before debug, and installed if found.

The policy file is not installed unless asked for. Concept 10 makes it the
highest layer on this platform, and a copy of it under ~/.local would be a
control an administrator did not put there and cannot see.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
        --policy) policy="${2:?--policy needs a directory}"; shift 2 ;;
        --binary) binary="${2:?--binary needs a path}"; shift 2 ;;
        --no-binary) binary="none"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "install.sh: unknown argument $1" >&2; usage >&2; exit 2 ;;
    esac
done

# The executable, where one was not named and one was not refused.
#
# Cargo is asked where its target directory is rather than guessed at, because
# `[build] target-dir` in a Cargo configuration file moves it and no environment
# variable then says so. That is the sibling repository's finding and it holds
# on the same machines.
if [ -z "$binary" ]; then
    target_dir=""
    if command -v cargo >/dev/null 2>&1; then
        target_dir=$(cd "${here}/../.." && cargo metadata --format-version 1 --no-deps 2>/dev/null |
            sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    fi
    [ -n "$target_dir" ] || target_dir="${here}/../../target"

    for candidate in "${target_dir}/release/slipcase-open" "${target_dir}/debug/slipcase-open"
    do
        if [ -x "$candidate" ]; then found_binary="$candidate"; break; fi
    done
elif [ "$binary" != "none" ]; then
    [ -x "$binary" ] || { echo "install.sh: $binary is not an executable" >&2; exit 1; }
    found_binary="$binary"
fi

mkdir -p "${prefix}/share/applications"

install -m 0644 "${here}/slipcase-open.desktop" \
    "${prefix}/share/applications/slipcase-open.desktop"

if [ -n "$policy" ]; then
    mkdir -p "${policy}/slipcase"
    if [ -e "${policy}/slipcase/open.toml" ]; then
        echo "left ${policy}/slipcase/open.toml alone; it is already there"
    else
        install -m 0644 "${here}/open.toml" "${policy}/slipcase/open.toml"
        echo "installed ${policy}/slipcase/open.toml"
    fi
fi

if [ -n "$found_binary" ]; then
    mkdir -p "${prefix}/bin"
    install -m 0755 "$found_binary" "${prefix}/bin/slipcase-open"
    echo "installed ${prefix}/bin/slipcase-open from ${found_binary}"
else
    echo "no executable installed; slipcase-open must be on PATH for the entry to work"
fi

# Absent on a minimal system and the failure is survivable: the files are in
# place either way, and the next login or the next package installation rebuilds
# the cache.
[ -x "$(command -v update-desktop-database || true)" ] &&
    update-desktop-database "${prefix}/share/applications" || true

echo "installed the payload entry under ${prefix}"

# Said rather than assumed. An entry naming a type nothing has declared is an
# entry no file manager will ever offer, and the symptom — double-clicking a
# container and getting the archive tool — looks like an association fight
# rather than a missing package.
# Asked of the compiled database rather than of the filenames in it. Every
# product's declaration is a differently named file and a fourth could arrive;
# `types` is what `update-mime-database` writes and it answers the question that
# matters, which is whether this machine knows the type at all.
if ! grep -qsx 'application/x.slipcase+zip' \
        "${prefix}/share/mime/types" \
        /usr/local/share/mime/types \
        /usr/share/mime/types
then
    echo
    echo "The slipcase media type is not declared on this machine."
    echo "Install slipcase-common, or run its install.sh, or nothing will"
    echo "associate a .slpc with this entry."
fi

echo
echo "check it with:"
echo "  xdg-mime query filetype SOME.slpc     # application/x.slipcase+zip"
echo "  xdg-mime query default application/x.slipcase+zip"
