# AGENTS.md - ArmadAI

This file provides instructions for AI coding agents working on the ArmadAI project.

## Project Overview

ArmadAI is an AI agent fleet orchestrator written in Rust. It allows defining,
managing and executing specialized AI agents configured via Markdown files.

## Build & Test

```bash
# Build
cargo build

# Run tests
cargo test

# Run with debug logging
RUST_LOG=armadai=debug cargo run -- <command>

# Run clippy
cargo clippy -- -D warnings

# Format code
cargo fmt
```

## Code Style

- Follow standard Rust conventions (rustfmt defaults)
- Use `anyhow::Result` for application-level errors
- Use `thiserror` for library-level typed errors
- Prefer `tracing` macros over `println!` for logging
- Keep functions small and focused
- Document public APIs with doc comments

## Architecture

See ARCHITECTURE.md for full details. Key modules:
- `src/cli/` — CLI commands (clap)
- `src/tui/` — Terminal UI (ratatui)
- `src/core/` — Domain: Agent, Coordinator, Task, Context
- `src/core/project.rs` — Project config (armadai.yaml) with agent/prompt/skill resolution
- `src/core/prompt.rs` — Composable prompt fragments with YAML frontmatter
- `src/core/skill.rs` — Skills following the SKILL.md open standard
- `src/core/starter.rs` — Starter packs installation
- `src/core/fleet.rs` — Fleet definitions linking agent groups to directories
- `src/parser/` — Markdown agent file parsing
- `src/parser/frontmatter.rs` — Generic YAML frontmatter extraction
- `src/providers/` — LLM provider abstraction (API, CLI, proxy)
- `src/linker/` — Generate native config for AI CLIs (Claude Code, Copilot, etc.)
- `src/registry/` — Community agent registry integration (awesome-copilot)
- `src/storage/` — SurrealDB persistence layer
- `src/secrets/` — SOPS + age secret management

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)]` module)
- Integration tests go in `tests/`
- Test agent files go in `starters/` (organized by pack)

## Commit Messages

- Use conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Keep the first line under 72 characters
- Add a body for non-trivial changes
