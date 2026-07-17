#!/usr/bin/env bash
#
# migrate-v0-to-v1.sh — mechanical, deterministic helpers for the ArmadAI
# v0 -> v1 migration.
#
# Full guide (read this first): docs/wiki/migration-v0-to-v1.md
#
# This script is SAFE BY DESIGN:
#   - Dry-run by default. Nothing is written unless you pass --apply.
#   - Every file it touches is backed up first (see --apply below).
#   - It never deletes anything.
#   - Its scope is intentionally narrow — only the 3 checks below. Anything
#     else in the migration guide (e.g. converting `fleet` -> `teams`) needs
#     a human decision and is only *reported*, never rewritten.
#
# What it does:
#   (a) Detect legacy `fleet` usage:
#         - ~/.config/armadai/fleets/ directory (reported, never touched)
#         - a top-level `fleet:` key in armadai.yaml / .armadai/config.yaml
#           (reported, never touched — auto-detection of this format was
#           removed in v1, see docs/wiki/migration-v0-to-v1.md#1)
#   (b) Rewrite deprecated provider syntax in agent .md files:
#         `- provider: cli claude` -> `- provider: claude`
#       (only for the known unified tool names: claude, gemini, gpt, aider)
#   (c) Detect deprecated model names in agent .md files and point to
#       `armadai models update` (never rewritten here — that command is the
#       authoritative source of truth for model aliases).
#
# Usage:
#   scripts/migrate-v0-to-v1.sh [--apply] [OPTIONS]
#
# Options:
#   --apply                  Actually write changes (default: dry-run).
#   --user-agents-dir DIR    Override the user agent library directory
#                            (default: ~/.config/armadai/agents).
#   --user-fleets-dir DIR    Override the legacy fleets directory
#                            (default: ~/.config/armadai/fleets).
#   --project-dir DIR        Project directory to scan (default: current
#                            directory). Looks for agents/, .armadai/agents/,
#                            armadai.yaml, armadai.yml, .armadai/config.yaml.
#   -h, --help               Show this help and exit.
#
# Exit codes:
#   0  success (dry-run or apply completed; findings are printed, not an
#      error condition by themselves)
#   1  usage error (bad flag, missing argument)
#   2  internal error (e.g. backup could not be created before a write)
#
set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------

CONFIG_DIR="${ARMADAI_CONFIG_DIR:-$HOME/.config/armadai}"
USER_AGENTS_DIR="$CONFIG_DIR/agents"
USER_FLEETS_DIR="$CONFIG_DIR/fleets"
PROJECT_DIR="$(pwd)"
APPLY=0
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"

# Known unified tool names (src/providers/factory.rs::KNOWN_TOOLS).
KNOWN_TOOLS=(claude gemini gpt aider)

# Deprecated model names embedded in src/linker/model_aliases.rs at the time
# this script was written. This list is for *detection only* — the
# authoritative, always-up-to-date source is `armadai models check`.
DEPRECATED_MODELS=(
    gemini-3.0-pro
    gemini-1.5-flash
    gemini-1.5-pro
    gemini-1.0-pro
    gpt-4-turbo
    gpt-3.5-turbo
)

# Counters for the final summary.
COUNT_FLEET_DIR=0
COUNT_FLEET_YAML=0
COUNT_PROVIDER_FILES=0
COUNT_PROVIDER_LINES=0
COUNT_MODEL_FILES=0
COUNT_MODEL_HITS=0

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

