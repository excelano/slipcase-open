#!/bin/sh
# Everything that must be true before a release is built, asked at once.
#
#     ./packaging/preflight.sh          # everything it can check locally
#     ./packaging/preflight.sh --ci     # and ask GitHub about HEAD
#     ./packaging/preflight.sh --quick  # skip the gate, which is the slow one
#
# It refuses; it does not repair. A check that quietly fixed what it found would
# be a release nobody looked at.
#
# **The gate is run rather than reimplemented.** `check.sh` is what `CLAUDE.md`
# requires before every commit, and a release is a commit with more at stake, so
# this calls it and takes its exit status. Copying its three steps here would
# make two gates that could disagree, and the one that disagreed quietly would
# be this one.
#
# **What this cannot check is the listing.** `packaging/store-listing.md` makes
# claims about behaviour, and every one has to be true of the artefact being
# uploaded. Nothing here can read a sentence and decide whether it is true, so
# that is a person's job and `RELEASE.md` carries the list.
#
# Author: David M. Anderson
# Built with AI assistance (Claude, Anthropic)
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "${here}/.." && pwd)
cd "$root"

ask_ci=no
quick=no
while [ $# -gt 0 ]; do
    case "$1" in
        --ci) ask_ci=yes; shift ;;
        --quick) quick=yes; shift ;;
        -h|--help)
            sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *) echo "preflight.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done

failed=0
say() { printf '  %-46s %s\n' "$1" "$2"; }
ok()   { say "$1" "ok"; }
bad()  { say "$1" "NO - $2"; failed=$((failed + 1)); }

# One parser, and it lives here because there is one consumer of it in this
# repository that is not PowerShell. `build-msix.ps1` reads the same line with
# the same shape; check 4 is what keeps the two from drifting apart in silence.
version=$(sed -n 's/^version *= *"\([0-9][^"]*\)".*/\1/p' Cargo.toml | head -1)
if [ -z "$version" ]; then
    echo "preflight.sh: could not read a version out of Cargo.toml" >&2
    exit 1
fi
echo "slipcase-open ${version}"
echo

# 1. A release is cut from what is committed. A dirty tree means the artefact
#    and the tag describe different code, and nothing afterwards can tell them
#    apart.
if [ -z "$(git status --porcelain)" ]; then
    ok "the tree is clean"
else
    bad "the tree is clean" "$(git status --porcelain | wc -l | tr -d ' ') uncommitted files"
fi

# 2. Same, one step further out: a tag on an unpushed commit points at something
#    nobody else can fetch. A branch with no upstream at all is the same
#    problem and is reported as itself rather than as a pass.
if upstream=$(git rev-parse --abbrev-ref '@{u}' 2>/dev/null); then
    behind=$(git log --oneline '@{u}..' | wc -l | tr -d ' ')
    if [ "$behind" -eq 0 ]; then
        ok "nothing unpushed (${upstream})"
    else
        bad "nothing unpushed" "${behind} commits not on ${upstream}"
    fi
else
    bad "nothing unpushed" "this branch has no upstream"
fi

# 3. The Debian changelog is hand-written and can name a version Cargo does not.
#    Finding out here is cheaper than finding out from a package that installs
#    under the wrong number.
deb_version=$(sed -n '1s/^[^(]*(\([^)]*\)-[0-9]*).*/\1/p' debian/changelog)
if [ "$deb_version" = "$version" ]; then
    ok "debian/changelog names ${version}"
else
    bad "debian/changelog names ${version}" "it names ${deb_version:-nothing}"
fi

# 4. The Store requires four parts with the fourth a zero, and `build-msix.ps1`
#    builds that by appending `.0` to exactly this shape. A version Cargo
#    accepts and that regex does not - a pre-release suffix, most likely - would
#    fail on the machine that packages rather than here.
if printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    ok "the version has an AppxManifest spelling (${version}.0)"
else
    bad "the version has an AppxManifest spelling" "build-msix.ps1 will not read '${version}'"
fi

# 5. **A version is never released twice**, which is this family's rule and the
#    one thing its versioning convention rules out. A tag that already exists
#    means either the number was not moved or somebody is about to overwrite a
#    release.
if git rev-parse --verify --quiet "refs/tags/v${version}" >/dev/null; then
    bad "v${version} is not already tagged" "the tag exists"
else
    ok "v${version} is not already tagged"
fi

# 6. The gate, run rather than reimplemented, with the status taken from it.
if [ "$quick" = yes ]; then
    say "check.sh passes" "skipped - --quick"
elif ./check.sh >/dev/null 2>&1; then
    ok "check.sh passes"
else
    bad "check.sh passes" "it does not - run it and read the output"
fi

# 7. Green on *this* commit, not on some commit. Asked of GitHub because nothing
#    local knows.
if [ "$ask_ci" = yes ]; then
    sha=$(git rev-parse HEAD)
    if ! command -v gh >/dev/null 2>&1; then
        bad "CI is green on HEAD" "no gh to ask with"
    else
        # Counted by `jq` rather than by grepping text, and in three states
        # rather than two: a run still going is not a failure and is not a pass
        # either, because a release cut mid-flight is a release nobody checked.
        counts=$(gh run list --limit 20 --json headSha,status,conclusion \
            --jq "[.[] | select(.headSha == \"${sha}\")]
                  | \"\(length) \(map(select(.status != \"completed\")) | length) \(map(select(.status == \"completed\" and .conclusion != \"success\")) | length)\"" \
            2>/dev/null) || counts=""
        set -- ${counts:-0 0 0}
        total=$1 running=$2 red=$3
        if [ "$total" -eq 0 ]; then
            bad "CI is green on HEAD" "no runs for ${sha}"
        elif [ "$red" -gt 0 ]; then
            bad "CI is green on HEAD" "${red} of ${total} failed"
        elif [ "$running" -gt 0 ]; then
            bad "CI is green on HEAD" "${running} of ${total} still running"
        else
            ok "CI is green on HEAD (${total} runs)"
        fi
    fi
else
    say "CI is green on HEAD" "skipped - pass --ci"
fi

echo
if [ "$failed" -eq 0 ]; then
    echo "Nothing local is stopping a release of ${version}."
    echo "The listing's claims and the certification kit are still a person's job;"
    echo "RELEASE.md has both."
else
    echo "${failed} check(s) failed. Nothing was changed."
    exit 1
fi
