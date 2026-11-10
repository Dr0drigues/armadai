#!/usr/bin/env bash
# Scratch copies under a unique prefix, removed by literal path only.
#   scratch.sh new <name>   create ${TMPDIR:-/tmp}/armadai-<name>-<pid>-<epoch>, print its path
#   scratch.sh rm <path>    remove <path>, refused unless it starts with that prefix
set -u

base="${TMPDIR:-/tmp}"
base="${base%/}"
prefix="$base/armadai-"

die() { echo "scratch.sh: $*" >&2; exit 1; }

case "${1:-}" in
  new)
    [ $# -eq 2 ] || die "usage: scratch.sh new <name>"
    case "$2" in
      ''|*[!A-Za-z0-9._-]*) die "refused: the name must be letters, digits, '.', '_' or '-'" ;;
    esac
    dir="$prefix$2-$$-$(date +%s)"
    mkdir -- "$dir" || die "cannot create $dir"
    echo "$dir"
    ;;
  rm)
    [ $# -eq 2 ] || die "usage: scratch.sh rm <path>"
    path="$2"
    case "$path" in
      "$prefix"?*) ;;
      *) die "refused: '$path' does not start with $prefix" ;;
    esac
    case "/$path/" in
      */../*) die "refused: '$path' contains '..'" ;;
    esac
    # Literal path, never a pattern or a glob.
    rm -rf -- "$path"
    ;;
  *)
    die "usage: scratch.sh new <name> | rm <path>"
    ;;
esac