print_help() {
    # Print the usage block at the top of this file (everything between the
    # shebang and the `set -euo pipefail` line).
    sed -n '2,/^set -euo pipefail/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

log()  { printf '%s\n' "$*"; }
info() { printf '  %s\n' "$*"; }
warn() { printf 'WARNING: %s\n' "$*" >&2; }

section() {
    printf '\n=== %s ===\n' "$*"
}

# backup_file <path>
# Copies <path> to <path>.bak-<TIMESTAMP> before any in-place modification.
# Aborts the whole script (exit 2) if the backup cannot be created — we never
# want to modify a file we failed to back up.
backup_file() {
    local file="$1"
    local backup="${file}.bak-${TIMESTAMP}"
    if ! cp -p "$file" "$backup"; then
        warn "could not create backup '$backup' for '$file' — aborting before any write."
        exit 2
    fi
    info "backup created: $backup"
}

# find_md_files <dir...>
# Recursively lists .md files under the given directories (skipping any that
# don't exist). Safe to call with zero existing directories.
find_md_files() {
    local dir
    for dir in "$@"; do
        [ -d "$dir" ] || continue
        find "$dir" -type f -name '*.md' 2>/dev/null
    done
}

# ---------------------------------------------------------------------------
# (a) Legacy fleet detection — report only, never modified.
# ---------------------------------------------------------------------------

check_fleet_legacy() {
    section "Legacy fleet detection"

    if [ -d "$USER_FLEETS_DIR" ]; then
        local fleet_files
        fleet_files="$(find "$USER_FLEETS_DIR" -type f 2>/dev/null || true)"
        if [ -n "$fleet_files" ]; then
            local n
            n="$(printf '%s\n' "$fleet_files" | grep -c . || true)"
            COUNT_FLEET_DIR="$n"
            log "Found legacy fleet directory: $USER_FLEETS_DIR ($n file(s))"
            printf '%s\n' "$fleet_files" | sed 's/^/  - /'
            cat <<'EOF'

  ArmadAI no longer reads this directory (removed in v1.0.0-beta.1, #138).
  This script will NOT convert or delete these files: fleet -> teams
  conversion depends on how each fleet was actually used and needs a human
  decision. See docs/wiki/migration-v0-to-v1.md#1-removal-of-fleet for
  the manual conversion steps, then remove this directory yourself once
  you've verified the equivalent agents/teams/orchestration config works.
EOF
        else
            log "Legacy fleet directory exists but is empty: $USER_FLEETS_DIR"
        fi
    else
        log "No legacy fleet directory found ($USER_FLEETS_DIR) — OK."
    fi

    local cfg
    for cfg in "$PROJECT_DIR/armadai.yaml" "$PROJECT_DIR/armadai.yml" \
               "$PROJECT_DIR/.armadai/config.yaml"; do
        [ -f "$cfg" ] || continue
        if grep -qE '^fleet:[[:space:]]*[^[:space:]]' "$cfg"; then
            COUNT_FLEET_YAML=$((COUNT_FLEET_YAML + 1))
            log "Found legacy 'fleet:' key in: $cfg"
            cat <<EOF

  This file uses the pre-v1 fleet YAML format ('fleet:' + 'agents:' +
  'source:'). ArmadAI v1 no longer detects or converts this format
  automatically — it will be parsed as a modern (mostly empty)
  ProjectConfig instead. This script will NOT rewrite it for you.
  Convert it manually to the v1 agents:/orchestration: format — see
  docs/wiki/migration-v0-to-v1.md#1-removal-of-fleet.
EOF
        fi
    done

    if [ "$COUNT_FLEET_DIR" -eq 0 ] && [ "$COUNT_FLEET_YAML" -eq 0 ]; then
        log "No legacy fleet YAML config found in project — OK."
    fi
}

# ---------------------------------------------------------------------------
# (b) Deprecated provider syntax — `provider: cli <tool>` -> `provider: <tool>`
# ---------------------------------------------------------------------------

check_and_fix_provider_syntax() {
    section "Deprecated provider syntax ('provider: cli <tool>')"

    local tools_alt
    tools_alt="$(IFS='|'; echo "${KNOWN_TOOLS[*]}")"
    local pattern="^([[:space:]]*-[[:space:]]*provider:[[:space:]]*)cli[[:space:]]+(${tools_alt})[[:space:]]*\$"

    local files
    files="$(find_md_files "$USER_AGENTS_DIR" \
                            "$PROJECT_DIR/agents" \
                            "$PROJECT_DIR/.armadai/agents" | sort -u || true)"

    if [ -z "$files" ]; then
        log "No agent .md files found under:"
        info "$USER_AGENTS_DIR"
        info "$PROJECT_DIR/agents"
        info "$PROJECT_DIR/.armadai/agents"
        return 0
    fi

    local file matches n
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        matches="$(grep -nE "$pattern" "$file" 2>/dev/null || true)"
        [ -n "$matches" ] || continue

        n="$(printf '%s\n' "$matches" | grep -c . || true)"
        COUNT_PROVIDER_FILES=$((COUNT_PROVIDER_FILES + 1))
        COUNT_PROVIDER_LINES=$((COUNT_PROVIDER_LINES + n))

        log "$file ($n line(s)):"
        printf '%s\n' "$matches" | sed 's/^/  /'

        if [ "$APPLY" -eq 1 ]; then
            backup_file "$file"
            local tmp
            tmp="$(mktemp "${file}.tmp.XXXXXX")"
            sed -E "s/$pattern/\\1\\2/" "$file" > "$tmp"
            mv "$tmp" "$file"
            info "rewritten: provider: cli <tool> -> provider: <tool>"
        else
            info "[dry-run] would rewrite the line(s) above (use --apply to write)"
        fi
    done <<< "$files"

    if [ "$COUNT_PROVIDER_FILES" -eq 0 ]; then
        log "No deprecated 'provider: cli <tool>' syntax found — OK."
    fi
}

# ---------------------------------------------------------------------------
# (c) Deprecated models — report only, points to `armadai models update`.
# ---------------------------------------------------------------------------

check_deprecated_models() {
    section "Deprecated model references"

    local files
    files="$(find_md_files "$USER_AGENTS_DIR" \
                            "$PROJECT_DIR/agents" \
                            "$PROJECT_DIR/.armadai/agents" | sort -u || true)"

    if [ -z "$files" ]; then
        log "No agent .md files found to scan for deprecated models."
        return 0
    fi

    local model_alt
    model_alt="$(IFS='|'; echo "${DEPRECATED_MODELS[*]}")"
    # Match on lines that mention "model" and contain one of the deprecated
    # names (covers both `- model: X` and `- model_fallback: [X, ...]`).
    local pattern="model.*(${model_alt})"

    local file matches n
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        matches="$(grep -nE "$pattern" "$file" 2>/dev/null || true)"
        [ -n "$matches" ] || continue

        n="$(printf '%s\n' "$matches" | grep -c . || true)"
        COUNT_MODEL_FILES=$((COUNT_MODEL_FILES + 1))
        COUNT_MODEL_HITS=$((COUNT_MODEL_HITS + n))

        log "$file ($n reference(s)):"
        printf '%s\n' "$matches" | sed 's/^/  /'
    done <<< "$files"

    if [ "$COUNT_MODEL_FILES" -eq 0 ]; then
        log "No known deprecated model references found — OK."
    else
        cat <<'EOF'

  Deprecated models are resolved automatically at runtime (with a warning),
  but you should clean them up explicitly:

    armadai models check --all     # diagnostic only
    armadai models update --all    # rewrites deprecated models in place

  This script does not rewrite models itself — `armadai models update` is
  the authoritative implementation (embedded alias table + local overrides
  in ~/.config/armadai/model-aliases.json).
EOF
    fi
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

while [ $# -gt 0 ]; do
    case "$1" in
        --apply)
            APPLY=1
            shift
            ;;
        --user-agents-dir)
            [ $# -ge 2 ] || { warn "--user-agents-dir requires an argument"; exit 1; }
            USER_AGENTS_DIR="$2"
            shift 2
            ;;
        --user-fleets-dir)
            [ $# -ge 2 ] || { warn "--user-fleets-dir requires an argument"; exit 1; }
            USER_FLEETS_DIR="$2"
            shift 2
            ;;
        --project-dir)
            [ $# -ge 2 ] || { warn "--project-dir requires an argument"; exit 1; }
            PROJECT_DIR="$2"
            shift 2
            ;;
        -h|--help)
            print_help
            exit 0
            ;;
        *)
            warn "unknown argument: $1"
            print_help
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

log "ArmadAI v0 -> v1 migration helper"
log "Guide: docs/wiki/migration-v0-to-v1.md"
if [ "$APPLY" -eq 1 ]; then
    log "Mode: APPLY (files will be modified; backups written as <file>.bak-${TIMESTAMP})"
else
    log "Mode: DRY-RUN (nothing will be written — pass --apply to write changes)"
fi
log "User agents dir : $USER_AGENTS_DIR"
log "User fleets dir : $USER_FLEETS_DIR"
log "Project dir     : $PROJECT_DIR"

check_fleet_legacy
check_and_fix_provider_syntax
check_deprecated_models

section "Summary"
log "Legacy fleet directory files found : $COUNT_FLEET_DIR"
log "Legacy 'fleet:' YAML configs found  : $COUNT_FLEET_YAML"
if [ "$APPLY" -eq 1 ]; then
    log "Provider syntax lines rewritten     : $COUNT_PROVIDER_LINES (in $COUNT_PROVIDER_FILES file(s))"
else
    log "Provider syntax lines to rewrite    : $COUNT_PROVIDER_LINES (in $COUNT_PROVIDER_FILES file(s))"
fi
log "Deprecated model references found   : $COUNT_MODEL_HITS (in $COUNT_MODEL_FILES file(s))"

if [ "$APPLY" -eq 0 ]; then
    log ""
    log "This was a dry-run. Re-run with --apply to write the provider-syntax fixes."
    log "Fleet detection and deprecated-model detection are report-only regardless of --apply."
fi

exit 0
