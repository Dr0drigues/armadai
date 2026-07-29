# armadai-workroom (Claude Code plugin)

Registers each Claude Code session (session id + transcript path) into
`~/.config/armadai/claude-sessions.jsonl` via a `SessionStart` hook that calls
`armadai __claude-register-session`. Then run `armadai watch` to visualize the
session live (or replay it) in the ArmadAI Workroom.

## Install

Requires the `armadai` binary on your `PATH`.

    claude plugin install ./crates/armadai/assets/claude-plugin

## Use

    armadai watch            # pick the most recent session
    armadai watch --last     # most recent, no prompt
    armadai watch --session <id>
    armadai watch --json     # JSONL instead of the TUI
