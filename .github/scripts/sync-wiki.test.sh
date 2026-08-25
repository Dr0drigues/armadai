#!/usr/bin/env bash
#
# Tests for sync-wiki.sh. Run from the repository root:
#   .github/scripts/sync-wiki.test.sh
#
# Each case asserts against a real sync into a temporary wiki working copy,
# using the repository's own docs/wiki/ as input, so the tests fail when a
# page is added that the sync would mishandle.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYNC="$SCRIPT_DIR/sync-wiki.sh"
SRC="$SCRIPT_DIR/../../docs/wiki"

fails=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n     %s\n' "$1" "$2"; fails=$((fails + 1)); }

WIKI=$(mktemp -d)
trap 'rm -rf "$WIKI"' EXIT
"$SYNC" "$SRC" "$WIKI" >/dev/null || { echo "sync failed outright"; exit 1; }

# mdBook's table of contents drives the book sidebar and is meaningless as a
# wiki page — the wiki generates its own Home.
if [ -e "$WIKI/SUMMARY.md" ]; then
    fail "SUMMARY.md is excluded" "SUMMARY.md was published to the wiki"
else
    pass "SUMMARY.md is excluded"
fi

# Every source page except the excluded one reaches the wiki under its
# Title-Case name.
missing=""
for src in "$SRC"/*.md; do
    filename=$(basename "$src")
    [ "$filename" = "SUMMARY.md" ] && continue
    expected=$(printf '%s' "${filename%.md}" | awk -F- '{for(i=1;i<=NF;i++){printf "%s%s%s", (i>1?"-":""), toupper(substr($i,1,1)), substr($i,2)}}')
    [ -e "$WIKI/$expected.md" ] || missing="$missing ${filename} -> ${expected}.md"
done
[ -z "$missing" ] && pass "every page is published under its Title-Case name" \
    || fail "every page is published under its Title-Case name" "missing:$missing"

# The regression this script exists for: links must be rewritten for ALL
# pages, not a frozen subset. A link left as `page.md` 404s on the wiki.
# The `(#anchor)?` matters: an anchored link looks rewritten right up to the
# `#`, so a pattern anchored on `)` walks straight past it.
leftover=$(grep -roE '\((\./)?[a-z0-9][a-z0-9-]*\.md(#[^)]*)?\)' "$WIKI" 2>/dev/null | grep -v 'Home.md:' || true)
[ -z "$leftover" ] && pass "no link still points at a .md source name" \
    || fail "no link still points at a .md source name" "$(printf '%s' "$leftover" | head -5)"

# Specifically the pages added after the old hand-maintained link table froze
# — the ones whose links shipped to the wiki broken. Each assertion states how
# many source links it actually covers: a page nothing links to yet cannot
# prove anything today, and saying so keeps it from reading as evidence it
# isn't. Those cases become real assertions the moment a link appears.
for page in audit declarative-agents migration-v0-to-v1 orchestration-guide policy-gate; do
    in_source=$(grep -roE "\((\./)?${page}\.md\)" "$SRC" 2>/dev/null | grep -v '/SUMMARY.md:' | wc -l | tr -d ' ')
    still_raw=$(grep -rlE "\((\./)?${page}\.md\)" "$WIKI" 2>/dev/null || true)
    if [ "$in_source" = "0" ]; then
        printf '  --   links to %s: nothing links to it yet, nothing to prove\n' "$page"
    elif [ -z "$still_raw" ]; then
        pass "links to $page are rewritten ($in_source in source)"
    else
        fail "links to $page are rewritten" "still raw in: $still_raw"
    fi
done

# Stopping the copy does not un-publish: a SUMMARY.md left on the wiki by an
# earlier run has to be actively removed.
WIKI2=$(mktemp -d)
printf '# stale\n' > "$WIKI2/SUMMARY.md"
printf '# hand-written\n' > "$WIKI2/Hand-Written-Page.md"
"$SYNC" "$SRC" "$WIKI2" >/dev/null
if [ -e "$WIKI2/SUMMARY.md" ]; then
    fail "a previously published SUMMARY.md is retired" "it survived the sync"
else
    pass "a previously published SUMMARY.md is retired"
fi
# ...but nothing else is: over-deletion here is the same failure the link
# manifest exists to prevent.
if [ -e "$WIKI2/Hand-Written-Page.md" ]; then
    pass "a hand-written wiki page is left alone"
else
    fail "a hand-written wiki page is left alone" "the sync deleted it"
fi
rm -rf "$WIKI2"

# The `./`-prefixed and anchored forms do not occur in docs/wiki/ today, so
# the real corpus cannot exercise them: without a synthetic source, half the
# rewrite could be deleted and the suite would stay green. Verified: it does.
SYNTH=$(mktemp -d)
SYNTH_WIKI=$(mktemp -d)
cat > "$SYNTH/alpha-page.md" <<'ALPHA'
# Alpha
bare [b](beta-page.md)
dot-slash [b](./beta-page.md)
anchored [b](beta-page.md#a-section)
dot-slash anchored [b](./beta-page.md#other)
ALPHA
printf '# Beta
' > "$SYNTH/beta-page.md"
"$SYNC" "$SYNTH" "$SYNTH_WIKI" >/dev/null
synth_out=$(cat "$SYNTH_WIKI/Alpha-Page.md")
for want in '(Beta-Page)' '(Beta-Page#a-section)' '(Beta-Page#other)'; do
    case "$synth_out" in
        *"$want"*) pass "rewrites to $want" ;;
        *) fail "rewrites to $want" "not found in synced output" ;;
    esac
done
case "$synth_out" in
    *'beta-page.md'*) fail "no raw beta-page.md survives any link form" \
        "$(printf '%s' "$synth_out" | grep -n 'beta-page.md')" ;;
    *) pass "no raw beta-page.md survives any link form" ;;
esac
rm -rf "$SYNTH" "$SYNTH_WIKI"

# The collection filter, tested apart from the late cleanup that also removes
# SUMMARY.md: with the filter gone, the cleanup still deletes the file from
# disk, so asserting only on the file cannot see the filter at all. Home is
# built from the collected list, so it can.
if grep -qiE '^- \[SUMMARY\]' "$WIKI/Home.md"; then
    fail "SUMMARY is not collected as a page" "Home lists it"
else
    pass "SUMMARY is not collected as a page"
fi

# The empty-source guard: it keeps a broken checkout from overwriting Home
# with an empty page list.
EMPTY_SRC=$(mktemp -d)
EMPTY_WIKI=$(mktemp -d)
printf 'existing\n' > "$EMPTY_WIKI/Home.md"
if "$SYNC" "$EMPTY_SRC" "$EMPTY_WIKI" >/dev/null 2>&1; then
    fail "an empty source directory is refused" "the sync reported success"
elif [ "$(cat "$EMPTY_WIKI/Home.md")" = "existing" ]; then
    pass "an empty source directory is refused, Home untouched"
else
    fail "an empty source directory is refused" "Home was overwritten anyway"
fi
rm -rf "$EMPTY_SRC" "$EMPTY_WIKI"

# Home lists every published page.
count_published=$(find "$WIKI" -name '*.md' -not -name 'Home.md' | wc -l | tr -d ' ')
# Only the Pages section — the Quick Links below it are also `- [...]` lines.
count_listed=$(sed -n '/^## Pages$/,/^## Quick Links$/p' "$WIKI/Home.md" | grep -cE '^- \[')
[ "$count_published" = "$count_listed" ] && pass "Home lists all $count_published pages" \
    || fail "Home lists all pages" "published=$count_published listed=$count_listed"

echo
if [ "$fails" -eq 0 ]; then
    echo "sync-wiki: all checks passed"
else
    echo "sync-wiki: $fails check(s) failed"
    exit 1
fi
