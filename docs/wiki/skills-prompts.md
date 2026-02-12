# Skills & Prompts

ArmadAI supports reusable **prompts** (Markdown fragments with optional frontmatter) and **skills** (autonomous capabilities following the Agent Skills standard).

## Prompts

Prompts are reusable Markdown fragments that represent coding conventions, project rules, or instructions to inject into generated configs.

### Format

```markdown
---
name: rust-conventions
description: Rust coding conventions
apply_to: "*.rs"
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
| `name` | string | Yes | Prompt identifier |
| `description` | string | No | Short description |
| `apply_to` | string | No | File glob pattern (maps to `globs:` in Cursor, `applyTo:` in Copilot) |

### Storage Locations

1. **Project-local** — referenced via `path:` in `armadai.yaml`
2. **User library** — `~/.config/armadai/prompts/<name>.md`
3. **Starter packs** — installed via `armadai init --pack`

### Usage in armadai.yaml

```yaml
prompts:
  - rust-conventions                   # From ~/.config/armadai/prompts/
  - path: ./prompts/project-rules.md   # Project-local
```

## Skills

Skills follow the open [Agent Skills standard](https://agentskills.io/specification) and provide autonomous capabilities with scripts and assets.

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

### Usage in armadai.yaml

```yaml
skills:
  - code-review                        # From ~/.config/armadai/skills/
  - path: ./scripts/deploy             # Project-local skill directory
```

## Skills Registry

Discover and install skills from GitHub repos without copying files manually.

### Sync remote sources

```bash
armadai skills sync
```

Clones or updates the default skill sources (shallow clone) and builds a local search index at `~/.config/armadai/skills-registry/`.

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

The skill directory (SKILL.md + scripts/ + references/ + assets/) is copied to `~/.config/armadai/skills/<skill-name>/`.

### Show skill info

```bash
armadai skills info webapp-testing
```

Displays metadata from the registry index and the SKILL.md content.

## How Prompts are Linked

When running `armadai link`, prompts are included in the generated config:

| Target CLI | Prompt Output |
|---|---|
| Claude Code | Section in `CLAUDE.md` |
| Copilot | `.github/copilot-instructions.md` or `.github/agents/*.agent.md` |
| Gemini CLI | Section in `GEMINI.md` |

## See Also

- [Agent Format](agent-format.md) — agent Markdown file reference
- [Link Command](link.md) — generating config for AI CLIs
- [Registry](registry.md) — community agents
- [Getting Started](getting-started.md) — installation and first steps
