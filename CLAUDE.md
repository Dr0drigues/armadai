# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

swarm-festai is an AI agent fleet orchestrator written in Rust (edition 2024). Agents are defined as Markdown files and executed against any LLM provider (API or CLI tool). The binary is named `swarm`.

## Build & Test Commands

```bash
# Fast development cycle (skips RocksDB C++ compilation)
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo test --no-default-features --features tui,providers-api
cargo fmt -- --check

# Full build (all features including RocksDB — slow first time)
cargo build --release

# Run a single test
cargo test --no-default-features --features tui,providers-api test_name

# Run with debug logs
RUST_LOG=debug cargo run -- list
```

CI runs clippy/test with `--no-default-features --features tui` (no `providers-api`) to minimize deps. Always verify clippy passes in **both** modes:
- `--no-default-features --features tui` (CI mode)
- `--no-default-features --features tui,providers-api` (with API providers)

## Feature Flags

Heavy optional dependencies are gated behind feature flags to keep CI fast (~1min vs ~15min):

| Feature | Gates | Impact |
|---|---|---|
| `tui` | ratatui, crossterm | TUI dashboard |
| `storage` | surrealdb (in-memory) | Embedded DB |
| `storage-rocksdb` | surrealdb + RocksDB | Persistent storage (C++ build) |
| `providers-api` | reqwest | HTTP-based LLM providers |

Default features: `tui`, `storage-rocksdb`, `providers-api`. CI build uses `--no-default-features --features tui,storage` to skip RocksDB.

Code that depends on optional features must use `#[cfg(feature = "...")]`. Several modules in `main.rs` use `#[allow(dead_code)]` because they are scaffolded but not fully wired yet.

## Architecture

**Execution flow**: CLI command → load agent `.md` file → parse with `pulldown-cmark` → create provider via factory → execute `complete()` or `stream()` → display result → record in storage.

**Key modules**:
- `cli/` — One file per command, each exports `async fn execute(...)`. Add new commands in `cli/mod.rs` (enum variant + handler).
- `parser/` — Converts Markdown agent files into `Agent` struct. Required sections: H1 (name), `## Metadata`, `## System Prompt`.
- `providers/` — `Provider` trait (in `traits.rs`) with `complete()` and `stream()` methods. Factory (`factory.rs`) constructs the right provider from agent metadata. Implementations: `api/anthropic.rs` (full), `cli.rs` (full), openai/google/proxy (stubs).
- `core/` — Domain types: `Agent`, `AgentMetadata`, `Task`, `SharedContext`, `Coordinator`, `Pipeline`.
- `storage/` — SurrealDB wrapper. `schema.rs` defines tables (`runs`, `agent_stats`), `queries.rs` has CRUD operations.
- `tui/` — Ratatui-based terminal UI. `app.rs` holds state, `views/` renders tabs (Dashboard/Execution/History/Costs), `widgets/` provides reusable components.
- `secrets/` — SOPS + age encrypted secrets loader.

**Provider trait** (`providers/traits.rs`):
```rust
trait Provider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(&self, request: CompletionRequest) -> Result<TokenStream>;
    fn metadata(&self) -> ProviderMetadata;
}
```

**Agent definition** lives in `agents/*.md`. Templates in `templates/*.md` use `{{name}}`, `{{stack}}`, `{{description}}` placeholders.

## Git Conventions

- **Branch model**: `master` (releases), `develop` (default/integration), `feature/*` branches
- **Conventional Commits** enforced by `.githooks/commit-msg` hook and CI. Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **PR process**: Always squash merge to `develop`. Before merging: check for Dependabot PRs, verify CI passes (all 6 checks: fmt, clippy, test, build, conventional commits, audit).
- Enable hooks after clone: `git config core.hooksPath .githooks`

## Language

All communication with the user must be in **French**. Code, comments, and commit messages remain in English.
