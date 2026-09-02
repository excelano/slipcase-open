#!/bin/sh
# Undo install.sh. The machine policy file is left alone unless --policy names
# where it went, and even then only if it has not been edited: concept 10 makes
# it an administrator's, and removing somebody's policy on an uninstall is the
# one deletion this script must not make by accident.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix="${HOME}/.local"
policy=""

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
        --policy) policy="${2:?--policy needs a directory}"; shift 2 ;;
        -h|--help)
            echo "usage: uninstall.sh [--prefix DIR] [--policy DIR]"; exit 0 ;;
        *) echo "uninstall.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done

# The media type is `slipcase-common`'s and is left alone: removing it here
# would take the file type away from every other Slipcase product on the
# machine.
rm -f "${prefix}/share/applications/slipcase-open.desktop" \
      "${prefix}/bin/slipcase-open"

if [ -n "$policy" ] && [ -e "${policy}/slipcase/open.toml" ]; then
    if cmp -s "${here}/open.toml" "${policy}/slipcase/open.toml"; then
        rm -f "${policy}/slipcase/open.toml"
        rmdir "${policy}/slipcase" 2>/dev/null || true
        echo "removed ${policy}/slipcase/open.toml, which was untouched"
    else
        echo "left ${policy}/slipcase/open.toml alone; it has been edited"
    fi
fi

[ -x "$(command -v update-desktop-database || true)" ] &&
    update-desktop-database "${prefix}/share/applications" || true

echo "removed the slipcase-open desktop integration from ${prefix}"
