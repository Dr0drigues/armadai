# ArmadAI Agent Creation Guide

> Universal prompt for any AI assistant (Claude Code, Codex, OpenCode, Copilot, etc.)
> to help users create valid ArmadAI agent files.

## What is an ArmadAI Agent?

An ArmadAI agent is one Markdown file, `.armadai/agents/<id>.md` of a project (a folder holding
`.armadai/project.yaml`): a YAML frontmatter, then the prompt as the body. ArmadAI does not run
it: `armadai link --target <t>` compiles it into the native agent file of Claude Code, Codex,
OpenCode or Copilot, and that assistant runs it.

The quickest start is a command, then an edit of the file it writes:

```bash
armadai new agent rust-reviewer --template dev-review --stack rust
armadai new agent -i        # guided, in a terminal
```

## Agent File Format

```markdown
---
description: <when to use this agent, one specific sentence>
tier: pro
temperature: 0.3
tags: [tag1, tag2]
skills: [skill-id]
capabilities:
  write: deny
---
<The prompt: the agent's role, what it does and does not do.>

## Instructions

<Optional steps.>

## Output Format

<Optional description of the expected output.>
```

Frontmatter keys (any other key is an error):

| Key | Required | Value |
|---|---|---|
| `description` | yes | When to use the agent: the assistant picks a sub-agent by it |
| `title` | no | Display name |
| `tier` | no | `fast`, `pro` (default choice) or `max`; resolved to a model per target |
| `temperature` | no | A number; only targets that have the setting use it |
| `tags` | no | Lowercase words |
| `skills` | no | Ids of skills of the project (`.armadai/skills/<id>/SKILL.md`) |
| `capabilities` | no | `read`, `write`, `shell`, `web`, `delegate`: `allow`, `ask` or `deny` |
| `native` | no | Per-target keys passed as is (`claude`, `codex`, `copilot`, `opencode`) |

## Tier and temperature

| Tier | Use |
|---|---|
| `fast` | Lookups, transformations, cost-sensitive work |
| `pro` | Analysis, generation, review (the default choice) |
| `max` | Complex reasoning, architecture, critical reviews |

Temperature: 0.2 for review and analysis, 0.5 for general work, 0.7 for writing and ideas.

## Creation Flow

When helping a user create an ArmadAI agent, ask in order:

1. **Id**: kebab-case (`code-reviewer`): lowercase letters, digits, hyphens, at most 64.
2. **Description**: when should the assistant use this agent?
3. **Tier**: fast, pro or max.
4. **Prompt**: the role and expertise, what it must not do.
5. **Instructions** and **output format** (optional).
6. **Skills** of the project it uses, **capabilities** to restrict (optional).

## Complete Example

`.armadai/agents/rust-reviewer.md`:

```markdown
---
description: Reviews Rust code for bugs, unsafe patterns, performance and idioms
tier: pro
temperature: 0.3
tags: [rust, review]
---
You are an expert Rust code reviewer. You analyze code for bugs, unsafe patterns,
performance issues and idiomatic Rust violations; you do not rewrite the code.

## Instructions

1. Read the code carefully, understanding the overall structure
2. Check for common Rust pitfalls (unwrap abuse, unnecessary clones, lifetime issues)
3. Identify potential panics and error handling gaps
4. Evaluate performance (unnecessary allocations, missing iterators)

## Output Format

Structured review with sections:
- **Critical**: must-fix issues (bugs, panics, UB)
- **Warning**: should-fix issues (performance, bad patterns)
- **Info**: style and readability suggestions

Each item: severity, location, description, suggested fix.
```

## Validation and linking

Nothing needs to list the file: every `.md` under `.armadai/agents/` is an agent of the project.

```bash
armadai validate                            # file:line:column diagnostics
armadai inspect rust-reviewer               # the agent as ArmadAI reads it
armadai link --target claude --dry-run      # preview the generated files and the report
armadai link --target claude                # write them
```

To chain agents, write a flow: `armadai new flow review --template review-loop --agents
dev,rust-reviewer`, and see it with `armadai graph review`.
