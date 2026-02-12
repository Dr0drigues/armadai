# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ArmadAI is an AI agent fleet orchestrator written in Rust (edition 2024). Agents are defined as Markdown files and executed against any LLM provider (API or CLI tool). The binary is named `armadai`.

## Build & Test Commands

```bash
# Development cycle
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo test --no-default-features --features tui,providers-api
cargo fmt -- --check

# Full build (all features)
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

Heavy optional dependencies are gated behind feature flags:

| Feature | Gates | Impact |
|---|---|---|
| `tui` | ratatui, crossterm | TUI dashboard |
| `storage` | rusqlite (bundled SQLite) | Persistent storage |
| `web` | axum, tower-http | Web UI dashboard |
| `providers-api` | reqwest | HTTP-based LLM providers |

Default features: `tui`, `web`, `storage`, `providers-api`.

Code that depends on optional features must use `#[cfg(feature = "...")]`.

## Architecture

**Execution flow**: CLI command → load agent `.md` file → parse with `pulldown-cmark` → create provider via factory → execute `complete()` or `stream()` → display result → record in storage.

**Key modules**:
- `cli/` — One file per command, each exports `async fn execute(...)`. Add new commands in `cli/mod.rs` (enum variant + handler).
- `parser/` — Converts Markdown agent files into `Agent` struct. Required sections: H1 (name), `## Metadata`, `## System Prompt`.
- `providers/` — `Provider` trait (in `traits.rs`) with `complete()` and `stream()` methods. Factory (`factory.rs`) constructs the right provider from agent metadata. Implementations: `api/anthropic.rs` (full), `cli.rs` (full), openai/google/proxy (stubs).
- `core/` — Domain types: `Agent`, `AgentMetadata`, `Task`, `SharedContext`, `Coordinator`, `Pipeline`.
- `core/project.rs` — Project config (`armadai.yaml`) with agent/prompt/skill resolution.
- `core/prompt.rs` — Composable prompt fragments with YAML frontmatter.
- `core/skill.rs` — Skills following the Agent Skills open standard (SKILL.md).
- `core/fleet.rs` — Fleet definitions linking agent groups to source directories.
- `core/starter.rs` — Starter packs: curated agent bundles installed via `armadai init --pack`.
- `parser/frontmatter.rs` — Generic YAML frontmatter extraction reused by prompts and skills.
- `linker/` — Generates native config files for target AI CLIs. Trait `Linker` with one implementation per CLI (claude, copilot, cursor, aider, codex, gemini, windsurf, cline).
- `registry/` — awesome-copilot integration. Sync, search, convert agents from the community catalog.
- `skills_registry/` — GitHub-based skills discovery. Sync repos, build search index, install skills (`sync.rs`, `cache.rs`, `search.rs`).
- `storage/` — SQLite wrapper (via rusqlite). `schema.rs` defines the `runs` table, `queries.rs` has CRUD operations.
- `tui/` — Ratatui-based terminal UI. `app.rs` holds state (incl. command palette), `views/` renders tabs (Agents/Detail/History/Costs + shortcuts bar + command palette overlay), `widgets/` provides reusable components.
- `web/` — Axum-based web UI. Embedded single-page HTML app with JSON API endpoints (`/api/agents`, `/api/history`, `/api/costs`).
- `secrets/` — SOPS + age encrypted secrets loader.

**Provider trait** (`providers/traits.rs`):
```rust
trait Provider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(&self, request: CompletionRequest) -> Result<TokenStream>;
    fn metadata(&self) -> ProviderMetadata;
}
```

**Agent definition** lives in `~/.config/armadai/agents/` (user library) or project-local paths. Templates in `templates/*.md` use `{{name}}`, `{{stack}}`, `{{description}}` placeholders.

**Config** lives in `~/.config/armadai/` (user) and `armadai.yaml` (project).

## Git Conventions

- **Branch model**: `master` (releases), `develop` (default/integration), `feature/*` branches
- **Conventional Commits** enforced by `.githooks/commit-msg` hook and CI. Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **PR process**: Always squash merge to `develop`. Before merging: check for Dependabot PRs, verify CI passes (all 6 checks: fmt, clippy, test, build, conventional commits, audit).
- Enable hooks after clone: `git config core.hooksPath .githooks`

## Language

All communication with the user must be in **French**. Code, comments, and commit messages remain in English.
