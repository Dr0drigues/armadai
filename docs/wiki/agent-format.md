# Agent Format

> **Legacy format.** This page describes the format of projects before v1 (`.armadai/config.yaml`,
> `armadai.yaml`). It is still read by `armadai migrate`, which converts such a project, and
> internally by `armadai new` (templates) and `armadai init --pack` (starter packs). Projects use
> the [Project format](project-format.md).

Agents are defined as Markdown files in `.armadai/agents/` or `agents/` (or in your user library,
`~/.config/armadai/agents/`). Each file represents one specialized agent; `armadai link` compiles it
into the native agent file of each target (see [Link](link.md)).

A project can also declare agents in `.armadai/agents.yaml` instead of writing this file by hand —
see [Declarative agents](declarative-agents.md) for that format, including the one rule that
surprises people (a name that collides with a `.md` file, anywhere in your agent libraries, is
refused with no precedence either way).

## File Structure

```markdown
# Agent Name          ← H1: required, becomes the agent's name

## Metadata           ← H2: required, key-value configuration
- provider: claude
- model: latest:pro
- temperature: 0.3

## System Prompt      ← H2: required, the agent's prompt

Your role and behavior description here.

## Instructions       ← H2: optional, step-by-step guidance

1. Step one
2. Step two

## Output Format      ← H2: optional, expected output structure

Description of the expected output format.

## Context            ← H2: optional, additional context

Extra context for the agent.
```

The parser also accepts `## Pipeline`, `## Triggers` and `## Ring Config`. They configured the
removed execution engine; they are still parsed (an unparsable value there still fails the file,
see below) but nothing executes them; `armadai validate` still checks the syntax of
`## Triggers`.

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

Consequences worth knowing:

