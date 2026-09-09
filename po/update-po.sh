#!/bin/sh
# Re-read every translatable string out of the source, and bring each
# catalogue up to date with what it found.
#
# Run it after changing any sentence a person reads, and commit what it
# changes. It is a command rather than a build step for the reason the
# conformance corpus is one: it needs `gettext`, which is on the Linux machine
# and on neither of the other two, and a build that quietly skips itself where
# a tool is missing is a build that silently ships last month's German.
#
# **The file list is a glob and never a `POTFILES.in`.** GNOME projects keep
# that list by hand and it goes stale the first time somebody adds a file and
# forgets — the strings in it are then untranslated, the catalogue looks
# complete, and nothing reports it. Comma carries such a list. This does not.
#
# **`xgettext` has no Rust.** Its `--language` list ends at Vala; `.rs` is an
# extension it does not know, so the files are handed to it as C. That reads
# `t("…")` and `tn("…", "…", n)` correctly, keeps `//` comments out, and joins
# adjacent string literals the way C does — which Rust does not do, so a
# message split across two literals would be silently glued into one msgid
# nothing looks up. Write every message as one literal. Raw strings (`r"…"`)
# are the other gap: C reads the `r` as an identifier and the string as a
# string, so a raw string inside `t(…)` extracts wrongly. Neither shape appears
# in this tree.
#
# **It prints a screen of warnings and they are noise.** `unterminated
# character constant` is a Rust lifetime — `&'static str` — read as the start
# of a C character literal, and `unterminated string literal` is an apostrophe
# in a comment. Measured on 2026-09-09 rather than assumed: every string handed
# to `t`, `tc` or `tn` was compared against the msgids this produced and the two
# sets matched exactly — measured in slipcase-desktop, where this script was
# written, and again here. The warnings cost nothing, but
# they are also where a genuine miss would hide, so the way a dropped string is
# found is not by reading them — it is the pseudolocale in `CHECKLIST.md`,
# where anything still in English stands out on sight.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$here/.."

domain=slipcase-open
pot="po/$domain.pot"

# Sorted so that two runs on two machines produce the same file, and `find`
# rather than a shell glob because the sources are two directories deep.
sources=$(find src -name '*.rs' | sort)

# `--keyword` with no argument first, which drops xgettext's built-in C
# keywords: `gettext` and its family are not what this application calls, and
# leaving them in would extract from any function that happened to share a
# name.
#
# `tc:1c,2` says the first argument is the context and the second the message.
# `tn:1,2` says the first two are the singular and the plural.
#
# No version in `Project-Id-Version`: it would be one more place a release has
# to remember, and every catalogue in the repository would show a diff for
# every release that changed nothing anybody translates.
xgettext \
    --language=C \
    --from-code=UTF-8 \
    --keyword \
    --keyword=t \
    --keyword=tc:1c,2 \
    --keyword=tn:1,2 \
    --add-comments=Translators: \
    --sort-by-file \
    --package-name="$domain" \
    --msgid-bugs-address=https://github.com/excelano/slipcase-open/issues \
    --output="$pot" \
    $sources

# xgettext writes `PACKAGE VERSION` into a header it was not given a version
# for, which is a placeholder no reader benefits from.
sed -i "s/^\"Project-Id-Version: $domain VERSION/\"Project-Id-Version: $domain/" "$pot"

# **And it writes `charset=CHARSET`, which is a trap when every message is
# ASCII.** `msginit` reads that placeholder, sees nothing but ASCII in the
# messages, and writes `charset=ASCII` into the new catalogue — after which the
# first German word makes `msgfmt` refuse the file with *invalid multibyte
# sequence*. It does not happen where a source string already carries a
# character like `…`, which is why three catalogues in this fleet were written
# before one met it. Declared here so no catalogue starts life wrong.
sed -i 's/charset=CHARSET/charset=UTF-8/' "$pot"

# Every catalogue beside the template. `msgmerge` is the whole reason this
# project speaks `.po`: where a message's English has changed, it finds the
# entry the new text descended from, carries the old German over, and marks it
# `#, fuzzy` — and `potext` refuses to show a fuzzy entry, so the window falls
# back to English until a person has looked at it. A translation is never
# silently wrong; it is either current or visibly absent.
for catalogue in po/*.po; do
    [ -e "$catalogue" ] || continue
    msgmerge --update --backup=none --previous "$catalogue" "$pot"
    # Syntax is caught here, before a commit, because `potext` cannot report it
    # at run time: a catalogue is compiled into the binary and an application
    # that refuses to start over a stray quote in a translation would be worse
    # than one that shows English.
    msgfmt --check --output-file=/dev/null "$catalogue"
    printf '%s: ' "$catalogue"
    msgfmt --statistics --output-file=/dev/null "$catalogue" 2>&1
done
