# AGENTS.md - swarm-festai

This file provides instructions for AI coding agents working on the swarm-festai project.

## Project Overview

swarm-festai is an AI agent fleet orchestrator written in Rust. It allows defining,
managing and executing specialized AI agents configured via Markdown files.

## Build & Test

```bash
# Build
cargo build

# Run tests
cargo test

# Run with debug logging
RUST_LOG=swarm_festai=debug cargo run -- <command>

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
- `src/parser/` — Markdown agent file parsing
- `src/providers/` — LLM provider abstraction (API, CLI, proxy)
- `src/storage/` — SurrealDB persistence layer
- `src/secrets/` — SOPS + age secret management

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)]` module)
- Integration tests go in `tests/`
- Test agent files go in `agents/examples/`

## Commit Messages

- Use conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Keep the first line under 72 characters
- Add a body for non-trivial changes
