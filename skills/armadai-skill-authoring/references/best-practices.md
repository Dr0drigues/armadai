# Skill Authoring Best Practices

## Skill vs Prompt

| Aspect | Skill | Prompt |
|---|---|---|
| **Structure** | Directory with SKILL.md + subdirectories | Single `.md` file |
| **Purpose** | Structured knowledge pack | Composable instruction fragment |
| **Content** | Multi-file reference documentation | Inline instructions |
| **Scope** | Self-contained domain knowledge | Targeted behavioral directives |
| **Targeting** | Referenced by name in `armadai.yaml` | Can use `apply_to` for auto-injection |

### When to Use a Skill

- You have **multiple reference documents** about a domain (framework docs, API specs)
- The content is **factual/reference** rather than behavioral instructions
- You want to provide **scripts or assets** alongside documentation
- The knowledge is **reusable** across multiple projects or agents

### When to Use a Prompt

- You need a **single instruction fragment** (coding conventions, style rules)
- The content is **behavioral** — tells the agent how to act
- You want **auto-injection** via `apply_to` patterns
- The instruction is **composable** — multiple prompts combine for one agent

## Reference File Organization

### One File Per Domain

Each reference file should cover one coherent domain:

```
references/
├── overview.md       # High-level architecture and concepts
├── web.md            # Web/HTTP specific docs
├── security.md       # Security patterns and config
├── testing.md        # Test utilities and patterns
└── configuration.md  # Configuration reference
```

### File Content Structure

Each reference file should follow a consistent structure:

```markdown
# Topic Name

Brief introduction (2-3 sentences).

## Key Concepts

Core concepts and terminology.

## Usage

How to use this in practice, with code examples.

## Common Patterns

Frequently used patterns and configurations.

## Pitfalls

Common mistakes and how to avoid them.
```

## Naming Conventions

- **Skill directory**: kebab-case, descriptive — `platodin-reference`, `ci-templates`
- **Reference files**: kebab-case, domain-focused — `overview.md`, `api-patterns.md`
- **Script files**: kebab-case with extension — `setup.sh`, `validate.py`
- **Asset files**: descriptive names — `schema.json`, `default-config.yaml`

## Versioning

Use semantic versioning in the `version` field:

```yaml
version: "1.0"    # Initial release
version: "1.1"    # Backward-compatible additions
version: "2.0"    # Breaking changes to skill structure
```

## Content Guidelines

1. **Be concrete** — Include code examples, not just descriptions
2. **Be accurate** — Reference actual API signatures and configuration keys
3. **Be organized** — Use consistent headings and formatting across reference files
4. **Be complete** — Cover the full domain; partial skills confuse agents
5. **Stay focused** — One skill = one domain. Don't mix unrelated topics
