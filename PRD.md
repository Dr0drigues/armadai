# PRD — Centralized Configuration (`~/.config/armadai/`)

## Problem

All configuration paths in ArmadAI are hardcoded across ~18 call sites (`Path::new("agents")`, `Path::new("config")`, `"data/armadai.db"`, etc.). There is no centralized config module, no user-level configuration, and `config/settings.yaml` exists but is never loaded. Users cannot customize default provider, model, storage path, or rate limits without editing agent files individually.

## Goals

1. **Centralized config module** — Single source of truth for all path resolution and user settings.
2. **XDG compliance** — Respect `$XDG_CONFIG_HOME`, `$ARMADAI_CONFIG_DIR`, and `$HOME/.config/armadai` convention.
3. **Layered configuration** — Environment variables > user config file > built-in defaults.
4. **`armadai init` command** — Bootstrap the user config directory with sensible defaults.
5. **Backward compatibility** — Project-local `agents/`, `templates/`, `config/` directories still take precedence over global paths.

## Non-goals

- Hot-reloading of config files
- GUI config editor
- Config file format migration tool (just a hint message)
- Per-project config overrides (future issue #46)

## Solution

### Directory structure

```
~/.config/armadai/
├── config.yaml          # User defaults (provider, model, storage, rate limits, costs, logging)
├── providers.yaml       # Provider endpoints and model lists (non-sensitive)
├── agents/              # User agent library (global agents)
├── prompts/             # Composable prompt fragments
├── skills/              # Agent skills
├── fleets/              # Fleet definition files
└── registry/            # awesome-copilot registry cache
```

### Path resolution order

For `agents/`, `templates/`, `config/`:
1. Project-local directory (if it exists in cwd)
2. Global directory under `~/.config/armadai/`

For the config root:
1. `$ARMADAI_CONFIG_DIR` (explicit override)
2. `$XDG_CONFIG_HOME/armadai`
3. `$HOME/.config/armadai`

### Config layering

1. Built-in Rust defaults (`Default` impl)
2. `~/.config/armadai/config.yaml` (serde deserialization with `#[serde(default)]`)
3. Environment variables: `ARMADAI_PROVIDER`, `ARMADAI_MODEL`, `ARMADAI_TEMPERATURE`

### CLI commands

```
armadai init              # Create ~/.config/armadai/ with defaults
armadai init --force      # Overwrite existing config files
armadai init --project    # Create armadai.yaml in current directory
```

### Migration

When a project-local `config/settings.yaml` is detected, a hint is printed to stderr suggesting `armadai init`.

## Success criteria

- All hardcoded paths replaced with `AppPaths::resolve()` or config helpers
- `armadai init` creates the full directory tree with config files
- Existing projects with local `agents/` directory continue to work unchanged
- `ARMADAI_CONFIG_DIR` override works for testing and custom setups
- All tests pass, clippy clean in both feature modes
