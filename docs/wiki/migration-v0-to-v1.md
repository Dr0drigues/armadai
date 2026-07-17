# v0 → v1 Migration Guide

This guide lists the breaking changes between ArmadAI v0.x and v1.0.0, and
provides a step-by-step procedure for migrating an existing environment
(user agents in `~/.config/armadai/`, projects using `armadai.yaml` /
`.armadai/`).

A partial automation script is available:
[`scripts/migrate-v0-to-v1.sh`](../../scripts/migrate-v0-to-v1.sh). It only
covers mechanical, deterministic migrations (see below) — read this guide
before running it, especially for anything related to `fleet`.

## Table of Contents

- [1. Removal of `fleet`](#1-removal-of-fleet)
- [2. Non-canonical `provider` syntax](#2-non-canonical-provider-syntax)
- [3. Deprecated models](#3-deprecated-models)
- [4. `.armadai/` project format (recommended, non-blocking)](#4-armadai-project-format-recommended-non-blocking)
- [5. Diagnostic tools to use](#5-diagnostic-tools-to-use)
- [6. What's new in v1 that makes migration worthwhile](#6-whats-new-in-v1-that-makes-migration-worthwhile)
- [7. Migration checklist](#7-migration-checklist)

---

## 1. Removal of `fleet`

**Confirmed** — commit [`ded362f`](../../) `chore(core): remove legacy
fleet concept (#138)`, marked `BREAKING CHANGE` in its commit message.

The legacy **`fleet`** concept was fully removed in v1.0.0-beta.1:

- the `armadai fleet` command (create / link / list / show) no longer
  exists;
- the `FleetDefinition` type and the `src/core/fleet.rs` file were removed;
- the `~/.config/armadai/fleets/` directory is no longer created (removed
  from `ensure_config_dirs()`) and is no longer read anywhere in the code;
- **automatic detection of the legacy YAML format was also removed.**
  Before #138, `ProjectConfig::load()` detected a `fleet:` key at the top of
  the file and silently converted the old format into a `ProjectConfig`
  (with a warning). That safety net is gone: an `armadai.yaml` file still in
  the `fleet:` format will **no longer be recognized** — it will simply be
  interpreted (with empty default values) as a modern `ProjectConfig` with
  no agents.

Naming was also harmonized throughout the codebase and docs: *"AI agent
fleet orchestrator"* → *"AI agent orchestrator"*, and *"fleet of agents"* →
*"team of agents"* / *"agent library"*.

### BEFORE (v0) — legacy `fleet` format

```yaml
# armadai.yaml (old format, auto-detected before #138)
fleet: my-fleet
agents:
  - code-reviewer
  - test-writer
source: /home/user/armadai
```

```bash
# Commands that no longer exist
armadai fleet create my-fleet --all
armadai fleet link my-fleet
armadai fleet list
armadai fleet show my-fleet
```

### AFTER (v1) — modern `armadai.yaml`

```yaml
# armadai.yaml (or .armadai/config.yaml, see section 4)
agents:
  - name: code-reviewer
  - name: test-writer

link:
  target: claude
  coordinator: dev-lead
```

### Migration steps

1. Check whether you still have a legacy directory:
   ```bash
   ls ~/.config/armadai/fleets/ 2>/dev/null
   ```
   If it exists and contains files, note the fleet names and their
   associated agents (`agents:` in each fleet file) — they will never be
   read by ArmadAI again.
2. For each fleet, recreate an equivalent entry:
   - **Simple case** (a fleet = a group of agents used on an ad-hoc basis)
     → a project `armadai.yaml` listing the same agents under `agents:`.
   - **Multi-agent coordination case** (the fleet was used to orchestrate
     several agents together) → an `orchestration:` block with `teams:` and
     `coordinator:` (see the
     [orchestration guide](orchestration-guide.md)), or a team in the agent
     library (`~/.config/armadai/agents/`).
3. Recreate the links to your AI tools with `armadai link --target <cli>`
   (replaces `armadai fleet link`).
4. Remove `~/.config/armadai/fleets/` once the conversion has been verified
   (the migration script does *not* do this automatically — see below).

> The fleet → teams conversion is not mechanical: it depends on how each
> fleet was actually used. The migration script only **detects and
> reports** this directory; it never rewrites anything blindly.

---

## 2. Non-canonical `provider` syntax

**Confirmed** — `docs/SESSION-STATE.md` (item 44, agent library audit)
flags 6 core ArmadAI agents (`dev-lead`,
`core/provider/cli/ui/qa-specialist`) using the form `provider: cli claude`.
This form has **never been supported** by the parser
(`src/parser/metadata.rs` stores the raw value of the `provider` field
as-is) nor by the provider factory (`src/providers/factory.rs::create_provider`):
it produces the literal string `"cli claude"`, which does not match any
branch of the `match` (`"cli"`, `"anthropic"/"openai"/"google"/"proxy"`, or
a unified tool name like `"claude"`) and fails at runtime with `Unknown
provider: 'cli claude'`.

This is therefore not an API removal but a **configuration error
inherited** from old templates/documentation — widespread enough in the
agent library to warrant automated detection before moving to v1.

Two canonical forms exist today:

### BEFORE (v0, invalid syntax) — a single combined field

```markdown
## Metadata
- provider: cli claude
- timeout: 120
```

### AFTER (v1) — two canonical options

**Option A — unified tool name (recommended)**: ArmadAI automatically
detects whether the CLI is installed and falls back to the API otherwise.

```markdown
## Metadata
- provider: claude
- model: latest:pro
- timeout: 120
```

**Option B — explicit CLI mode**: if you need a custom command or
arguments.

```markdown
## Metadata
- provider: cli
- command: claude
- args: ["-p", "--model", "sonnet", "--output-format", "json"]
- timeout: 120
```

See [`docs/wiki/providers.md`](providers.md) for the complete list of
unified names (`claude`, `gemini`, `gpt`, `aider`) and their CLI/API
mapping.

### Migration steps

1. Search for affected agents:
   ```bash
   grep -rln '^- provider: cli [a-z]' ~/.config/armadai/agents/ 2>/dev/null
   ```
2. The `scripts/migrate-v0-to-v1.sh` script automatically rewrites
   `provider: cli <tool>` → `provider: <tool>` for the 4 known tools
   (`claude`, `gemini`, `gpt`, `aider`) — this is a purely textual,
   deterministic rewrite (no information loss: there were no separate
   `command:`/`args:` to preserve).
3. If you deliberately needed explicit CLI mode with custom `args:`, use
   option B above manually instead.

---

## 3. Deprecated models

**Confirmed** — `src/linker/model_aliases.rs`, function
`embedded_aliases()`. This is not an API removal but an ongoing
replacement: with each version, the embedded table grows and the old model
name is automatically resolved (with a warning) to its replacement.

| Deprecated model | Replacement |
|---|---|
| `gemini-3.0-pro` (name never actually shipped, anticipated by mistake) | `latest:pro` |
| `gemini-1.5-flash` | `gemini-2.5-flash` |
| `gemini-1.5-pro` | `gemini-2.0-pro` |
| `gemini-1.0-pro` | `gemini-2.0-pro` |
| `gpt-4-turbo` | `gpt-4o` |
| `gpt-3.5-turbo` | `gpt-4o-mini` |

Resolution is transitive (A→B→C becomes A→C) and protected against cycles.
You can also override/extend this table locally via
`~/.config/armadai/model-aliases.json`.

### BEFORE (v0)

```markdown
## Metadata
- provider: google
- model: gemini-3.0-pro
```

### AFTER (v1)

```markdown
## Metadata
- provider: google
- model: latest:pro
```

### Migration steps

ArmadAI already resolves these aliases automatically at runtime (with a
`tracing::warn!`) — so this is not blocking in itself. But to clean up your
agent files and stop relying on implicit resolution:

```bash
# Diagnostic only (writes nothing)
armadai models check --all

# In-place rewrite of deprecated models in .md files
armadai models update --all
```

`armadai models list` shows the registered projects these commands can
iterate over (`--all`).

---

## 4. `.armadai/` project format (recommended, non-blocking)

**Non-breaking, confirmed** — `src/core/project.rs`
(`check_dotarmadai_migration_hint`) and
[`docs/wiki/getting-started.md`](getting-started.md#legacy-format).
Introduced in v0.7.0, this format **never removed** support for the old
`armadai.yaml` at the project root: both coexist, with a simple
informational message (a "hint") encouraging migration.

```
.armadai/
├── config.yaml     # modern equivalent of armadai.yaml
├── agents/
├── prompts/
├── skills/
└── starters/
```

### Migration steps (optional)

```bash
armadai init --project
```

Then move the contents of your `armadai.yaml` to `.armadai/config.yaml`,
and your local `agents/`, `prompts/`, `skills/` directories to
`.armadai/agents/`, etc. The resource resolution order remains:
`.armadai/{type}/` → `{type}/` (root, legacy) →
`~/.config/armadai/{type}/` (user library).

---

## 5. Diagnostic tools to use

Before and after the migration, use the existing commands to validate your
configuration:

| Command | Purpose |
|---|---|
| `armadai validate [path]` | Lints a starter `pack.yaml` or a project config (consistent agents/teams, coordinator, existing referenced prompts/skills, valid `Triggers`). |
| `armadai models check --all` | Detects deprecated models in agents (diagnostic only). |
| `armadai models update --all` | Rewrites detected deprecated models in place. |
| `armadai audit [path] [--propose] [--deep]` | Static analysis of **native** agentic configs (Claude Code) — detects collisions, duplicates, broken references; `--propose` generates an installable ArmadAI pack, `--deep` adds an optional LLM pass. Does not replace this guide (different scope: native configs, not the legacy ArmadAI format), but useful as a complement once migration is done. |

---

## 6. What's new in v1 that makes migration worthwhile

Beyond cleaning up v0 format debt, v1.0.0 brings several major new
features:

- **Conversational shell** (`armadai shell`, since v0.12.0) — interactive
  multi-provider TUI (Claude, Gemini, Aider, Codex), prompt history,
  persistent sessions resumed with `/resume`, slash commands (`/help`,
  `/cost`, `/agents`, `/model`, `/switch`, …).
- **Multi-pattern orchestration** — beyond `direct` mode (single agent),
  `armadai.yaml` lets you declare:
  - **Blackboard** — agents working in parallel on a shared board,
    converging by consensus;
  - **Ring** — sequential review with weighted voting;
  - **Hierarchical** — coordinator + multi-level teams, parallel
    delegation (`tokio::spawn`).
  See the [orchestration guide](orchestration-guide.md) for full parameter
  details and pattern selection.
- **Headless mode for CI** (`armadai run --headless --json`,
  v1.0.0-beta.3) — non-interactive execution (skips the model-update
  prompt), structured JSONL event stream on stdout (`run_start`,
  `agent_start`, `agent_end`, `warning`, `result`, `error`), `--quiet` to
  keep only the final event, `--max-content N` to truncate intermediate
  event content. Dedicated exit codes: `0` success, `1` execution error,
  `2` usage error, `3` budget exhausted, `4` provider unavailable.
- **Dynamic model router** (`model: latest:auto`, v1.0.0-beta.3) — an
  agent's tier (`Fast`/`Pro`/`Max`) is chosen at runtime by static
  heuristics (input length, keywords, agent tags, budget), configurable via
  the `routing:` section of `armadai.yaml` (`length_thresholds`,
  `keywords`, `tags`, `budget_downgrade_ratio`).

### Complete v1 `armadai.yaml` example

```yaml
agents:
  - name: dev-lead
  - name: core-specialist

orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: dev-lead
  teams:
    - name: core-team
      agents: [core-specialist, qa-specialist]

routing:
  length_thresholds: { fast_max: 100, pro_max: 1000 }
  tags: { hot: max }

shell:
  default_provider: claude

link:
  target: claude
  coordinator: dev-lead
```

---

## 7. Migration checklist

- [ ] `ls ~/.config/armadai/fleets/` — if not empty, plan a manual
      conversion to `teams:`/agent library (section 1), **do not delete
      before verifying**.
- [ ] `scripts/migrate-v0-to-v1.sh` (dry-run) on `~/.config/armadai/agents/`
      and the current project(s) to spot `provider: cli <tool>` and
      deprecated models.
- [ ] `scripts/migrate-v0-to-v1.sh --apply` once the changes have been
      reviewed (`.bak` files are created automatically).
- [ ] `armadai models check --all` / `armadai models update --all`.
- [ ] `armadai validate` on each project/pack.
- [ ] (optional) `armadai init --project` then migrate to `.armadai/`.
- [ ] Re-read [the orchestration guide](orchestration-guide.md) if you were
      using multi-agent fleets, to choose the replacement pattern
      (`blackboard`, `ring`, or `hierarchical`).
