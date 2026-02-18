# AGENTS.md - ArmadAI

This file provides instructions for AI coding agents working on the ArmadAI project.

## Project Overview

ArmadAI is an AI agent fleet orchestrator written in Rust. It allows defining,
managing and executing specialized AI agents configured via Markdown files.

## Build & Test

```bash
# Build (all features)
cargo build

# Run tests (with API providers)
cargo test --no-default-features --features tui,providers-api

# Run clippy (both modes — CI and full)
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings

# Run with debug logging
RUST_LOG=armadai=debug cargo run -- <command>

# Format code
cargo fmt -- --check
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
- `src/linker/` — Generate native config for AI CLIs (Claude Code, Copilot, Gemini, etc.)
- `src/registry/` — Community agent registry integration (awesome-copilot)
- `src/skills_registry/` — GitHub-based skills discovery and installation
- `src/model_registry/` — Dynamic model catalog from models.dev (fetch, cache, enriched selection)
- `src/storage/` — SQLite persistence layer (rusqlite)
- `src/web/` — Axum-based web UI dashboard
- `src/secrets/` — SOPS + age secret management

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)]` module)
- Integration tests go in `tests/`
- Test agent files go in `starters/` (organized by pack)

## Commit Messages

- Use conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Keep the first line under 72 characters
- Add a body for non-trivial changes
