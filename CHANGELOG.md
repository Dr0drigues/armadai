## v1.0.0-rc.3 (2026-07-21)

Third release candidate for 1.0.0. Completes the rc.3 scope (dynamic agent
routing + an event-based shell Workroom) and adds a substantial UX pass across
both terminal UIs plus genuinely useful run history/costs. Scope remains frozen
(event-sourcing and the declarative engine stay in 1.1/v2).

### New: C8 — Declarative agent routing

- **Deterministic routing** (#187, #188): select which roster agents run a task
  via **named routes** (`orchestration.routes:`) and/or **capability tags**
  (`--tags`), on top of `--route`. New `armadai run` flags `--route`, `--tags`,
  and `--dry-run` (preview the selection — agents + reason + pattern — with zero
  tokens). Targets blackboard/ring; hierarchical keeps its own topology.

### New: Event-based shell Workroom + drill-down

- **Marker-driven state machine** (#189, #190): the shell Workroom panel is now
  driven by the ArmadAI protocol markers in the stream (`ARMADAI_DELEGATE` /
  `META` / `END`) instead of fuzzy text matching — robust to markers echoed in
  recaps, resilient to chunk-split markers. `detect_mentions` removed.
- **Drill-down** (#194): `Ctrl+W` focuses the Workroom; `j`/`k` select an agent;
  `Enter` opens a detail popup (state, elapsed, last action, transition
  timeline); `Esc`/`Ctrl+W` exit. Works between AND during a streaming turn.

### New: Useful run history & costs

- **Absolute shared DB** (#208): the default storage path is now
  `~/.local/share/armadai/armadai.sqlite` (was a CWD-relative `data/…`), so runs
  and `armadai tui` share one database regardless of directory.
- **Per-run project** (#208): schema migration v2 tags every run (sequential and
  orchestrated) with its originating project; the dashboard History shows a
  PROJECT column.
- **Real CLI cost/tokens** (#208): the CLI provider now parses the underlying
  tool's stream-json output (`total_cost_usd` + usage) instead of reporting
  `$0.00`. Agents with explicit `args:` are respected verbatim.

### Changed: UX/UI pass across both TUIs

- **Shared semantic theme + light-terminal support** (#195): a `theme.rs` with
  named (terminal-adaptive) colors; the statusbar, detail views and markdown
  rendering are now legible on light terminals (were white-on-white / dark bar).
- **Safe Esc + shortcut bar** (#196): in the shell, Esc clears the input first,
  then requires a second Esc to quit (Ctrl+C still quits immediately); a
  persistent hint bar documents the shortcuts.
- **Dashboard polish** (#197, #191): scrollable detail views, the Costs tab
  brought to list conventions (selection/search/sort), `Ctrl+C` quits, and
  consistency fixes (legible Workroom colors, centralized cost/context
  formatting, deduped search bar & spinner).

### Fixed

- **Hierarchical orchestration was broken for kebab-keyed agents** (#198): the
  engine keyed agents by their H1 title instead of the config key, so the
  coordinator lookup failed (`No provider found for 'dev-lead'`). Now keyed by
  the roster key. Same class of fix as C8's routing (#192).
- **`--dry-run` now short-circuits without `--route`/`--tags`** (#192): a plain
  `--dry-run` previews the full roster instead of executing.
- **Workroom lifecycle** (#192, #193): CLI/provider pseudo-agents (e.g.
  `claude`) filtered out; final agent states persist between turns (reset moved
  to turn start).

## v1.0.0-rc.2 (2026-07-20)

Second release candidate for 1.0.0. Consolidates the post-rc.1 work: the
OH3/OH4 orchestration instrumentation, audit refinements, the web trace UI,
custom registries, and the full **C9 pattern mixing** feature. Scope remains
frozen (event-sourcing and the declarative-engine vision stay in 1.1/v2).

### New: C9 — Pattern mixing (hierarchical → nested blackboard/ring)

- **Engine** (#183): a hierarchical team can declare a nested sub-pattern
  (`teams[].pattern: blackboard|ring`) and run its agents as that sub-pattern
  instead of flat delegation. Budget/depth are shared with the parent run,
  metrics fold back, and the team lead gets an **arbitration turn** over the
  sub-run outcome (accept/refine/override). New `NestedStart`/`NestedEnd`
  JSONL events. A dedicated `NestedPattern` enum makes deeper nesting
  impossible by construction (single level).
- **Storage** (#184): a real schema migration mechanism (`PRAGMA
  user_version`) — the missing brick — relaxes the orchestration CHECK to
  allow `hierarchical`, adds `parent_run_id`, and a `delegation_events`
  table. Hierarchical runs and their nested sub-runs are now persisted and
  linked. Legacy databases migrate without data loss (foreign keys are
  disabled only around the table rebuild).
- **Exposition** (#185): the web trace UI renders hierarchical runs — a
  delegation-tree diagram plus expandable nested sub-runs (reusing the
  existing sequence/timeline views). The trace list shows root runs only.

### New: Custom registries (B2 Lot A)

- **Configurable registries** (#182): a `registries:` section (user
  `~/.config/armadai/registries.yaml` and/or project `armadai.yaml`) adds
  custom sources for agents, skills, and models on top of the built-in
  defaults (union, dedup). New CLI: `armadai registry sources
  list|add|remove <agents|skills|models> <url>`.

### New: Web orchestration traces (C6)

- **Traces tab** (#181): the web dashboard lists orchestration runs and shows
  a run's flow (Mermaid sequence diagram + timeline) via a detail endpoint.

### Changed

- **OH3/OH4 consolidation** (#179): JSONL run events (`Delegate`/`Vote`/
  `Board`) are now emitted across the orchestration engines, and
  `model: latest:auto` tier routing applies inside orchestration
  (blackboard/ring/hierarchical), not just single-agent runs.
- **Audit refinements** (#180): post-rc backlog cleanups for `armadai audit`.

## v1.0.0-rc.1 (2026-07-17)

First release candidate for 1.0.0. Scope frozen (event-sourcing and the
declarative-engine vision are deferred to 1.1/v2).

### Fixed

- **Beta.2/beta.3 technical debt resolved** (#176): exact metrics from
  `result_event` now used in Pipeline/Tandem modes (previously
  approximated), `deprecated_model` warning now emitted consistently in
  orchestration mode (previously single-agent-only), plus assorted
  cleanups (`content_out` clone avoidance, duplicate `exit_code_for` call,
  obsolete `#[allow(dead_code)]` removal, documented lint suppressions,
  redundant `#[serde(default)]` removal in `ProjectConfig`).

### New: v0 → v1 Migration Guide

- **[`docs/wiki/migration-v0-to-v1.md`](docs/wiki/migration-v0-to-v1.md)**:
  step-by-step guide covering breaking changes for v0.x users — removal of
  `fleet`, non-canonical `provider` syntax, deprecated models, the
  `.armadai/` project format, and diagnostic tooling.
- **[`scripts/migrate-v0-to-v1.sh`](scripts/migrate-v0-to-v1.sh)**: companion
  automation script for the mechanical, deterministic parts of the
  migration. Dry-run by default (nothing is written unless `--apply` is
  passed), backs up every file it touches, and never deletes anything.

## v1.0.0-beta.3 (2026-07-17)

Third beta of the 1.0.0 release. Adds two OpenHands-study features: a
CI-first headless mode for `armadai run` and a dynamic model-tier router.

### Feat

- **[OH3] Headless CI mode for `armadai run`** (#173): new `--headless`
  (non-interactive, CI exit codes, skips the model-updater prompt), `--json`
  (structured JSONL event stream on stdout), `--quiet` (result event only),
  and `--max-content N` (truncates intermediate event content) flags.
  Introduces `RunEvent`/`EventSink` (`NullSink`/`JsonlSink`) in
  `core/events.rs`, with short JSON keys for token economy. High-level
  events — `run_start`, `agent_start`, `agent_end`, `warning`, `result`,
  `error` — are instrumented on the single-agent path, `--pipe`, and
  orchestration (blackboard/ring/hierarchical) at the per-agent level. CI
  exit codes: `0` success, `1` execution error, `2` usage error, `3` budget
  exhaustion (`Err`-propagating paths only — orchestration budget halts
  remain a graceful partial result with exit `0`), `4` provider unavailable.
- **[OH4] Dynamic model-tier router (`model: latest:auto`)** (#174): an
  agent declaring `model: latest:auto` has its tier (`Fast`/`Pro`/`Max`)
  selected at run time by zero-token static heuristics — input length,
  keyword matching, agent tags (override), and budget (cap) — then resolved
  to a concrete model via `resolve_model_for_tier`. Signal precedence:
  tag override → `max(length, keywords)` → budget cap. Defaults are
  embedded and overridable per-field via `armadai.yaml > routing:`.
  `ModelTier` is now orderable (`Fast < Pro < Max`). Emits
  `RunEvent::Route { agent, tier, reason }` in `--json` mode (OH3 synergy).
  Scoped to the single-agent/`--pipe` path in this release; orchestration
  continues to use its own `agent_model` resolution.

## v1.0.0-beta.2 (2026-07-17)

Second beta of the 1.0.0 release. Resolves the four P0 blockers from the
v1.0.0 review and integrates the RUSTSEC security fixes carried over from the
dependency syncs.

### Fixed

- **[B1] Real streaming in Pipeline and Tandem modes** (#169): progressive
  output rendering with a final drain, concurrent stderr capture that is also
  shown on failure, and removal of the previous output duplication.
- **[B2] Cursor wrapping on multi-line input** (#170): correct cursor position
  on wrapped lines using `unicode-width`, with scroll support.
- **[B3] Poisoned mutex handling in the hierarchical orchestration engine**
  (#168): recovery and `Result`-based propagation instead of panicking.
- **[B4] I/O errors are logged instead of silently ignored** (#167): failures
  are surfaced via `tracing::warn!` / `tracing::debug!`.

### Security

- **RUSTSEC**: transitive advisories in `quick-xml` and `quinn-proto` resolved
  via dependency sync bumps.

## v0.12.0 (2026-04-09)

### New: `armadai shell` — Interactive Conversational TUI

- One-shot CLI runner supporting Gemini, Claude, Aider, Codex
- Rich markdown rendering with syntax-highlighted code blocks (tui-markdown)
- Mouse scroll, input history (↑↓), popup overlays for slash commands
- Spinner animation with elapsed time during loading
- Auto-detect provider from PATH, model from config
- UTF-8 cursor handling for accented characters
- Terminal restore on panic (panic hook)

### New: Wizard Setup

- Auto-detect project config and existing links
- Interactive init with starter pack selection
- Interactive link target selection
- Provider auth verification

### New: Slash Commands

- `/help`, `/clear`, `/cost`, `/agents`, `/model`, `/history` — session management
- `/providers`, `/switch <name>` — multi-provider support
- `/sessions`, `/resume <id>`, `/save` — persistent sessions
- `/quit` — exit shell
- All commands render as popup overlays with markdown formatting

### New: Multi-Provider Support

- Switch between Gemini, Claude, Aider, Codex mid-session
- Preserves conversation history across provider switches
- Provider detection with model name and pricing display

### New: Persistent Sessions

- Auto-save after each turn to `~/.config/armadai/sessions/`
- Resume conversations with `/resume`
- JSON-based storage (no SQLite dependency)
- Relative timestamps ("2 hours ago", "yesterday")

### New: Response Protocol

- `armadai-protocol` skill with standardized markers (END, DELEGATE, META)
- Injected automatically in all linker outputs (Gemini, Claude, Copilot, Codex, OpenCode)
- Shell parser module for marker extraction

### Web UI Improvements

- Light/dark mode toggle with localStorage persistence
- Markdown rendering for system prompts, skills, prompts (marked.js)
- Download `.md` buttons on agent, prompt, skill detail views
- Orchestration config display in starter detail views
- Graceful Ctrl+C shutdown

### Dependencies

- New: `tui-markdown` 0.3 (rich markdown rendering in TUI)

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
