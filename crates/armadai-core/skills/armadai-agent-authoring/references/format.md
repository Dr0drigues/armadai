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
- model: latest:pro
- temperature: 0.5
- tags: [review, quality]
- stacks: [rust, typescript]
```

#### Model Placeholders

Instead of hardcoding model names, use `latest:*` placeholders that `armadai link` resolves per target, offline, from the local model-catalogue cache or the embedded table (examples below; the actual ids depend on the cache):

| Placeholder | Tier | Resolves to (examples) |
|---|---|---|
| `latest` or `latest:pro` | Balanced (default) | `claude-sonnet-4-5-20250929`, `gpt-4o`, `gemini-2.5-pro` |
| `latest:fast` | Fast / cheap | `claude-haiku-4-5-20251001`, `gpt-4o-mini`, `gemini-2.5-flash` |
| `latest:max` | Most capable | `claude-opus-4-5-20250929`, `o3-pro`, `gemini-2.5-ultra` |

#### Metadata Fields

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `provider` | string | **yes** | — | Required for the agent to load. Names the vendor whose model catalogue resolves a `latest:*` tier: `claude`/`anthropic` (Anthropic), `gemini`/`google` (Google), `gpt`/`aider`/`codex`/`openai` (OpenAI); any other value uses the built-in Anthropic defaults |
| `model` | string | no | — | Model identifier or `latest:*` placeholder (e.g., `latest:pro`, `latest:fast`), written into the generated agent file after tier resolution |
| `temperature` | float | no | `0.7` | Sampling temperature; written by the OpenCode target when it differs from `0.7` |
| `tags` | string[] | no | `[]` | Categorization tags: `[review, security, coordinator]` |
| `stacks` | string[] | no | `[]` | Technology stacks: `[rust, java, python]` |
| `scope` | string[] | no | `[]` | File glob patterns this agent focuses on: `[src/**/*.rs, tests/]` (parsed; not written by any target) |
| `model_fallback` | string[] | no | `[]` | Fallback models; checked and rewritten by `armadai models` like `model` |

`command`, `args`, `max_tokens`, `timeout`, `cost_limit`, `rate_limit`, `context_window` and `mode`
are still parsed for compatibility, but they configured the removed execution engine and nothing
uses them now.

#### List Syntax

Lists use bracket notation: `[item1, item2, "item with spaces"]`. Quotes are optional for simple values.

#### Validation Rules

- **`provider`** is required
- A value that does not parse (e.g. `temperature: warm`) makes the whole file fail to load

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

### Engine-only sections

`## Pipeline`, `## Triggers` and `## Ring Config` are still parsed, but they configured the
removed execution engine (agent chaining, Blackboard, Ring) and nothing uses them now. Any other
`##` heading is dropped by the parser.

## Complete Minimal Example

```markdown
# My Agent

## Metadata
- provider: anthropic
- model: latest:pro
- temperature: 0.7

## System Prompt

You are a helpful coding assistant.
```

## Naming Conventions

- File names use **kebab-case**: `code-reviewer.md`, `java-api-analyzer.md`
- The H1 title can use spaces and mixed case: `# Java API Analyzer`
- Tags and stacks use lowercase: `[rust, typescript]`
