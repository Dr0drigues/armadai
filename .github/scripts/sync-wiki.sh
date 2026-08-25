#!/usr/bin/env bash
#
# Copy docs/wiki/*.md into a checked-out GitHub Wiki working copy, renaming
# each page to the wiki's Title-Case convention and rewriting internal links
# to match.
#
# Usage: sync-wiki.sh <source-dir> <wiki-working-copy>
#
# Both the page names and the link rewrites are derived from the files that
# are actually present. An earlier version kept two hand-maintained tables
# instead; the page-name table turned out to reproduce exactly what the
# derivation already produced, while the link table silently froze at nine
# pages — so links to every page added afterwards shipped to the wiki
# pointing at `page.md`, which the wiki does not serve.

set -euo pipefail

SRC_DIR="${1:?usage: sync-wiki.sh <source-dir> <wiki-working-copy>}"
WIKI_DIR="${2:?usage: sync-wiki.sh <source-dir> <wiki-working-copy>}"

# mdBook's table of contents. It drives the book's sidebar and has no meaning
# as a wiki page — the wiki has its own sidebar and a generated Home.
readonly EXCLUDED="SUMMARY.md"

# kebab-case.md -> Kebab-Case.md. Written without GNU sed's `\u` so it behaves
# the same on a maintainer's macOS shell as on the CI runner.
wiki_page_name() {
    local base="${1%.md}" out="" seg head
    local IFS='-'
    # shellcheck disable=SC2206 # deliberate word split on IFS
    local segs=($base)
    unset IFS
    for seg in "${segs[@]}"; do
        head=$(printf '%s' "${seg:0:1}" | tr '[:lower:]' '[:upper:]')
        out+="${out:+-}${head}${seg:1}"
    done
    printf '%s.md' "$out"
}

# Collect every page that will exist on the wiki, so links can be rewritten
# for all of them rather than for a frozen subset.
declare -a SRC_FILES=()
for src in "$SRC_DIR"/*.md; do
    [ -e "$src" ] || continue
    filename=$(basename "$src")
    [ "$filename" = "$EXCLUDED" ] && continue
    SRC_FILES+=("$filename")
done

if [ ${#SRC_FILES[@]} -eq 0 ]; then
    echo "sync-wiki: no pages found in $SRC_DIR" >&2
    exit 1
fi

for filename in "${SRC_FILES[@]}"; do
    dest="$WIKI_DIR/$(wiki_page_name "$filename")"
    cp "$SRC_DIR/$filename" "$dest"

    # Rewrite links to every wiki page: bare, `./`-prefixed, and with a
    # `#section` anchor. The wiki serves pages without the .md extension, so an
    # unrewritten link 404s — and an anchored link is the easiest of the three
    # to miss, since it looks rewritten right up to the `#`.
    for target in "${SRC_FILES[@]}"; do
        page_link="$(wiki_page_name "$target")"
        page_link="${page_link%.md}"
        # The filename's own dots must not act as ERE wildcards.
        target_re=$(printf '%s' "$target" | sed 's/\./\\./g')
        sed -i.bak -E "s|\((\./)?${target_re}(#[^)]*)?\)|(${page_link}\2)|g" "$dest"
        rm -f "$dest.bak"
    done
done

# Retire the excluded page if a previous run published it. Stopping the copy
# does not un-publish what is already on the wiki, and this one page is a
# known artefact of the old sync rather than anything a person wrote.
#
# Deliberately narrow: removing everything on the wiki without a matching
# source would delete hand-written pages, which is the same over-deletion
# failure the link manifest exists to prevent. Anything else stale has to be
# retired by hand, on purpose.
if [ -e "$WIKI_DIR/$EXCLUDED" ]; then
    rm -f "$WIKI_DIR/$EXCLUDED"
    echo "sync-wiki: removed stale $EXCLUDED published by an earlier run"
fi

# Home.md — the wiki's landing page, listing every synced page.
{
    printf '# ArmadAI Wiki\n\n'
    printf 'AI agent fleet orchestrator — define, manage and run specialized agents from Markdown files.\n\n'
    printf '## Pages\n\n'
    for filename in "${SRC_FILES[@]}"; do
        page="$(wiki_page_name "$filename")"
        page="${page%.md}"
        printf -- '- [%s](%s)\n' "${page//-/ }" "$page"
    done
    printf '\n## Quick Links\n\n'
    printf -- '- [GitHub Repository](https://github.com/Dr0drigues/armadai)\n'
    printf -- '- [Releases](https://github.com/Dr0drigues/armadai/releases)\n'
    printf -- '- [Issues](https://github.com/Dr0drigues/armadai/issues)\n'
} > "$WIKI_DIR/Home.md"

echo "sync-wiki: synced ${#SRC_FILES[@]} pages (excluded $EXCLUDED)"
