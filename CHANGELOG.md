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
