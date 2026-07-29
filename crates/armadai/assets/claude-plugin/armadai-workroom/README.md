# armadai-workroom (Claude Code plugin)

A `SessionStart` hook registers each Claude Code session (session id + transcript
path) into `~/.config/armadai/claude-sessions.jsonl` via `armadai
__claude-register-session`. Then `armadai watch` visualizes the session — live or
replay — in the ArmadAI Workroom.

## Prerequisite

The `armadai` binary **with the `watch` command** (>= 1.0.0-rc.5) must be on your
`PATH` (the hook calls it). From the repo:

    cargo install --path crates/armadai --bin armadai --force

## Install (local marketplace)

Local plugins install via a marketplace. From the repo root:

    claude plugin marketplace add "$(pwd)/crates/armadai/assets/claude-plugin"
    claude plugin install armadai-workroom@armadai

Start (or restart) a Claude Code session — the hook records it in the index.

## Use

    armadai watch --last            # follow the most recently registered session
    armadai watch --session <id>
    armadai watch --json            # JSONL RunEvents instead of the TUI

## Uninstall

    claude plugin uninstall armadai-workroom@armadai
    claude plugin marketplace remove armadai
