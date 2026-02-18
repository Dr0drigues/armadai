# Link Command

The `armadai link` command generates native configuration files for your preferred AI CLI tool from your ArmadAI agent definitions.

## Usage

```bash
armadai link <target>           # Generate config for a specific CLI
armadai link <target> --dry-run # Preview without writing files
armadai link <target> --force   # Overwrite existing files
```

## Supported Targets

| CLI | Target | Generated Files |
|---|---|---|
| Claude Code | `claude` | `CLAUDE.md` + `.claude/commands/*.md` |
| GitHub Copilot | `copilot` | `.github/copilot-instructions.md` + `.github/agents/*.agent.md` |
| Gemini CLI | `gemini` | `GEMINI.md` |

More targets (Cursor, Aider, Codex, Windsurf, Cline) are planned.

## How It Works

1. **Load** — Reads `armadai.yaml` from the project root
2. **Resolve** — Resolves agent references (user library, registry, local paths)
3. **Transform** — Converts ArmadAI agents to the target CLI's native format
4. **Write** — Generates files in the appropriate directories

## Examples

### Claude Code

```bash
armadai link claude
```

Generates:
- `CLAUDE.md` — Project instructions with all agent system prompts and conventions
- `.claude/commands/<agent>.md` — One slash command per agent

### GitHub Copilot

```bash
armadai link copilot
```

Generates:
- `.github/copilot-instructions.md` — Global instructions
- `.github/agents/<agent>.agent.md` — One agent file per agent (with YAML frontmatter)

### Gemini CLI

```bash
armadai link gemini
```

Generates:
- `GEMINI.md` — Project instructions for Gemini CLI

## Conflict Detection

By default, `armadai link` warns before overwriting existing files. Use `--force` to skip confirmation, or `--dry-run` to preview first.

## Agent-to-Format Mapping

| ArmadAI Field | Claude Code | Copilot | Gemini |
|---|---|---|---|
| Agent name (H1) | Command name | `name:` frontmatter | Section heading |
| System Prompt | Markdown body | Markdown body | Markdown body |
| Instructions | Appended to body | Appended to body | Appended to body |
| Tags | Free text | `description:` | Free text |
| Model | N/A | `model:` frontmatter | N/A |

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Skills & Prompts](skills-prompts.md) — composable prompts and skills
