# Swarm Agent Creation Guide

> Universal prompt for any AI assistant (Claude Code, Gemini CLI, Cursor, Copilot, etc.)
> to help users create valid swarm agent files.

## What is a Swarm Agent?

A swarm agent is a Markdown file (`.md`) that defines an AI-powered specialist. Each agent has a role, a provider, and a system prompt. Agents live in the `agents/` directory and can be run with `swarm run <name> "<input>"`.

## Agent File Format

```markdown
# Agent Display Name

## Metadata
- provider: <provider>
- model: <model>
- temperature: <0.0 - 2.0>
- max_tokens: <integer>
- tags: [tag1, tag2]
- stacks: [rust, python]

## System Prompt

<The system prompt defining the agent's role and behavior.>

## Instructions

<Optional step-by-step execution instructions.>

## Output Format

<Optional description of expected output structure.>
```

## Valid Providers

| Provider | Type | Requires |
|----------|------|----------|
| `claude` | Unified (CLI or API) | Auto-detects Claude CLI or falls back to Anthropic API |
| `gemini` | Unified (CLI or API) | Auto-detects Gemini CLI or falls back to Google API |
| `gpt` | Unified (CLI or API) | Auto-detects GPT CLI or falls back to OpenAI API |
| `aider` | Unified (CLI or API) | Auto-detects aider CLI or falls back to OpenAI API |
| `anthropic` | API only | `model` field, `ANTHROPIC_API_KEY` env var |
| `openai` | API only | `model` field, `OPENAI_API_KEY` env var |
| `google` | API only | `model` field, `GOOGLE_API_KEY` env var |
| `cli` | CLI only | `command` field (e.g. `command: claude`) |
| `proxy` | API proxy | `model` field, proxy running via `swarm up` |

## Known Models

**Anthropic**: `claude-opus-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`
**OpenAI**: `gpt-4o`, `gpt-4o-mini`, `o1`, `o3-mini`
**Google**: `gemini-2.0-flash`, `gemini-2.0-pro`

## Temperature Guide

| Preset | Value | Best for |
|--------|-------|----------|
| Focused | 0.2 | Code review, analysis, factual tasks |
| Balanced | 0.5 | General-purpose, moderate creativity |
| Creative | 0.7 | Writing, brainstorming, ideation |

Range: 0.0 (deterministic) to 2.0 (maximum randomness).

## Creation Flow

When helping a user create a swarm agent, ask these questions in order:

1. **Name**: What should the agent be called? (slug format: `my-agent-name`)
2. **Provider**: Which LLM provider? (claude, gemini, gpt, anthropic, openai, google, cli, proxy)
3. **Model**: Which model? (suggest based on provider — see Known Models above)
4. **Temperature**: What creativity level? (Focused 0.2 / Balanced 0.5 / Creative 0.7)
5. **Max tokens**: Set a limit? (optional, e.g. 4096)
6. **Tags**: Categories for filtering? (comma-separated, e.g. `dev, review`)
7. **Stacks**: Tech stacks? (comma-separated, e.g. `rust, typescript`)
8. **System prompt**: What is the agent's role and expertise?
9. **Instructions**: Step-by-step process? (optional)
10. **Output format**: Expected output structure? (optional)

## Naming Convention

- Use kebab-case: `code-reviewer`, `test-writer`, `doc-generator`
- Only letters, digits, and hyphens
- The filename becomes the agent ID: `agents/code-reviewer.md` → `swarm run code-reviewer`
- The H1 heading is the display name: `# Code Reviewer`

## Complete Example

```markdown
# Rust Code Reviewer

## Metadata
- provider: claude
- model: claude-sonnet-4-5-20250929
- temperature: 0.3
- max_tokens: 4096
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

## Validation

After creating the file, validate it:

```bash
swarm validate <agent-name>
```

This checks:
- Required sections are present (H1 title, Metadata, System Prompt)
- Provider/model consistency
- Temperature is within range (0.0 - 2.0)

## Running the Agent

```bash
# Direct input
swarm run code-reviewer "Review this function: fn add(a: i32, b: i32) -> i32 { a + b }"

# File input
swarm run code-reviewer @src/main.rs

# Pipeline (chain agents)
swarm run --pipe code-reviewer test-writer src/lib.rs
```
