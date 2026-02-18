# Starter Pack Format Specification

## Directory Structure

A starter pack is a directory containing a `pack.yaml` manifest and subdirectories for its content:

```
my-pack/
├── pack.yaml              # Manifest (required)
├── agents/                # Agent .md files
│   ├── coordinator.md
│   ├── specialist-a.md
│   └── specialist-b.md
├── prompts/               # Prompt .md files
│   └── shared-conventions.md
└── skills/                # Skill directories (bundled only)
    └── my-custom-skill/
        ├── SKILL.md
        └── references/
            └── ...
```

## `pack.yaml` Format

The manifest declares the pack's metadata and content inventory:

```yaml
name: my-pack
description: "Short description of what this pack provides"
agents: [coordinator, specialist-a, specialist-b]
prompts: [shared-conventions]
skills: [my-custom-skill, some-builtin-skill]
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Pack identifier, kebab-case |
| `description` | string | yes | Short human-readable description |
| `agents` | list of strings | no | Agent names (without `.md` extension) |
| `prompts` | list of strings | no | Prompt names (without `.md` extension) |
| `skills` | list of strings | no | Skill directory names |

### Naming Conventions

- Pack name: kebab-case (e.g., `rust-dev`, `code-analysis-web`)
- Agent filenames: kebab-case `.md` files matching the names in `agents` list
- Prompt filenames: kebab-case `.md` files matching the names in `prompts` list
- Skill directories: kebab-case directories matching the names in `skills` list

## Skills: Built-in vs Bundled

Skills listed in `pack.yaml` can be either:

1. **Bundled** — The skill directory exists inside the pack's `skills/` subdirectory. It will be copied to the user's skill library during installation.
2. **Referenced (built-in)** — The skill is NOT bundled in the pack. It is expected to already be installed (e.g., built-in skills shipped with ArmadAI). These are silently skipped during installation.

This allows packs to declare dependencies on built-in skills without duplicating them.

## Resolution Order

When `armadai` looks for starter packs, it checks these locations in order:

1. `./starters/` — Relative to CWD (development)
2. `$CARGO_MANIFEST_DIR/starters/` — Compile-time path (development)
3. Next to the binary — Packaged installs
4. `~/.config/armadai/starters/` — Extracted from embedded data on first use

The first location that contains a valid directory wins.

## Installation Behavior

When a user runs `armadai init --pack my-pack`:

1. Agent `.md` files are copied from `<pack>/agents/` to `~/.config/armadai/agents/`
2. Prompt `.md` files are copied from `<pack>/prompts/` to `~/.config/armadai/prompts/`
3. Skill directories are recursively copied from `<pack>/skills/` to `~/.config/armadai/skills/`
4. Files that already exist are **skipped** (unless `--force` is used)
5. Skills listed but not bundled in the pack are silently skipped
6. The `--force` flag overwrites existing files

## Embedding at Compile Time

All packs under the `starters/` directory at the project root are embedded into the binary via `include_dir!`. They are extracted to the user's config directory on first use, with per-pack version markers (`.armadai-version`) to handle updates.
