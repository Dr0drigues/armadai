# AGENTS.md - ArmadAI

This file provides instructions for AI coding agents working on the ArmadAI project.

## Project Overview

ArmadAI is a declarative compiler written in Rust (edition 2024). A team is described once in
`.armadai/` (`project.yaml`, one Markdown file per agent in `agents/`, `skills/`, `flows/`,
`automations/`), then `armadai link` compiles it into the native files of Claude Code, Codex,
OpenCode and Copilot. The legacy format (`.armadai/config.yaml`, `armadai.yaml`,
`.armadai/agents.yaml`, prompts) is read only by `armadai migrate`. ArmadAI does not run agents:
there is no execution engine, no TUI and no web UI.

## Build & Test

There is a single configuration (no optional cargo features), identical to CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
cargo build -p armadai-core   # the core builds standalone
cargo build -p armadai-model  # so does the model

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

- `crates/armadai-model` — the canonical project model, pure (no other workspace crate): types,
  strict loader, validator with diagnostics, writer, JSON Schemas (`schemas/v1/`), flow templates,
  Mermaid export.
- `crates/armadai-core` — the legacy model and parser, read by `migrate` (`agent.rs`,
  `agent_decl.rs`, `agent_source.rs`, `parser/`, `project.rs`, `orchestration/`), plus the
  embedded skills and starter packs, the static model tier table and the linked-projects registry.
- `crates/armadai` — the binary: `cli/` (commands), `compile/` (one plan per target, the Claude
  Code adapter, the conformance report), `linker/` (the Codex, Copilot and OpenCode linkers and
  the link manifest), `migrate/`, `claude_adapter/` (the `__claude-policy-gate` hook),
  `skills_registry/`, `starters_registry/`, `templates/`, and integration tests with the golden
  outputs in `tests/`.

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