- **Use sub-headings freely — in prose sections.** Long prompts read better with them, and
  nothing is lost. (Before [#392](https://github.com/Dr0drigues/armadai/issues/392), a section
  ended at the next heading of *any* level, so everything from the first `###` onwards was
  silently dropped from the prompt `link` wrote.)
- **But not in the configuration sections.** `## Metadata` (and the three legacy sections above)
  are read **line by line**, so a `###` block inside them does not hide its contents — it
  configures the agent. Measured on a fixture whose `## Metadata` carried an
  `### Alternative setup (not in use)` block: the agent switched provider from `anthropic` to
  `openai` and temperature from `0.2` to `1.0` (last value wins). Worse, an unparsable value
  there is fatal: `link` reports `warn: failed to parse …`, skips that agent, and still
  **exits 0** — so it disappears from the generated config. Keep commented-out alternatives out
  of these sections, or in a fenced code block.

  Since [#396](https://github.com/Dr0drigues/armadai/issues/396) this is no longer silent.
  `## Metadata`, `## Triggers` and `## Ring Config` warn on stderr for every key they set
  twice, naming the file, the losing value and the winning one:

  ```text
  2026-08-28T05:56:43.104399Z  WARN armadai_core::parser::metadata: /abs/path/to/agents/dup.md: ## Metadata sets 'provider' again: 'anthropic' is overridden by 'openai' (the last value wins)
  ```

  It goes through `tracing`, so it carries a timestamp, a level and a target, and names the
  file by its absolute path. `RUST_LOG=off` silences it along with every other log line. Which
  value wins is unchanged — it is still the last.
- **`---` and `===` under a line are headings too.** A paragraph followed by a line of dashes
  or equals signs is a setext H2/H1 in CommonMark, so it ends the section exactly as `##`
  would — and the text after it is lost from the prompt. Use a fenced block, or a different
  separator.
- **A literal `##` line inside a section still ends it.** If your prompt needs to *show*
  an H2 (documenting this very format, for instance), put it in a fenced code block —
  fenced content is never read as a heading — or write it as bold text.

### What reaches the generated files

`## System Prompt`, `## Instructions`, `## Output Format` and `## Context` are composed into one
prompt body, in that order, each reintroduced under its own `##` heading and separated by a blank
line:

```
<System Prompt body>

## Instructions

<Instructions body>

## Output Format

<Output Format body>

## Context

<Context body>
```

An absent section contributes nothing at all — no empty heading. Every per-agent file `link`
writes uses this one composition (`armadai_core::agent::compose_agent_prompt`).

Two things fall outside that sentence:

- **The coordinator's root instructions file.** The coordinator's prompt becomes the target's
  root instructions file (`.claude/CLAUDE.md` and its equivalents), and that file is *not*
  composed this way: it carries the system prompt and the `## Instructions` body — the body
  alone, without its heading — and drops `## Output Format` and `## Context` entirely. Put
  anything a coordinator must obey in its system prompt until issue #409 is fixed.
- **`##` headings the parser does not know.** The parser keys sections by their exact heading,
  case-insensitively, and knows eight: `Metadata`, `System Prompt`, `Instructions`,
  `Output Format`, `Pipeline`, `Context`, `Triggers`, `Ring Config`. Any other `##` is dropped
  in silence — nothing warns, the content simply never reaches a generated file. Put prose under
  one of the eight, or demote its heading to `###` so it stays inside the section above it.

The first non-empty line of `## System Prompt` is also used as the agent's description in the
generated files.

### Metadata (required)

Key-value pairs configuring the agent. The parser accepts the keys below; the third column says
what `armadai link` and the other kept commands do with each.

| Key | Type | Required | Used by |
|---|---|---|---|
| `provider` | string | Yes | Required to load the agent. Names the vendor whose model catalogue resolves a `latest:*` tier (see [Providers and models](providers.md)) |
| `model` | string | No | Written into the generated agent file (Claude Code, Codex, OpenCode), after tier resolution |
| `temperature` | float | No (default `0.7`) | Written by the OpenCode target when it differs from `0.7` |
| `tags` | list | No | Filtering in `armadai list --tags` |
| `stacks` | list | No | Filtering in `armadai list --stack` |
| `model_fallback` | list | No | Checked and rewritten by `armadai models` like `model`. Plural alias `model_fallbacks` accepted |
| `scope` | list | No | Parsed; not written by any target |
| `orchestration` | string | No | Parsed (`direct`, `blackboard`, `ring`, `hierarchical`, `auto`); no kept command reads this per-agent field. It is not the project config's `orchestration.pattern`, which `link` reads (see [Policy gate](policy-gate.md)) |
| `command`, `args`, `max_tokens`, `timeout`, `cost_limit`, `rate_limit`, `context_window`, `mode` | — | No | Parsed for compatibility; they configured the removed execution engine and nothing executes them (`armadai inspect` still displays `command`, `args`, `max_tokens`, `timeout`) |

A value that does not parse (e.g. `temperature: warm`) still makes the whole file fail to load.
The model of these fields will be replaced by a versioned schema (planned, see the
[step 0 spec](https://github.com/Dr0drigues/armadai/blob/master/docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md)).

### Model placeholders

Instead of hardcoding model names, use `latest:*` placeholders that `link` resolves per target:

| Placeholder | Tier |
|---|---|
| `latest` or `latest:pro` | Balanced (default) |
| `latest:fast` | Fast / cheap |
| `latest:max` | Most capable |

How a tier becomes a concrete id, and which vendor's catalogue is read, is described in
[Providers and models](providers.md).

## File Organization

Agents can be organized in subdirectories:

```
agents/
├── _coordinator.md       ← Hub agent (prefixed with _ for sorting)
├── code-reviewer.md
├── test-writer.md
└── team-specific/
    └── deploy-checker.md
```

All `.md` files in `agents/` and subdirectories are loaded recursively.

## Validation

```bash
# Validate the project config in the current directory
armadai validate

# Validate a starter pack or another project
armadai validate path/to/pack-or-project

# Show how one agent was parsed
armadai inspect code-reviewer
```

`validate` checks that every agent, prompt and skill the config (or `pack.yaml`) references can be
found and parsed, that the `orchestration:` block is consistent, and that `link.coordinator`
names a declared agent.
