# Agent File Format Specification

ArmadAI agents are defined as Markdown `.md` files. The parser uses `pulldown-cmark` to extract structured sections from the document.

## File Location

Agent files can be stored in:
- **User library**: `~/.config/armadai/agents/<name>.md`
- **Project-local**: `<project>/agents/<name>.md`
- **Arbitrary path**: Referenced via `path:` in `armadai.yaml`

## Required Sections

### `# Title` (H1)

The first H1 heading defines the agent's display name. Exactly one H1 is required.

```markdown
# Code Reviewer
```

### `## Metadata`

YAML-like key-value list defining the agent's configuration. Each field is on its own line, optionally prefixed with `- `.

```markdown
## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.5
- max_tokens: 4096
- tags: [review, quality]
- stacks: [rust, typescript]
```

#### Metadata Fields

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `provider` | string | **yes** | — | Provider backend: `anthropic`, `openai`, `google`, `gemini`, `cli`, `proxy`, `aider` |
| `model` | string | for API providers | — | Model identifier (e.g., `claude-sonnet-4-5-20250929`, `gpt-4o`) |
| `command` | string | for `cli` provider | — | CLI command to execute |
| `args` | string[] | no | — | Arguments for CLI command: `[--flag, value]` |
| `temperature` | float | no | `0.7` | Sampling temperature, range `0.0` to `2.0` |
| `max_tokens` | integer | no | — | Maximum tokens in response |
| `timeout` | integer | no | — | Timeout in seconds (mainly for CLI provider) |
| `tags` | string[] | no | `[]` | Categorization tags: `[review, security, coordinator]` |
| `stacks` | string[] | no | `[]` | Technology stacks: `[rust, java, python]` |
| `scope` | string[] | no | `[]` | File glob patterns this agent operates on: `[src/**/*.rs, tests/]` |
| `model_fallback` | string[] | no | `[]` | Fallback models tried in order if primary fails |
| `cost_limit` | float | no | — | Maximum cost in USD per run |
| `rate_limit` | string | no | — | Rate limiting spec (e.g., `10/min`) |
| `context_window` | integer | no | — | Override context window size in tokens |

#### List Syntax

Lists use bracket notation: `[item1, item2, "item with spaces"]`. Quotes are optional for simple values.

#### Validation Rules

- **CLI provider**: `command` field is required
- **API providers** (`anthropic`, `openai`, `google`, `proxy`): `model` field is required
- **Temperature**: Must be between `0.0` and `2.0`

### `## System Prompt`

The core instructions that define the agent's behavior. Supports full Markdown formatting (bold, lists, code blocks, etc.) which is preserved verbatim.

```markdown
## System Prompt

You are a senior code reviewer specialized in Rust.

Your responsibilities:
- **Correctness** — identify logic errors and edge cases
- **Security** — flag potential vulnerabilities
- **Performance** — spot unnecessary allocations and N+1 patterns
```

## Optional Sections

### `## Instructions`

Additional operational instructions, separate from the core system prompt.

```markdown
## Instructions

1. Read the provided code carefully
2. Classify each finding by severity (critical, warning, info)
3. Propose a concrete fix for each issue
```

### `## Output Format`

Expected format for the agent's response.

```markdown
## Output Format

Return a JSON array of findings:
\```json
[{"severity": "warning", "line": 42, "message": "..."}]
\```
```

### `## Context`

Additional context or background information injected into the agent.

```markdown
## Context

This project uses the Axum web framework with SQLite storage.
```

### `## Pipeline`

Defines agent chaining — which agents run after this one completes.

```markdown
## Pipeline
- test-writer
- code-reviewer
```

Each entry is the name of another agent. The output of the current agent is passed as input to the next agent in sequence.

## Complete Minimal Example

```markdown
# My Agent

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.7

## System Prompt

You are a helpful coding assistant.
```

## Naming Conventions

- File names use **kebab-case**: `code-reviewer.md`, `java-api-analyzer.md`
- The H1 title can use spaces and mixed case: `# Java API Analyzer`
- Tags and stacks use lowercase: `[rust, typescript]`
