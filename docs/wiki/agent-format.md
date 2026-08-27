# Agent Format

Agents are defined as Markdown files in the `agents/` directory. Each file represents one specialized agent.

A project can also declare agents in `.armadai/agents.yaml` instead of writing this file by hand —
see [Declarative agents](declarative-agents.md) for that format, including the one rule that
surprises people (a name that collides with a `.md` file, anywhere in your agent libraries, is
refused with no precedence either way) and an honest read on what it does and doesn't buy you.

## File Structure

```markdown
# Agent Name          ← H1: required, becomes the agent's display name

## Metadata           ← H2: required, key-value configuration
- provider: anthropic
- model: latest:pro
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

## Triggers           ← H2: optional, Blackboard activation rules

- requires: [finding]
- min_round: 1
- priority: 80

## Ring Config         ← H2: optional, Ring pattern settings

- role: challenger
- vote_weight: 1.5
```

## Sections Reference

### Section boundaries

A `##` section runs until the next heading of level **H1 or H2** — so it *owns* its
sub-headings. `###`, `####` and deeper are structure **inside** the section and stay
part of it:

```markdown
## System Prompt

An agent file has these required sections:

### Required Structure          ← part of System Prompt

- `# Name`, `## Metadata`, `## System Prompt`

### Optional Sections          ← still part of System Prompt

- `## Instructions`, `## Output Format`

## Instructions                 ← ends System Prompt
```

Two consequences worth knowing:

- **Use sub-headings freely — in prose sections.** Long prompts read better with them, and
  nothing is lost.
  (Before [#392](https://github.com/Dr0drigues/armadai/issues/392), a section ended at the
  next heading of *any* level, so everything from the first `###` onwards was silently
  dropped from the prompt `link` wrote and `run` sent.)
- **But not in the configuration sections.** `## Metadata`, `## Pipeline`, `## Triggers` and
  `## Ring Config` are read **line by line**, so a `###` block inside them no longer hides its
  contents — it configures the agent. Measured on a fixture whose `## Metadata` carried an
  `### Alternative setup (not in use)` block: the agent switched provider from `anthropic` to
  `openai` and temperature from `0.2` to `1.0`, with no warning (last value wins). The same
  applies to `requires`/`priority` in `## Triggers`, to `role`/`vote_weight` in
  `## Ring Config`, and to `## Pipeline`, where an agent listed under a `###` heading now runs.
  Worse, an unparsable value there is fatal: `link` reports `warn: failed to parse …`, skips
  that agent, and still **exits 0** — so it disappears from the generated config silently.
  Keep commented-out alternatives out of these four sections, or in a fenced code block.
- **`---` and `===` under a line are headings too.** A paragraph followed by a line of dashes
  or equals signs is a setext H2/H1 in CommonMark, so it ends the section exactly as `##`
  would — and the text after it is lost from the prompt. This predates #392 and is unchanged
  by it, but a `---` separator inside a long prompt is a natural thing to write. Use a fenced
  block, or a different separator.
- **A literal `##` line inside a section still ends it.** If your prompt needs to *show*
  an H2 (documenting this very format, for instance), put it in a fenced code block —
  fenced content is never read as a heading — or write it as bold text.

### Metadata (required)

Key-value pairs configuring the agent's technical behavior.

| Key | Type | Required | Default | Description |
|---|---|---|---|---|
| `provider` | string | Yes | — | Provider type: `anthropic`, `openai`, `google`, `cli`, `proxy` |
| `model` | string | API providers | — | Model identifier or `latest:*` placeholder (e.g. `latest:pro`, `latest:fast`) |
| `command` | string | CLI provider | — | CLI command to execute |
| `args` | list | No | — | CLI arguments: `["-p", "--model", "sonnet"]` |
| `temperature` | float | No | `0.7` | Sampling temperature (0.0 - 2.0) |
| `max_tokens` | int | No | — | Maximum output tokens |
| `timeout` | int | No | — | CLI-provider inactivity timeout in seconds (300s default for a non-orchestrated agent, 600s for one in an orchestrated run) — bounds the gap between two lines of subprocess output, not the call's total duration; a CLI call that keeps producing output survives past it, one that goes silent for this long is killed. Ignored by API providers. |
| `tags` | list | No | `[]` | Tags for filtering: `[dev, review]` |
| `stacks` | list | No | `[]` | Tech stacks: `[rust, typescript]` |
| `cost_limit` | float | No | — | Max cost per execution in USD |
| `rate_limit` | string | No | — | Rate limit: `"10/min"` |
| `context_window` | int | No | — | Context window size override |
| `orchestration` | string | No | — | Orchestration pattern: `blackboard`, `ring` |

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

### Triggers (optional, Blackboard pattern)

Controls when an agent activates during Blackboard orchestration. Agents without a Triggers section participate in every round.

```markdown
## Triggers
- requires: [finding]
- excludes: [synthesis]
- min_round: 1
- max_round: 4
- priority: 80
```

| Key | Type | Default | Description |
|---|---|---|---|
| `requires` | list | `[]` | Entry kinds that must be present on the board for agent to activate |
| `excludes` | list | `[]` | Entry kinds that prevent agent from activating |
| `min_round` | int | `0` | Earliest round the agent can contribute |
| `max_round` | int | — | Latest round the agent can contribute |
| `priority` | int | `50` | Agent priority (0-100, higher = runs earlier when budget is tight) |

Entry kinds: `finding`, `challenge`, `confirmation`, `synthesis`, `question`, `answer`.

### Ring Config (optional, Ring pattern)

Configures an agent's role and weight in Ring orchestration.

```markdown
## Ring Config
- role: challenger
- position: 2
- vote_weight: 1.5
```

| Key | Type | Default | Description |
|---|---|---|---|
| `role` | string | `specialist` | Role in the ring: `initiator`, `specialist`, `challenger`, `synthesizer` |
| `position` | int | — | Position in the ring order (0-indexed, auto-assigned if omitted) |
| `vote_weight` | float | `1.0` | Multiplier applied to this agent's vote during resolution |

## Provider Types

### Model Placeholders

Instead of hardcoding model names, use `latest:*` placeholders that resolve to the best available model for each provider:

| Placeholder | Tier | Examples |
|---|---|---|
| `latest` or `latest:pro` | Balanced (default) | `claude-sonnet-4-5-20250929`, `gpt-4o`, `gemini-2.5-pro` |
| `latest:fast` | Fast / cheap | `claude-haiku-4-5-20251001`, `gpt-4o-mini`, `gemini-2.5-flash` |
| `latest:max` | Most capable | `claude-opus-4-5-20250929`, `o3-pro`, `gemini-2.5-ultra` |

### API Providers (`anthropic`, `openai`, `google`)

Send requests to LLM APIs. Require `model` field.

```markdown
## Metadata
- provider: anthropic
- model: latest:pro
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
- model: latest:pro
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
