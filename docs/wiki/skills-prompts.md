# Skills & Prompts

> **Legacy format.** This page describes the format of projects before v1 (`.armadai/config.yaml`,
> `armadai.yaml`). It is still read by `armadai migrate`, which converts such a project, and by
> `armadai init --pack` for a starter pack in the legacy format (a `pack.yaml` without
> `format: 1`). Projects, agent templates and the built-in packs use the
> [Project format](project-format.md).

ArmadAI supports reusable **prompts** (Markdown fragments with optional frontmatter) and
**skills** (capabilities following the Agent Skills standard). A project lists the ones it uses in
`.armadai/config.yaml`, and `armadai link` copies them next to the generated agent files.

## Prompts

Prompts are reusable Markdown fragments that represent coding conventions, project rules, or
instructions shared by several agents.

### Format

```markdown
---
name: rust-conventions
description: Rust coding conventions
apply_to:
  - code-reviewer
  - test-writer
---

# Rust Conventions

- Use Rust edition 2024
- Prefer functional patterns over imperative loops
- Use `thiserror` for custom error types
- Use `anyhow::Result` for application-level errors
```

### Frontmatter Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | No | Prompt identifier (defaults to the file name) |
| `description` | string | No | Short description |
| `apply_to` | list | No | Agent names this prompt is meant for (`*` = every agent). |

### Storage Locations

1. **Project-local** — `.armadai/prompts/`, or referenced via `path:` in the project config
2. **User library** — `~/.config/armadai/prompts/<name>.md`
3. **Starter packs** — a legacy pack's prompts are converted into skills by `armadai init --pack`

### Usage in the project config

```yaml
prompts:
  - name: rust-conventions                     # from the project or user library
  - path: .armadai/prompts/project-rules.md    # explicit file
```

`armadai migrate` turns each prompt into a skill of the project (`armadai prompts` was removed
with the prompt concept).

## Skills

Skills follow the open [Agent Skills standard](https://agentskills.io/specification) and provide
capabilities with scripts and assets.

### Structure

```
my-skill/
├── SKILL.md         # Frontmatter + instructions
├── scripts/         # Executable scripts
├── references/      # Documentation
└── assets/          # Templates, resources
```

### SKILL.md Format

```markdown
---
name: code-review
description: Expert code review with security analysis
---

# Code Review Skill

1. Analyze the code structure
2. Check for security vulnerabilities
3. Provide actionable feedback
```

### Built-in Skills

ArmadAI ships built-in skills that `armadai init` installs into `~/.config/armadai/skills/`:
`armadai-agent-authoring`, `armadai-skill-authoring`, `armadai-starter-authoring`,
`armadai-orchestration-patterns` and `zsh-env-authoring`. The four `armadai-*` skills describe
the canonical format (agents, skills, native packs, flows and the [policy gate](policy-gate.md));
copy one into `.armadai/skills/` to give it to the agents of a project. In the canonical format a
former prompt is a skill.

Built-in skills are re-installed when the binary version changes (tracked via `.armadai-version`
marker files).

### Usage in the project config

```yaml
skills:
  - name: code-review                  # from ~/.config/armadai/skills/
  - path: .armadai/skills/deploy       # project-local skill directory
```

```bash
armadai skills list
armadai skills show code-review
```

## Skills Registry

Discover and install skills from GitHub repos without copying files manually.

### Sync remote sources

```bash
armadai skills sync
```

Clones or updates the skill sources (shallow clone) and builds a local search index at
`~/.config/armadai/skills-registry/`.

### Search skills

```bash
armadai skills search "testing"
armadai skills search "docker compose"
```

Multi-keyword AND search with weighted scoring (name > description > tags > source repo).

### Install a skill

```bash
# Install a specific skill from a repo
armadai skills add anthropics/skills/webapp-testing

# If the repo has only one skill, the skill name is optional
armadai skills add owner/repo

# Overwrite an existing skill
armadai skills add anthropics/skills/webapp-testing --force
```

The skill directory (SKILL.md + scripts/ + references/ + assets/) is copied to
`~/.config/armadai/skills/<skill-name>/`.

### Show skill info

```bash
armadai skills info webapp-testing
```

Displays metadata from the registry index and the SKILL.md content.

## How skills are linked (canonical format)

In a project in the [Project format](project-format.md), skills live in
`.armadai/skills/<id>/` (a legacy project's prompts become skills when `armadai migrate` converts
it). `armadai link` copies each skill folder whole, byte for byte, into the target's output
directory and records each file in the link manifest:

| Target | Skills |
|---|---|
| Claude Code | `.claude/skills/<id>/` (an agent listing it in `skills:` preloads it; the main session discovers it by its description) |
| Codex | `.codex/skills/<id>/` |
| OpenCode | `.opencode/skills/<id>/` |
| Copilot | `.github/skills/<id>/` |

The conformance report of `link` says, per target, how each skill and each agent's `skills:` were
rendered. Prompts no longer exist in the canonical format: there is no `prompts/` output.

## See Also

- [Agent Format](agent-format.md) — agent Markdown file reference
- [Link](link.md) — generating config for Claude Code, Codex, OpenCode and Copilot
- [Getting Started](getting-started.md) — installation and first steps
