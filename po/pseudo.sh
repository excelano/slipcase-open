#!/bin/sh
# Write the pseudolocale catalogue from the template.
#
# A pseudolocale is a catalogue that translates nothing and changes everything:
# every message comes back accented and 40% longer, inside brackets. Run the
# application in it and three defects show themselves at a glance, none of
# which any test in this repository can reach.
#
#   A string still in English is a string that never went through `t`.
#   A sentence with no brackets around it is one the catalogue never saw.
#   A label with its end cut off is a layout built to the width of English.
#
# The third is the one German will actually hit — German runs about a third
# longer than English — and this finds it without anybody reading German, which
# is what makes it worth having before the translation rather than after.
#
# Its tag is `en-x-pseudo`: a BCP-47 private-use subtag rather than gettext's
# `en@pseudo`, because `potext` drops the `@modifier` when it normalises a
# locale name and `en@pseudo` would arrive as plain `en`.
#
#     ./po/pseudo.sh && cargo run -- report.pdf.slpc      # with POTEXT_LANG set
#
# The catalogue is committed and is compiled into debug builds alone, so a
# release carries nothing of it. `src/main.rs` is where that is spelled.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$here/.."

pot=po/slipcase-open.pot
out=po/en-x-pseudo.po

# `msgen` fills every msgstr with its own msgid, which is the English
# catalogue; `msgfilter` then pipes each one through the transform below.
# Both are gettext's own, so this needs nothing the update script does not.
#
# **The filter leaves `{placeholders}` alone.** They are looked up by name at
# run time, and an accented `{réäsön}` would be a placeholder no call site
# fills — the message would come out with braces in it and the defect would
# look like the pseudolocale's rather than the window's.
# **The charset is forced to UTF-8 before the filter and not after it.** Where
# every message is ASCII, `msgen` writes `charset=ASCII` into the catalogue it
# hands on, and `msgfilter` then drops every non-ASCII byte the filter produced:
# the accents and the padding dots vanish and what comes back is the English
# with letters missing — `[chgd c t cm t f th ctr ]` for *unchanged since it
# came out of the container*. It looks like a broken filter and is a declared
# encoding. Measured in slipcase-open on 2026-09-09, the first application here
# whose messages carry no `…` or accent of their own.
msgen "$pot" |
    sed 's/charset=[A-Za-z0-9_-]*/charset=UTF-8/' |
    msgfilter --keep-header --output-file="$out" awk '
{
    line = $0
    out = ""
    while (match(line, /\{[^}]*\}/)) {
        out = out accent(substr(line, 1, RSTART - 1)) substr(line, RSTART, RLENGTH)
        line = substr(line, RSTART + RLENGTH)
    }
    out = out accent(line)
    # Four tenths again in dots: the padding a translation is likely to need,
    # made of a character no message contains, so a clipped label is obvious.
    pad = ""
    for (i = 0; i < int(length($0) * 0.4) + 1; i++) pad = pad "·"
    # `printf` and not `print`: the newline `print` adds becomes part of the
    # message, and `msgfmt` refuses a msgstr that ends in one where the msgid
    # does not.
    printf "%s", "[" out " " pad "]"
}
function accent(s,   n, i, c, r) {
    n = ""
    for (i = 1; i <= length(s); i++) {
        c = substr(s, i, 1)
        r = c
        if (c == "a") r = "ä"; else if (c == "A") r = "Å"
        else if (c == "e") r = "é"; else if (c == "E") r = "É"
        else if (c == "i") r = "í"; else if (c == "I") r = "Î"
        else if (c == "o") r = "ö"; else if (c == "O") r = "Ö"
        else if (c == "u") r = "ü"; else if (c == "U") r = "Ü"
        else if (c == "n") r = "ñ"; else if (c == "s") r = "š"
        n = n r
    }
    return n
}'

# The header `msgen` copies is the template's, which says `charset=CHARSET`.
# `msgfmt` refuses that, and so would anything else that reads the file.
sed -i 's/charset=CHARSET/charset=UTF-8/' "$out"
sed -i 's/^"Language: \\n"/"Language: en-x-pseudo\\n"/' "$out"
sed -i 's/^"Language-Team: LANGUAGE <LL@li.org>\\n"/"Language-Team: none\\n"/' "$out"
sed -i 's/^"Plural-Forms: nplurals=INTEGER; plural=EXPRESSION;\\n"/"Plural-Forms: nplurals=2; plural=(n != 1);\\n"/' "$out"

msgfmt --check --output-file=/dev/null "$out"
printf '%s: ' "$out"
msgfmt --statistics --output-file=/dev/null "$out" 2>&1
