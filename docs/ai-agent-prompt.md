# ArmadAI Agent Creation Guide

> Universal prompt for any AI assistant (Claude Code, Codex, OpenCode, Copilot, etc.)
> to help users create valid ArmadAI agent files.

## What is an ArmadAI Agent?

An ArmadAI agent is a Markdown file (`.md`) that defines an AI specialist: a role, a system prompt
and a few metadata fields. Agents live in `.armadai/agents/`, `agents/` or the user library
(`~/.config/armadai/agents/`). ArmadAI does not run them: `armadai link --target <t>` compiles
them into the native agent files of Claude Code, Codex, OpenCode or Copilot, and that assistant
runs them.

## Agent File Format

```markdown
# Agent Display Name

## Metadata
- provider: <provider>
- model: <model>
- temperature: <0.0 - 2.0>
- tags: [tag1, tag2]
- stacks: [rust, python]

## System Prompt

<The system prompt defining the agent's role and behavior.>

## Instructions

<Optional step-by-step instructions.>

## Output Format

<Optional description of expected output structure.>
```

The first non-empty line of the system prompt becomes the agent's description in the generated
files: make it a one-line summary of the role.

## The `provider` field

`provider` is required. It names the vendor whose model catalogue resolves a `latest:*`
placeholder:

| Provider | Model vendor |
|----------|--------------|
| `claude` or `anthropic` | Anthropic |
| `gpt`, `openai`, `codex` or `aider` | OpenAI |
| `gemini` or `google` | Google |

## Models

Prefer `latest:*` placeholders to hardcoded model names:

| Placeholder | Tier |
|---|---|
| `latest` or `latest:pro` | Balanced (default) |
| `latest:fast` | Fast / cheap |
| `latest:max` | Most capable |

`link --target claude` always writes an Anthropic model and `link --target codex` an OpenAI model,
whatever the agent says; see the wiki page `docs/wiki/providers.md` for the exact rules.

## Temperature Guide

| Preset | Value | Best for |
|--------|-------|----------|
| Focused | 0.2 | Code review, analysis, factual tasks |
| Balanced | 0.5 | General-purpose, moderate creativity |
| Creative | 0.7 | Writing, brainstorming, ideation |

Range: 0.0 (deterministic) to 2.0 (maximum randomness). Only the OpenCode target writes it.

## Creation Flow

When helping a user create an ArmadAI agent, ask these questions in order:

1. **Name**: What should the agent be called? (slug format: `my-agent-name`)
2. **Provider**: Which model vendor? (claude, gpt, gemini)
3. **Model**: Which tier or model? (suggest a `latest:*` placeholder)
4. **Temperature**: What creativity level? (Focused 0.2 / Balanced 0.5 / Creative 0.7)
5. **Tags**: Categories for filtering? (comma-separated, e.g. `dev, review`)
6. **Stacks**: Tech stacks? (comma-separated, e.g. `rust, typescript`)
7. **System prompt**: What is the agent's role and expertise?
8. **Instructions**: Step-by-step process? (optional)
9. **Output format**: Expected output structure? (optional)

## Naming Convention

- Use kebab-case: `code-reviewer`, `test-writer`, `doc-generator`
- Only letters, digits, and hyphens
- The H1 heading is the display name: `# Code Reviewer`; its slug names the generated files
  (`.claude/agents/code-reviewer.md`)

## Complete Example

```markdown
# Rust Code Reviewer

## Metadata
- provider: claude
- model: latest:pro
- temperature: 0.3
- tags: [dev, review, quality]
- stacks: [rust]

## System Prompt

You are an expert Rust code reviewer. You analyze code for bugs, unsafe patterns,
performance issues, and idiomatic Rust violations. You provide constructive feedback
with specific line references and concrete fix suggestions.

## Instructions

1. Read the code carefully, understanding the overall structure
2. Check for common Rust pitfalls (unwrap abuse, unnecessary clones, lifetime issues)
3. Identify potential panics and error handling gaps
4. Evaluate performance (unnecessary allocations, missing iterators)
5. Verify adherence to Rust idioms and conventions

## Output Format

Structured review with sections:
- **Critical**: Must-fix issues (bugs, panics, UB)
- **Warning**: Should-fix issues (performance, bad patterns)
- **Info**: Style and readability suggestions

Each item: severity, location, description, suggested fix.
```

## Validation and linking

After creating the file, reference it in `.armadai/config.yaml` (`agents: [{ name: rust-code-reviewer }]`
or `- path: .armadai/agents/rust-code-reviewer.md`), then:

```bash
armadai inspect rust-code-reviewer          # show how the file was parsed
armadai validate                            # check the project config and its references
armadai link --target claude --dry-run      # preview the generated files
armadai link --target claude                # write them
```
