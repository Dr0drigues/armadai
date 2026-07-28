# Skill Format Specification

## Directory Structure

Each skill is a directory containing a `SKILL.md` file and optional subdirectories:

```
my-skill/
├── SKILL.md              # Required — skill definition
├── references/           # Optional — reference documentation files
│   ├── overview.md
│   ├── api.md
│   └── patterns.md
├── scripts/              # Optional — executable scripts
│   ├── setup.sh
│   └── validate.py
└── assets/               # Optional — static files (configs, schemas, etc.)
    └── schema.json
```

## SKILL.md Format

The `SKILL.md` file has two parts: an optional YAML frontmatter and a Markdown body.

### Frontmatter

```yaml
---
name: my-skill
description: One-line description of what this skill provides
version: "1.0"
tools:
  - docker
  - kubectl
---
```

#### Frontmatter Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | no | Skill name. Falls back to directory name if omitted |
| `description` | string | no | One-line description shown in `armadai skills list` |
| `version` | string | no | Semantic version of the skill content |
| `tools` | string[] | no | External tools this skill references or requires |

All frontmatter fields are optional. A minimal `SKILL.md` can be just a Markdown body with no frontmatter.

### Body

The Markdown body provides an overview of the skill. It should:
- Describe what the skill covers
- Explain when to use it
- Reference the `references/` directory for detailed content

```markdown
# My Skill

Overview of what this skill provides and when to use it.

See the `references/` directory for detailed documentation.
```

## Skill Location

Skills are resolved in this order:

1. **Project-local**: `<project>/skills/<name>/`
2. **User library**: `~/.config/armadai/skills/<name>/`

## Loading Behavior

When a skill is loaded (`Skill::load()`):

1. Reads and parses `SKILL.md` (frontmatter + body)
2. Lists all files in `scripts/` (non-recursive)
3. Lists all files in `references/` (non-recursive)
4. Lists all files in `assets/` (non-recursive)

The file listings are sorted alphabetically.

## Referencing Skills

In `armadai.yaml`:

```yaml
skills:
  # By name (resolved via project-local then user library)
  - name: platodin-reference

  # By explicit path
  - path: .armadai/skills/my-custom-skill/
```

## Naming Conventions

- Directory names use **kebab-case**: `platodin-reference`, `docker-compose`
- The `name` field in frontmatter should match the directory name
- Reference files use descriptive kebab-case names: `overview.md`, `api-patterns.md`
