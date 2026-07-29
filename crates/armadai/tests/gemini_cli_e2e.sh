#!/usr/bin/env bash
# End-to-end integration test: init → link → verify with Gemini CLI
#
# Prerequisites:
#   - gemini CLI installed and authenticated
#   - armadai binary built (cargo build --release)
#
# Usage:
#   ./tests/gemini_cli_e2e.sh [path/to/armadai]

set -euo pipefail

ARMADAI="${1:-./target/release/armadai}"
TESTDIR=$(mktemp -d)
PASS=0
FAIL=0

cleanup() { rm -rf "$TESTDIR"; }
trap cleanup EXIT

green() { printf "\033[32m%s\033[0m\n" "$1"; }
red()   { printf "\033[31m%s\033[0m\n" "$1"; }

assert_contains() {
  local label="$1" content="$2" expected="$3"
  if echo "$content" | grep -qi "$expected"; then
    green "  ✓ $label"
    PASS=$((PASS + 1))
  else
    red "  ✗ $label (expected '$expected' not found)"
    FAIL=$((FAIL + 1))
  fi
}

assert_file_exists() {
  local label="$1" path="$2"
  if [[ -f "$path" ]]; then
    green "  ✓ $label"
    PASS=$((PASS + 1))
  else
    red "  ✗ $label ($path not found)"
    FAIL=$((FAIL + 1))
  fi
}

# ── Check prerequisites ──────────────────────────────────────────
if ! command -v gemini &>/dev/null; then
  echo "SKIP: gemini CLI not found"
  exit 0
fi

if [[ ! -x "$ARMADAI" ]]; then
  echo "SKIP: armadai binary not found at $ARMADAI"
  exit 0
fi

echo "=== ArmadAI + Gemini CLI End-to-End Test ==="
echo "  armadai: $ARMADAI"
echo "  gemini:  $(which gemini)"
echo "  tmpdir:  $TESTDIR"
echo

# ── Step 1: Init project with orchestration-demo starter ─────────
echo "Step 1: Init project with orchestration-demo starter"
cd "$TESTDIR"
git init -q

"$ARMADAI" init --pack orchestration-demo 2>&1 | tail -3

# Create project config
cat > armadai.yaml <<'YAML'
agents:
  - name: demo-coordinator
  - name: demo-analyst
  - name: demo-reviewer
  - name: demo-writer

prompts:
  - name: demo-conventions

link:
  target: gemini
  coordinator: demo-coordinator

orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: demo-coordinator
  teams:
    - agents:
        - demo-analyst
        - demo-reviewer
        - demo-writer
  max_depth: 3
YAML

green "  ✓ Project initialized"
echo

# ── Step 2: Link to Gemini CLI ───────────────────────────────────
echo "Step 2: Link agents to Gemini CLI"
"$ARMADAI" link --target gemini 2>&1 | grep -v "^hint:"

assert_file_exists "GEMINI.md generated" "$TESTDIR/.gemini/GEMINI.md"
assert_file_exists "demo-analyst.md generated" "$TESTDIR/.gemini/agents/demo-analyst.md"
assert_file_exists "demo-reviewer.md generated" "$TESTDIR/.gemini/agents/demo-reviewer.md"
assert_file_exists "demo-writer.md generated" "$TESTDIR/.gemini/agents/demo-writer.md"
echo

# ── Step 3: Verify linked file contents ──────────────────────────
echo "Step 3: Verify linked file contents"

GEMINI_MD=$(cat "$TESTDIR/.gemini/GEMINI.md")
assert_contains "GEMINI.md has coordinator prompt" "$GEMINI_MD" "coordinator"
assert_contains "GEMINI.md lists analyst" "$GEMINI_MD" "demo-analyst"
assert_contains "GEMINI.md lists reviewer" "$GEMINI_MD" "demo-reviewer"
assert_contains "GEMINI.md lists writer" "$GEMINI_MD" "demo-writer"
assert_contains "GEMINI.md has team section" "$GEMINI_MD" "## Team"

ANALYST_MD=$(cat "$TESTDIR/.gemini/agents/demo-analyst.md")
assert_contains "Analyst has frontmatter" "$ANALYST_MD" "name: demo-analyst"
assert_contains "Analyst has system prompt" "$ANALYST_MD" "analyst"

REVIEWER_MD=$(cat "$TESTDIR/.gemini/agents/demo-reviewer.md")
assert_contains "Reviewer has frontmatter" "$REVIEWER_MD" "name: demo-reviewer"
assert_contains "Reviewer has review role" "$REVIEWER_MD" "review"
echo

# ── Step 4: Test Gemini CLI reads the config correctly ───────────
echo "Step 4: Test Gemini CLI delegation (live — may take 10-30s)"

RESPONSE=$(cd "$TESTDIR" && gemini -p "Analyze this code: fn hello() { println!(\"hi\"); }" 2>&1 | grep -v "Keychain\|Using File\|Loaded cached\|Require stack\|node_modules")

assert_contains "Gemini produced output" "$RESPONSE" "."
assert_contains "Response mentions analyst or analysis" "$RESPONSE" "analy"
assert_contains "Response mentions review" "$RESPONSE" "review"
echo

# ── Step 5: Test direct agent invocation ─────────────────────────
echo "Step 5: Test direct agent invocation (live)"

REVIEW_RESPONSE=$(cd "$TESTDIR" && gemini -p "@demo-reviewer Evaluate: fn add(a: i32, b: i32) -> i32 { a + b }" 2>&1 | grep -v "Keychain\|Using File\|Loaded cached\|Require stack\|node_modules")

assert_contains "Reviewer responded" "$REVIEW_RESPONSE" "."
assert_contains "Review mentions quality/correctness" "$REVIEW_RESPONSE" "correct"
echo

# ── Results ──────────────────────────────────────────────────────
echo "=== Results ==="
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
[[ $FAIL -eq 0 ]] && green "  ALL TESTS PASSED" || red "  SOME TESTS FAILED"
exit $FAIL
