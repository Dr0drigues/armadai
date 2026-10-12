# AGENTS.md - ArmadAI

This file provides instructions for AI coding agents working on the ArmadAI project.

## Project Overview

ArmadAI is a declarative compiler written in Rust (edition 2024). Agents, prompts and skills are
described once (Markdown files or `.armadai/agents.yaml`), then `armadai link` compiles them into
the native files of Claude Code, Codex, OpenCode and Copilot. ArmadAI does not run agents: there
is no execution engine, no TUI and no web UI.

## Build & Test

There is a single configuration (no optional cargo features), identical to CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
cargo build -p armadai-core   # the core builds standalone

# Run with debug logging
RUST_LOG=armadai=debug cargo run -- <command>
```

`crates/armadai` is binary-only: `cargo test --lib` there runs nothing and reports no error; use
`--bin armadai` or `--test <name>`.

## Code Style

- Follow standard Rust conventions (rustfmt defaults)
- Use `anyhow::Result` for application-level errors
- Use `thiserror` for library-level typed errors
- Prefer `tracing` macros over `println!` for logging
- Keep functions small and focused
- Document public APIs with doc comments

## Architecture

See ARCHITECTURE.md for full details. Workspace crates:

- `crates/armadai-core` — the domain: agent model and parser (`agent.rs`, `agent_decl.rs`,
  `agent_source.rs`, `parser/`), project config (`project.rs`), prompts, skills, starter packs
  (embedded), model resolution, and the orchestration declaration types (`orchestration/`,
  including the Claude Code `policy` gate).
- `crates/armadai` — the binary: `cli/` (commands), `linker/` (one linker per target plus the
  link manifest), `claude_adapter/` (the `__claude-policy-gate` hook), `skills_registry/`,
  `starters_registry/`, `templates/`, and integration tests with the golden `link` outputs in
  `tests/`.
- `crates/armadai-providers` — read-only access to the local models.dev catalogue cache and the
  tool-to-vendor table. No network access.
- `crates/armadai-secrets` — SOPS + age secret management.

`frozen/armadai-audit/` holds the former `audit` command. It is outside the workspace and outside
CI: do not build or edit it.

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)]` module)
- Integration tests go in `crates/armadai/tests/`
- Never call a real API in a test; isolate the config directory (`ARMADAI_CONFIG_DIR`) in a temp dir

## Commit Messages

- Use conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Keep the first line under 72 characters
- Add a body for non-trivial changes
