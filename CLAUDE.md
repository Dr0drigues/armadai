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

CI runs clippy in **3 feature modes** to catch lints that only trip under one combo (see `.github/workflows/ci.yml`):
- `--no-default-features --features tui`
- `--no-default-features --features tui,providers-api`
- `--no-default-features --features tui,web,storage`

Tests run in 2 modes: `--no-default-features --features tui` and `--no-default-features --features tui,storage` (the latter also covers the `e2e` integration test target). Build (`--release`) uses `--no-default-features --features tui,storage`.

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
- `providers/` — `Provider` trait (in `traits.rs`) with `complete()` and `stream()` methods. Factory (`factory.rs`) constructs the right provider from agent metadata. Implementations: `api/anthropic.rs` (full), `api/google.rs` (full), `cli.rs` (full); `api/openai.rs` and `proxy.rs` are `todo!()` stubs.
- `core/` — Domain types: `Agent`, `AgentMetadata`, `PipelineConfig`, `events::RunEvent`/`EventSink`, `routing.rs` (agent selection). Orchestration lives under `core/orchestration/` (`OrchestrationPattern { Direct, Blackboard, Ring, Hierarchical, Auto }`) with the event-sourced engine under `core/orchestration/es/`. There is no `Task`/`SharedContext`/`Coordinator`/`Pipeline` type.
- `core/project.rs` — Project config (`armadai.yaml`) with agent/prompt/skill resolution.
- `core/prompt.rs` — Composable prompt fragments with YAML frontmatter.
- `core/skill.rs` — Skills following the Agent Skills open standard (SKILL.md).
- `core/model_updater.rs` — Deprecated model detection, in-place update, and auto-check with interactive prompt (`auto_check_and_prompt()`). Called automatically by `run`, `link`, and `init`.
- `core/project_registry.rs` — JSON registry of known projects (auto-registered on `run`/`link`). Supports `prune` for stale entries.
- `core/starter.rs` — Starter packs: curated agent bundles installed via `armadai init --pack`.
- `core/embedded.rs` — Version-based extraction for embedded resources (`.armadai-version` marker).
- `core/events.rs` — `RunEvent`/`EventSink`: the provider-agnostic event stream emitted by a run (consumed by `--json` and by the TUI Workroom).
- `core/routing.rs` — C8 agent selection: named routes (`orchestration.routes`) and tag/stack matching for `--route`/`--tags`.
- `parser/frontmatter.rs` — Generic YAML frontmatter extraction reused by prompts and skills.
- `linker/` — Generates native config files for target AI CLIs. Trait `Linker` with one implementation per CLI (**claude, codex, copilot, gemini, opencode**). `model_resolution.rs` handles model remapping per target and exposes `preview_model_resolution()` for UI previews. `model_aliases.rs` maps deprecated model names to their replacements (embedded YAML registry). `armadai_protocol_block()` (in `mod.rs`) injects the `<!--ARMADAI_DELEGATE/META/END-->` marker protocol into generated configs (shell-relay delegation, see below).
- `registry/` — awesome-copilot integration. Sync, search, convert agents from the community catalog.
- `skills_registry/` — GitHub-based skills discovery. Sync repos, build search index, install skills (`sync.rs`, `cache.rs`, `search.rs`).
- `starters_registry/` — Remote starter pack registry (fetch/install curated starter bundles from external sources).
- `model_registry/` — Dynamic model catalog from models.dev. Fetches and caches model metadata (cost, context window) for enriched selection in `armadai new -i`. Gated behind `providers-api` for HTTP fetch, cache-only fallback otherwise. Sync cache-only helpers (`load_models_cached`, `load_all_providers_cached`) always available for TUI/Web.
- `storage/` — SQLite wrapper (via rusqlite). `schema.rs` defines the `runs` table, `queries.rs` has CRUD operations.
- `audit/` — `armadai audit`: agentic-asset adoption/collision audit engine (collision matrix, frontmatter passthrough).
- `tui/` — Ratatui-based terminal UI. `app.rs` holds state (incl. command palette), `views/` renders tabs (Agents/Prompts/Skills/Starters/History/Costs/Models + detail views + shortcuts bar + command palette overlay + orchestration/Workroom view), `widgets/` provides reusable components. Supports `i` key to init project from starters. Models tab (key `7`) shows cached model catalog from models.dev. Agent detail view includes model resolution preview for all link targets.
- `shell/` — `armadai shell`: conversational PTY shell relaying a native CLI. `app.rs`, `tui.rs`, `workroom.rs` (live run view), `json_runner.rs` (stream-json), `runner.rs`, `parser.rs`.
- `theme.rs` — Single shared theme (TUI + shell), color-tier aware (truecolor/256/16).
- `logging.rs` — Tracing setup, incl. a reload handle used to silence logs during the live Workroom view.
- `web/` — Axum-based web UI. The frontend is a **Svelte SPA** (`web/ui/`, built to `web/ui/dist/`) embedded at compile time via `include_dir!`. JSON API endpoints: `/api/agents(/{name})`, `/api/prompts(/{name})`, `/api/skills(/{name})`, `/api/starters(/{name}, /{name}/config)`, `/api/history`, `/api/costs`, `/api/models` (+ `/api/models/refresh`), `/api/orchestration/trace(/{run_id})`, `/api/orchestration/topology`.
- `secrets/` — SOPS + age encrypted secrets loader.

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

- **Branch model**: `master` (releases), `develop` (default/integration), `release/*` (release-line stabilization), `feature/*` branches
- **Conventional Commits** enforced by `.githooks/commit-msg` hook and CI. Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **PR process**: Always squash merge to `develop`. Before merging: check for Dependabot PRs, verify CI passes (all 6 checks: fmt, clippy, test, build, conventional commits, audit).
- Enable hooks after clone: `git config core.hooksPath .githooks`

## Language

All communication with the user must be in **French**. Code, comments, and commit messages remain in English.
