# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ArmadAI is a declarative compiler written in Rust (edition 2024). A team of agents (agents, skills, flows, gates, hooks) is described once, then compiled by `armadai link` into the native files of Claude Code, Codex, OpenCode and Copilot. ArmadAI does not run agents: there is no execution engine, no TUI, no web UI. The binary is named `armadai`; without arguments it prints the help.

Kept commands: `link`, `unlink`, `list`, `inspect`, `validate`, `new`, `init`, `prompts`, `skills`, `extract`, `models`, `config`, `update`, `completion`.

The refocus (what was removed, kept and frozen) is specified in `docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md`.

## Your role

You are the main coordinator of this project, you have knoledge and can help other agents to analyze, but your task is to mainly delegate to @dev-lead so that he can himself delegate to each agent.

## Build & Test Commands

There is a single configuration (no optional cargo features), so there is a single barrier, identical to CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
cargo build -p armadai-core   # portability guard: the core builds standalone

# Run a single test
cargo test --no-fail-fast test_name

# Run with debug logs
RUST_LOG=debug cargo run -- list
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, test, build (`cargo build --release` plus the core guard) and Conventional Commits; `audit.yml` runs the security audit. `.mise.toml` mirrors these as tasks.

## Architecture

**Flow**: declarative model (Markdown/YAML definitions) -> validation -> per-target adapters (linkers) -> native files for Claude Code, Codex, OpenCode and Copilot. `link` never touches the network (only `init --pack` on a local miss, `skills sync`/`add` and `update` do); `unlink` removes what `link` wrote, guided by the link manifest.

**Where things live**:
- `crates/armadai-core`: the domain — agent model and parser (`agent.rs`, `agent_decl.rs`, `agent_source.rs`, `parser/`, `project.rs`), skills, starters and prompts (embedded via `include_dir`), model resolution, and the declaration types of `orchestration/` (patterns, teams, and the Claude Code delegation `policy` gate). It also holds `test_support/` (env isolation).
- `crates/armadai`: the binary — `cli/` (commands), `linker/` (one linker per target: `claude.rs`, `codex.rs`, `opencode.rs`, `copilot.rs`, plus `manifest.rs`), `claude_adapter/`, `skills_registry/`, `starters_registry/`, `templates/`, and the integration tests and golden outputs in `tests/`.
- `crates/armadai-providers`: only a reader of the local models.dev catalogue cache (`model_registry/`; no network in this crate, nothing refreshes the cache) plus the tool-to-vendor table (`factory.rs`).
- `crates/armadai-secrets`: SOPS-based secret handling.
- `frozen/armadai-audit/`: the former `audit` command, frozen. It is outside the workspace and outside CI; do not build or edit it.

**Planned, not yet existing code**: a versioned schema with a validator and symbol table, `armadai migrate`, and adapters that emit a per-target conformity report (applied / partial / suggested). See the design spec above. Do not describe them as available.

**Non-obvious facts**:
- `crates/armadai` is **binary-only** — no `lib.rs`. `cargo test --lib` there returns "0 passed" with **no error**; use `--bin armadai` or `--test <name>`.
- There is no execution engine: `OrchestrationPattern { Direct, Blackboard, Ring, Hierarchical, Auto }` is a declaration type, not something ArmadAI runs, and the linkers do not read it. Its consumers are the parser, `validate`/`inspect` (through the parser), `validate_config` (run by `link` when `orchestration.enabled`) and the policy gate.
- Two delegation mechanisms exist, don't conflate them: the `<!--ARMADAI_DELEGATE-->` marker protocol, which `link` still writes into the root file but nothing consumes any more; and the `PreToolUse` policy gate (`armadai __claude-policy-gate`), which reads `.armadai/config.yaml` at runtime and is installed by hand (nothing installs or generates it, see `docs/wiki/policy-gate.md`).
- Test env isolation lives in `armadai_core::test_support` (behind the core's dev-only `test-support` cargo feature) and is **not reentrant** — two guards on one thread deadlock.

**Agent definition** lives in `~/.config/armadai/agents/` (user library) or project-local paths. Templates in `crates/armadai/templates/*.md` use `{{name}}`, `{{stack}}`, `{{description}}`, `{{model}}` placeholders.

**Config** lives in `~/.config/armadai/` (user) and `armadai.yaml` (project).

## Git Conventions

- **Branch model** (master-only): `master` (default/trunk), `release/*` (release-line stabilization), `feature/*` branches. No `develop`.
- **Conventional Commits** enforced by `.githooks/commit-msg` hook and CI. Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **PR process**: Always squash merge to `master`, and merge only when Dimitri's brief authorises it (otherwise open the PR and stop, see `.claude/CLAUDE.md`). Before merging: check for Dependabot PRs, verify CI passes (all 6 checks: fmt, clippy, test, build, conventional commits, audit — one single configuration, no feature matrix).
- Enable hooks after clone: `git config core.hooksPath .githooks`

## Language

All communication with the user must be in **French**. Code, comments, and commit messages remain in English.
