## v0.11.0 (2026-04-01)

### Feat

- TUI: inline search with `/`, column sort with `s`, new Orchestration tab (key `8`)
- Web UI: search/filter bars on all tabs, Mermaid.js orchestration topology diagram
- New API endpoint: `GET /api/orchestration/topology`
- Orchestration: parallel dispatch for hierarchical delegations via `tokio::spawn`
- Orchestration: cost budget enforcement with `token_budget` and `cost_limit` (graceful halt)
- New starter pack `armadai-dev`: 7 agents + `rust-conventions` prompt
- 4 Gemini API integration tests (gated behind `GOOGLE_API_KEY`)
- Gemini CLI E2E test script with 18 assertions (`tests/gemini_cli_e2e.sh`)
- `link.rs` validation improvements, 4 orchestration pattern examples, `demo-rust-team` config
- Orchestration user guide with decision matrix (`docs/wiki/orchestration-guide.md`)
- `pack.schema.json` for starter pack config validation

### Fix

- Remove unimplemented `--replay` flag from history command
- Serialise env-mutating tests with a shared mutex to prevent race conditions
- Fix `rustfmt` formatting in `web/mod.rs` route definition

### Refactor

- Harden error handling: 33 dangerous `unwrap()` replaced
- Document all unsafe blocks with `SAFETY` comments

## v0.10.6 (2026-03-31)

### Fix

- Revert coordination-only mode, always include coordinator prompt

## v0.10.5 (2026-03-28)

### Fix

- Skip coordinator system prompt when root context file exists
- Match coordinator name against slugified agent name

## v0.10.4 (2026-03-25)

### Feat

- add coordinator name and delegation instructions to generated CLAUDE.md/GEMINI.md

## v0.10.3 (2026-03-24)

### Feat

- add interactive shell setup (PATH + completions) on init and update

### Fix

- use ValueEnum for link/unlink --target autocompletion

## v0.10.0 (2026-03-23)

### Feat

- non-hierarchical orchestration: Blackboard (shared-state parallel agents) and Ring (sequential token-passing with voting)
- task-dependent classifier for automatic pattern selection (keyword heuristics + tag overlap)
- LLM agent wrappers with structured prompts (ACTION/TARGET/CONFIDENCE/CONTENT) and graceful fallback
- SQLite persistence for orchestration runs, board entries, ring contributions, and votes
- `--orchestrate blackboard|ring` CLI flag for manual pattern override
- new agent format sections: `## Triggers` (Blackboard) and `## Ring Config` (Ring)
- project-level orchestration config via `armadai.yaml` defaults (max_rounds, thresholds, budget, etc.)
- weighted voting in Ring pattern via `vote_weight` agent config
- position similarity grouping in vote resolution (Jaccard word-overlap)

### Refactor

- remove dead `coordinator.rs` and `pipeline.rs` execution code (hub & spoke pattern preserved for `link` command)
- remove global `serde/rc` feature, replaced by local `arc_vec_serde` module
- remove `PRAGMA foreign_keys = ON` from global schema (FK constraints kept for documentation)

### Fix

- prefix matching in classifier (tag "review" matches "reviewing", "infra" matches "infrastructure")
- parser fallback to Finding/Propose when LLM omits TARGET (no silent pointer to entry 0)

## v0.9.0 (2026-03-13)

### Feat

- add `armadai models check/update/list` commands for deprecated model management
- add project auto-registration on `run` and `link` commands
- add deprecated model alias resolution with embedded YAML registry
- auto-check deprecated models on `run`, `link`, and `init --project` with interactive prompt
- consolidate `example/` into `examples/` and migrate to `.armadai/` project format

## v0.8.0 (2026-02-24)

### Feat

- add Models catalog tab in TUI (key `7`) and Web UI (`/api/models`)
- add model resolution preview in agent detail views (TUI + Web)
- add `preview_model_resolution()` for link target model preview
- add sync cache-only helpers `load_models_cached` and `load_all_providers_cached`
- dynamic `{{model}}` placeholder in templates and starter-packs

## v0.7.0 (2026-02-19)

### Feat

- add .armadai/ project directory and ARMADAI_STARTERS_DIRS env var
- add `armadai config starters-dir` subcommand (list/add/remove)
- 3-level resource resolution: .armadai/ → project root → user library
- automatic migration hint for legacy armadai.yaml projects

## v0.6.1 (2026-02-18)

### Feat

- add starter-builder agent and armadai-starter-authoring skill

## v0.6.0 (2026-02-17)

### Feat

- embedded versioning, skill references content, init from UI (#72)

## v0.5.2 (2026-02-17)

### Feat

- detail views + starters tab + reorder tabs (TUI/Web) (#71)

## v0.5.1 (2026-02-17)

### Feat

- prompts & skills in TUI/web, fix template parser, add zsh-env skill

### Fix

- suppress tracing output in TUI for malformed agent files

## v0.5.0 (2026-02-17)

### Feat

- skills support in starter packs, add armadai-authoring pack
- agent mode (guided/autonomous) with project defaults, deprecate legacy fleet

## v0.4.0 (2026-02-13)

### Feat

- built-in skills meta, linker skills+prompts integration, unlink command
- add JSON Schema for armadai.yaml with IDE support

## v0.3.0 (2026-02-13)

### Feat

- model_fallback — automatic model retry chain (#66)

## v0.2.1 (2026-02-13)

### Fix

- correct awesome-copilot registry URL (#65)

## v0.2.0 (2026-02-13)

### Feat

- Google Gemini provider, code-analysis starters, scope & completions (#64)

## v0.1.3 (2026-02-12)

### Feat

- add models.dev registry for enriched model selection
- add OpenCode linker for link command

## v0.1.2 (2026-02-12)

### Feat

- add skills registry for GitHub-based discovery (#63)

### Fix

- embed starter packs in binary for installed usage

## v0.1.1 (2026-02-12)

### Feat

- migrate storage to SQLite and CI to cross-rs (#61)

## v0.1.0 (2026-02-12)

### Feat

- coordinator agent, pirate-crew demo & linker improvements (#60)
- add awesome-copilot registry integration (#58)
- add composable skills and prompts system (#56)
- add link command to generate native AI assistant configs (#55)
- add rich armadai.yaml project config format (#46)
- rebrand to ArmadAI + centralized config with XDG resolution (#53)
- rebrand to ArmadAI with install script and self-update (#52)
- add interactive agent creation and fleet management (#43)
- add web UI dashboard for fleet management (#40)
- implement SOPS + age secret management (#39)
- abstract provider configuration with unified tool names (#38)
- shell completion, TUI fleet management UX and demo agents (#37)
- implement cost tracking, history, and streaming TUI (#31)
- implement swarm run command and rate limiter (#30)
- add Anthropic API provider and enhance CLI provider (#29)
- implement swarm new, inspect and validate commands (#27)
- implement swarm list command (#25)
- initial project scaffolding

### Fix

- **ci**: add g++ cross-compiler for aarch64 RocksDB build
- switch reqwest from native-tls to rustls-tls for cross-compilation
- replace unsound serde_yml with serde_yaml_ng and update docs (#59)

### Perf

- feature flags to speed up CI builds (#26)
