# Starter Packs

> Packs are written in the [legacy format](agent-format.md): `armadai init --pack <name>` converts
> one, like `armadai migrate`, into a project in the current directory
> ([Project format](project-format.md)). Packs are no longer installed into a user library.

Starter packs are curated bundles of agents, prompts and skills that you can install with a single
command, then compile for your assistant with `armadai link`.

## Usage

```bash
# Install a pack (agents, prompts, skills copied to the user library)
armadai init --pack <pack-name>

# Install a pack + create .armadai/config.yaml declaring its agents and prompts
armadai init --pack <pack-name> --project
```

This copies the pack's agents, prompts and skills into your user library
(`~/.config/armadai/agents/`, `~/.config/armadai/prompts/`, `~/.config/armadai/skills/`).

> The project config written by `--project` sets `link.target` only when a supported target can
> be inferred from the pack's agents (their `provider:`, read in file-name order; `anthropic`
> maps to `claude`). When none can, no `link.target` is written, `init` prints a hint, and
> `armadai link --target <claude|codex|opencode|copilot>` must be given.

## Available Packs

The seven packs below ship embedded in the binary. `armadai-dev` and `armadai-authoring` were
revised for the declarative refocus. The three CLI example agents of `fullstack`
(`claude-cli-reviewer`, `echo-reviewer`, `gemini-reviewer`) were written for the removed execution
engine: their `command`/`args` fields are parsed (and shown by `armadai inspect`) but nothing executes them.

### rust-dev

Rust development essentials: code review, test writing, debugging, and Rust conventions.

| Type | Name |
|---|---|
| Agent | `code-reviewer`, `test-writer`, `debug` |
| Prompt | `rust-conventions` |

### fullstack

General-purpose pack — code review, testing, documentation, and CLI examples.

| Type | Name |
|---|---|
| Agent | `code-reviewer`, `test-writer`, `doc-generator`, `claude-cli-reviewer`, `echo-reviewer`, `gemini-reviewer` |

### code-analysis-rust

Code analysis crew for Rust projects — a coordinator with scoped specialists.

| Type | Name |
|---|---|
| Agent | `lead-analyst` (coordinator), `rust-reviewer`, `rust-test-analyzer`, `rust-doc-writer`, `rust-security` |
| Prompt | `analysis-standards` |

### code-analysis-web

Code analysis crew for JS/TS/Node projects — a coordinator with scoped specialists.

| Type | Name |
|---|---|
| Agent | `lead-analyst` (coordinator), `web-reviewer`, `web-test-analyzer`, `web-doc-writer`, `web-security` |
| Prompt | `analysis-standards` |

### armadai-authoring

ArmadAI authoring pack — design, create, review and test agents, prompts, skills and starters.

| Type | Name |
|---|---|
| Agent | `authoring-lead` (coordinator), `authoring-architect`, `agent-builder`, `prompt-builder`, `skill-builder`, `starter-builder`, `authoring-reviewer`, `authoring-tester` |
| Prompt | `armadai-conventions`, `armadai-v012-updates` |
| Skill | `armadai-agent-authoring`, `armadai-prompt-authoring`, `armadai-skill-authoring`, `armadai-starter-authoring`, `armadai-orchestration-patterns` |

### armadai-dev

A Rust development team modelled on ArmadAI's own: a dev lead delegating to specialists of the
declaration model, the target adapters, the CLI, QA and releases.

| Type | Name |
|---|---|
| Agent | `dev-lead` (coordinator), `schema-specialist`, `adapter-specialist`, `cli-specialist`, `qa-specialist`, `release-manager` |
| Prompt | `rust-conventions` |

### pirate-crew

A fun demo pack with pirate-themed agents: a captain coordinator and their crew.

| Type | Name |
|---|---|
| Agent | `capitaine` (coordinator), `vigie`, `charpentier`, `cartographe` |
| Prompt | `code-nautique` |

## Pack Format

Each pack lives in its own directory with this structure:

```
<pack-name>/
├── pack.yaml           # Pack manifest
├── agents/             # Agent definitions (.md files)
│   ├── agent-one.md
│   └── agent-two.md
├── prompts/            # Prompt definitions (.md files)
│   └── conventions.md
└── skills/             # Skill directories (optional)
    └── my-skill/
        ├── SKILL.md
        └── references/
```

### pack.yaml

```yaml
name: rust-dev
description: Rust development agent pack — code review, test writing, and debugging
agents:
  - code-reviewer
  - test-writer
  - debug
prompts:
  - rust-conventions
skills:
  - armadai-agent-authoring   # Refers to built-in or bundled skill
```

Skills listed in `pack.yaml` but not bundled in the pack directory (e.g. built-in skills already
installed by `armadai init`) are silently skipped during installation.

`armadai validate <pack-dir>` checks that every agent and prompt `pack.yaml` lists exists and
parses.

## Embedded Versioning

Starter packs embedded in the binary are extracted to `~/.config/armadai/starters/` on first use.
Each extracted pack contains a `.armadai-version` marker file tracking the binary version at
extraction time. When the binary is updated, packs are re-extracted to stay in sync. The same
mechanism applies to built-in skills.

## Creating Custom Packs

1. Create a directory with your pack name, in one of the starter directories below
2. Add a `pack.yaml` manifest listing agents, prompts and skills
3. Add agent `.md` files in `agents/`
4. Add prompt `.md` files in `prompts/` (optional)
5. Add skill directories in `skills/` (optional)
6. Follow the [Agent Format](agent-format.md) for agent files

`armadai init --pack <path>` also accepts a path to a directory containing `pack.yaml`.

## Custom Starter Directories

### Priority order

1. **Built-in** — embedded in the binary
2. **Project-local** — `.armadai/starters/` in the current project
3. **User library** — `~/.config/armadai/starters/`
4. **Environment variable** — `ARMADAI_STARTERS_DIRS` (colon-separated paths)
5. **Config file** — `starters_dirs` in `~/.config/armadai/config.yaml`

### Environment variable

```bash
export ARMADAI_STARTERS_DIRS=/path/to/starters1:/path/to/starters2
```

### Config file

Add custom directories to `~/.config/armadai/config.yaml`:

```yaml
starters_dirs:
  - /path/to/my-starters
  - /shared/team-starters
```

Or use the CLI:

```bash
armadai config starters-dir add /path/to/my-starters
armadai config starters-dir list
armadai config starters-dir remove /path/to/my-starters
```

### Remote starter registries

When `armadai init --pack <name>` finds no local pack of that name, it syncs the remote starter
registries declared under `registries:` (in `~/.config/armadai/registries.yaml` or the project
config) once, then retries. A pack that exists locally never triggers a sync.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Templates](templates.md) — creating agents from templates
- [Link](link.md) — compiling a pack's agents for your assistant
