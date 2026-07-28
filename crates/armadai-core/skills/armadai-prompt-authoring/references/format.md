# Prompt Format Specification

## File Format

A prompt is a single Markdown `.md` file with an optional YAML frontmatter followed by a Markdown body.

### With Frontmatter

```markdown
---
name: rust-conventions
description: Rust coding style and conventions
apply_to:
  - code-reviewer
  - test-writer
---

Always use snake_case for function and variable names.
Prefer `&str` over `String` in function parameters.
Use `thiserror` for library errors, `anyhow` for application errors.
```

### Without Frontmatter

```markdown
# General Guidelines

Be concise and specific in your responses.
Always include code examples when explaining concepts.
```

When no frontmatter is present, the prompt name is derived from the filename (stem without `.md`).

## Frontmatter Fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | no | Prompt name. Falls back to filename stem if omitted |
| `description` | string | no | One-line description shown in `armadai prompts list` |
| `apply_to` | string[] | no | Agent names this prompt auto-applies to. Use `*` for all agents |

All frontmatter fields are optional. The frontmatter block itself is optional.

## File Location

Prompts are resolved in this order:

1. **Project-local**: `<project>/prompts/<name>.md`
2. **User library**: `~/.config/armadai/prompts/<name>.md`

## The `apply_to` Mechanism

The `apply_to` field controls which agents receive this prompt automatically:

```yaml
apply_to:
  - code-reviewer     # Applies to the agent named "code-reviewer"
  - test-writer       # Also applies to "test-writer"
```

### Wildcard

Use `*` to apply to all agents:

```yaml
apply_to:
  - "*"
```

### No `apply_to`

When `apply_to` is empty or absent, the prompt is available but not auto-injected. It can still be referenced explicitly in `armadai.yaml`.

## Referencing Prompts

In `armadai.yaml`:

```yaml
prompts:
  # By name (resolved via project-local then user library)
  - name: rust-conventions

  # By explicit path
  - path: .armadai/prompts/style.md
```

## Naming Conventions

- Files use **kebab-case**: `rust-conventions.md`, `java-api-standards.md`
- The `name` field should match the filename (without `.md`)
- Descriptions should be one line, starting with a capital letter
