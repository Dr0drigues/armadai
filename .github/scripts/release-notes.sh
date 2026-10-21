#!/usr/bin/env bash
#
# Print the release notes of one version, taken from the hand-written
# CHANGELOG.md: the body of its `## <version> (date)` section, up to the next
# `## ` heading outside a fenced code block, with leading and trailing blank lines trimmed.
#
#   .github/scripts/release-notes.sh v1.0.0 CHANGELOG.md
#   .github/scripts/release-notes.sh --unreleased CHANGELOG.md
#
# Fails (exit 1, message on stderr) when the section is missing or empty:
# a release must not ship without notes.

set -euo pipefail

if [ $# -ne 2 ]; then
    echo "usage: $0 <version|--unreleased> <changelog>" >&2
    exit 2
fi

if [ "$1" = "--unreleased" ]; then
    heading="## Unreleased"
else
    heading="## $1"
fi
changelog=$2

status=0
# Exact string match on the heading, followed by end of line or a space, so
# v1.0.0-rc.1 never picks up the v1.0.0-rc.10 section.
notes=$(awk -v h="$heading" '
    found && !fence && /^## / { exit }
    found { lines[++n] = $0; if ($0 ~ /^```/) fence = !fence }
    !found && ($0 == h || index($0, h " ") == 1) { found = 1 }
    END {
        if (!found) exit 3
        first = 1; last = n
        while (first <= last && lines[first] ~ /^[[:space:]]*$/) first++
        while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
        if (first > last) exit 4
        for (i = first; i <= last; i++) print lines[i]
    }
' "$changelog") || status=$?

case $status in
    0) printf '%s\n' "$notes" ;;
    3) echo "error: no '$heading' section in $changelog" >&2; exit 1 ;;
    4) echo "error: the '$heading' section of $changelog is empty" >&2; exit 1 ;;
    *) echo "error: cannot read $changelog" >&2; exit 1 ;;
esac
