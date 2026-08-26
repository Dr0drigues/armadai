# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ArmadAI is an AI agent orchestrator written in Rust (edition 2024). Agents are defined as Markdown files and executed against any LLM provider (API or CLI tool). The binary is named `armadai`.

## Your role

You are the main coordinator of this project, you have knoledge and can help other agents to analyze, but your task is to mainly delegate to @dev-lead so that he can himself delegate to each agent.

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

CI runs clippy in **5 feature modes** to catch lints that only trip under one combo (see `.github/workflows/ci.yml`):
- `--no-default-features --features tui`
- `--no-default-features --features tui,providers-api`
- `--no-default-features --features tui,web,storage`
- `--no-default-features --features tui,web,storage,providers-api` — the crate's default feature set, and (since #355) the only mode where `web` and `providers-api` are both enabled, e.g. `web/api.rs`'s `refresh_models`
- `--no-default-features --features tui,storage,e2e-fake` — the only mode that compiles the gaveldrop e2e test surface

Tests run in 4 modes: `--no-default-features --features tui`; `--no-default-features --features tui,storage,e2e-fake,web` (this one also covers the SQLite storage paths, the gaveldrop e2e suite run via the `--test gaveldrop` target, and — since #350 — the `web/` module's own tests, previously compiled/linted by clippy's `tui,web,storage` combo but never executed by any test job); `--no-default-features --features tui,providers-api`; and `--no-default-features --features tui,web,storage,providers-api` (the default feature set — since #355, this is what exercises `web` and `providers-api` together, previously untested by any job). Build (`--release`) uses `--no-default-features --features tui,storage`.

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

**Where things live** — the one thing `ls` will not tell you, since OH7 (#252) split the
workspace: `crates/armadai` holds the binary-only surface (`cli/`, `linker/`, `tui/`, `shell/`,
`web/`, `audit/`, `registry/`, `skills_registry/`, `starters_registry/`, `claude_adapter/`),
while the domain lives in `crates/armadai-core` (incl. `parser/`, `orchestration/`,
`test_support/`), providers in `crates/armadai-providers` (incl. `model_registry/`), plus
`armadai-storage`, `armadai-secrets`, `armadai-fake`.

**Non-obvious facts** (the rest of the layout is readable from the tree):
- `crates/armadai` is **binary-only** — no `lib.rs`. `cargo test --lib` there returns
  "0 passed" with **no error**; use `--bin armadai` or `--test <name>`.
- There is no `Task`/`SharedContext`/`Coordinator`/`Pipeline` type. Orchestration is
  `OrchestrationPattern { Direct, Blackboard, Ring, Hierarchical, Auto }`, event-sourced under
  `core/orchestration/es/`.
- `routing.rs` is model-tier routing for `latest:auto` only — the `latest:pro`/`fast`/`max`
  aliases are **not** resolved on the `run` path (#376).
- Two delegation mechanisms, don't conflate them: the core engine's `@agent: task` text, and
  the linker-injected `<!--ARMADAI_DELEGATE-->` marker protocol (shell relay, Claude-only in
  practice).
- Test env isolation lives in `armadai_core::test_support` and is **not reentrant** — two
  guards on one thread deadlock.

**Provider trait** (`providers/traits.rs`):
```rust
trait Provider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(&self, request: CompletionRequest) -> Result<TokenStream>;
    fn metadata(&self) -> ProviderMetadata;
}
```

**Agent definition** lives in `~/.config/armadai/agents/` (user library) or project-local paths. Templates in `templates/*.md` use `{{name}}`, `{{stack}}`, `{{description}}`, `{{model}}` placeholders.

**Config** lives in `~/.config/armadai/` (user) and `armadai.yaml` (project).

## Git Conventions

- **Branch model** (master-only): `master` (default/trunk), `release/*` (release-line stabilization), `feature/*` branches. No `develop`.
- **Conventional Commits** enforced by `.githooks/commit-msg` hook and CI. Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **PR process**: Always squash merge to `master`. Before merging: check for Dependabot PRs, verify CI passes (all 6 checks: fmt, clippy, test, build, conventional commits, audit).
- Enable hooks after clone: `git config core.hooksPath .githooks`

## Language

All communication with the user must be in **French**. Code, comments, and commit messages remain in English.
