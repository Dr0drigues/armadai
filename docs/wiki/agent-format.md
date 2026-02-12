# Agent Format

Agents are defined as Markdown files in the `agents/` directory. Each file represents one specialized agent.

## File Structure

```markdown
# Agent Name          ← H1: required, becomes the agent's display name

## Metadata           ← H2: required, key-value configuration
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.3

## System Prompt      ← H2: required, the system prompt sent to the model

Your role and behavior description here.

## Instructions       ← H2: optional, step-by-step execution guidance

1. Step one
2. Step two

## Output Format      ← H2: optional, expected output structure

Description of the expected output format.

## Pipeline           ← H2: optional, agents to chain after this one
- next-agent-a
- next-agent-b

## Context            ← H2: optional, additional runtime context

Extra context injected at execution time.
```

## Sections Reference

### Metadata (required)

Key-value pairs configuring the agent's technical behavior.

| Key | Type | Required | Default | Description |
|---|---|---|---|---|
| `provider` | string | Yes | — | Provider type: `anthropic`, `openai`, `google`, `cli`, `proxy` |
| `model` | string | API providers | — | Model identifier (e.g. `claude-sonnet-4-5-20250929`) |
| `command` | string | CLI provider | — | CLI command to execute |
| `args` | list | No | — | CLI arguments: `["-p", "--model", "sonnet"]` |
| `temperature` | float | No | `0.7` | Sampling temperature (0.0 - 2.0) |
| `max_tokens` | int | No | — | Maximum output tokens |
| `timeout` | int | No | — | Execution timeout in seconds |
| `tags` | list | No | `[]` | Tags for filtering: `[dev, review]` |
| `stacks` | list | No | `[]` | Tech stacks: `[rust, typescript]` |
| `cost_limit` | float | No | — | Max cost per execution in USD |
| `rate_limit` | string | No | — | Rate limit: `"10/min"` |
| `context_window` | int | No | — | Context window size override |

### System Prompt (required)

The system prompt sent to the model. This defines the agent's identity, role, and behavioral boundaries.

### Instructions (optional)

Step-by-step instructions for how the agent should process input. Useful for complex multi-step tasks.

### Output Format (optional)

Description of the expected output structure. Helps the model produce consistent, parseable results.

### Pipeline (optional)

List of agent names to chain after this agent. Each agent receives the previous agent's output as input.

```markdown
## Pipeline
- test-writer
- doc-generator
```

### Context (optional)

Additional context injected at runtime. Can include project-specific information, coding standards, etc.

## Provider Types

### API Providers (`anthropic`, `openai`, `google`)

Send requests to LLM APIs. Require `model` field.

```markdown
## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.3
- max_tokens: 4096
```

### CLI Provider (`cli`)

Execute any command-line tool. Require `command` field.

```markdown
## Metadata
- provider: cli
- command: claude
- args: ["-p", "--output-format", "json"]
- timeout: 300
```

### Proxy Provider (`proxy`)

Route through an OpenAI-compatible proxy (LiteLLM, OpenRouter).

```markdown
## Metadata
- provider: proxy
- model: anthropic/claude-sonnet-4-5-20250929
```

## File Organization

Agents can be organized in subdirectories:

```
agents/
├── _coordinator.md       ← Hub agent (prefixed with _ for sorting)
├── code-reviewer.md
├── test-writer.md
├── examples/
│   ├── doc-generator.md
│   └── simple-chat.md
└── team-specific/
    └── deploy-checker.md
```

All `.md` files in `agents/` and subdirectories are loaded recursively.

## Validation

Validate agent configurations without making API calls:

```bash
# Validate all agents
armadai validate

# Validate a specific agent
armadai validate code-reviewer
```

Validation checks:
- Presence of required sections (H1 title, Metadata, System Prompt)
- Provider type consistency (API providers need `model`, CLI needs `command`)
- Temperature range (0.0 - 2.0)
