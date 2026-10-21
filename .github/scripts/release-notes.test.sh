#!/usr/bin/env bash
#
# Tests for release-notes.sh. Run from anywhere:
#   .github/scripts/release-notes.test.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NOTES="$SCRIPT_DIR/release-notes.sh"

fails=0
pass() { printf '  ok   %s\n' "$1"; }
fail() { printf '  FAIL %s\n     %s\n' "$1" "$2"; fails=$((fails + 1)); }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
CL="$TMP/CHANGELOG.md"
cat > "$CL" <<'EOF'
## Unreleased

### Fixed

- unreleased fix

## v1.0.0-rc.10 (2026-08-01)

- rc.10 only

## v1.0.0-rc.1 (2026-07-17)


### Added

- rc.1 feature

### Fixed

- rc.1 fix
  

## v1.0.0-beta.2 (2026-07-10)

## v0.9.0 (2026-01-01)

- last section
EOF

# expect_out <name> <expected stdout> <args...>: exit 0 and exact output.
expect_out() {
    local name=$1 expected=$2 out
    shift 2
    if out=$("$NOTES" "$@" 2>"$TMP/err"); then
        if [ "$out" = "$expected" ]; then
            pass "$name"
        else
            fail "$name" "got: $(printf '%s' "$out" | head -c 300)"
        fi
    else
        fail "$name" "exited non-zero: $(cat "$TMP/err")"
    fi
}

# expect_fail <name> <stderr fragment> <args...>: non-zero exit, empty stdout.
expect_fail() {
    local name=$1 fragment=$2 out
    shift 2
    if out=$("$NOTES" "$@" 2>"$TMP/err"); then
        fail "$name" "exited 0 with: $(printf '%s' "$out" | head -c 300)"
    elif [ -n "$out" ] || ! grep -qF -- "$fragment" "$TMP/err"; then
        fail "$name" "stdout '$out', stderr '$(cat "$TMP/err")'"
    else
        pass "$name"
    fi
}

# Section present, `-rc.N` version, `###` subheadings kept, blank lines at
# both ends trimmed (two leading, two trailing of which one holds spaces:
# `$(...)` would strip a bare trailing newline on its own).
expect_out "rc section: exact body, ### kept, blank lines trimmed" \
    "$(printf '### Added\n\n- rc.1 feature\n\n### Fixed\n\n- rc.1 fix')" \
    v1.0.0-rc.1 "$CL"

# The heading match must not be a prefix match: rc.1 is listed after rc.10.
expect_out "rc.10 is not mistaken for rc.1" "- rc.10 only" v1.0.0-rc.10 "$CL"

expect_out "last section of the file" "- last section" v0.9.0 "$CL"

expect_out "--unreleased takes the Unreleased section" \
    "$(printf '### Fixed\n\n- unreleased fix')" --unreleased "$CL"

expect_fail "missing section fails" "no '## v2.0.0' section" v2.0.0 "$CL"

expect_fail "a prefix of a version is not that version" "no '## v1.0.0-rc' section" v1.0.0-rc "$CL"

expect_fail "empty section fails" "section of $CL is empty" v1.0.0-beta.2 "$CL"

expect_fail "unreadable changelog fails" "cannot read" v0.9.0 "$TMP/nope.md"

echo
if [ "$fails" -ne 0 ]; then
    echo "$fails failure(s)"
    exit 1
fi
echo "all tests passed"
