---
name: armadai-v012-updates
description: Features from ArmadAI v0.12+ that pack authors must account for
apply_to:
  - authoring-lead
  - authoring-architect
  - agent-builder
  - starter-builder
---

# ArmadAI v0.12+ Features

When creating packs, always account for these recent additions. They change what agents can do and how they should be configured.

## 1. ArmadAI Shell

`armadai shell` (or just `armadai` since v0.12.1) launches an interactive TUI that wraps the provider CLI. Packs can be used:

- **Batch** (legacy): `armadai run <agent> "<prompt>"` — one-shot, returns stdout
- **Interactive**: `armadai shell` — multi-turn with session persistence, streaming, and coordination

Pack authors should ensure agents work in both modes. Keep system prompts self-contained; don't assume conversational context.

## 2. Shell config section

Packs can define a `shell:` section in `armadai.yaml` to preconfigure shell behavior:

```yaml
shell:
  default_provider: gemini
  default_model: latest:pro
  tandem: [...]
  pipeline:
    steps: [...]
```

Ship this in starters when the pack has a clear recommended workflow (e.g., analyze-then-review).

## 3. Model tiers (abstraction)

**Do not hardcode model names in agent metadata.** Use tiers:

- `latest:fast` — cheap, fast (haiku, flash, 4o-mini)
- `latest:pro` — balanced (sonnet, pro, 4o)
- `latest:max` — best (opus, pro-max, o3)

Aliases: `latest:low/medium/high` map respectively.

ArmadAI resolves tiers per provider at runtime via the model registry. Tier selection guidelines:
- Coordinator/lead → `latest:pro`
- Specialists doing lookups or transformations → `latest:fast`
- Architects, reviewers, critical reasoning → `latest:pro` or `latest:max`

## 4. Tandem and pipeline

Two new execution modes in the shell:

- **Tandem**: N providers receive the same prompt in parallel, user sees all responses
- **Pipeline**: ordered steps where each step's output feeds the next (analyze → generate → review)

Configure in `shell:` section. With a pipeline configured, every shell turn auto-runs the pipeline.

Use tandem when: comparative analysis, diverse perspectives, benchmarking.
Use pipeline when: generation + review workflow, staged analysis, quality improvement loop.

### Agent-backed pipeline steps

A pipeline step can reference a project agent directly instead of a raw provider:

```yaml
shell:
  pipeline:
    steps:
      - name: plan
        providers:
          - agent: architect       # loads architect.md's system prompt + metadata
      - name: review
        providers:
          - agent: reviewer
```

When `agent:` is set, it overrides `provider:`/`model:` — the agent's metadata defines the CLI and model, and its `## System Prompt` is prepended automatically. Mix `agent:` and `provider:` steps freely. Useful when the pack has specialized agents (architect, reviewer) that should drive a workflow without duplicating their prompts in the shell config.

## 5. Agent Workroom

A side panel in the shell shows real-time agent activity (delegating, working, done, idle). Triggered by:
- `<!--ARMADAI_DELEGATE:agent-name-->` markers in responses
- Agent name mentions in streamed text (heuristic)
- Project has multiple agents (auto-pins the panel)

Pack authors can write coordinator system prompts that produce DELEGATE markers explicitly for clearer workroom display.

## 6. Stream-JSON runner

ArmadAI parses providers' stream-JSON output for real metrics (tokens, cost, duration, model). Supported:
- Claude: `--output-format stream-json --verbose`
- Gemini: `-o stream-json`
- Codex: `--json`
- Copilot: `--output-format json`
- OpenCode: `--format json`
- Aider: text fallback

No pack changes needed — this works automatically. But know that costs shown are real (from CLI), not estimates.

## 7. Slash commands

The shell has these built-in commands (pack-agnostic but useful to mention in skills/prompts):

| Command | Purpose |
|---------|---------|
| `/help` | list commands |
| `/agents` | show pack agents + orchestration tree |
| `/model` | current provider and model |
| `/cost` | session cost |
| `/history` | prompt history |
| `/tandem [providers]` | run next message in tandem |
| `/pipeline [providers]` | run next message as pipeline |
| `/switch <provider>` | change provider mid-session |
| `/sessions`, `/resume <id>`, `/save` | session management |
| `/workroom` | toggle agent panel pin |
| `/clear` | clear conversation |
| `/quit` | exit |

## 8. Blackboard triggers — closed enum gotcha

When an agent uses `## Triggers` for Blackboard participation, only 5 fields are parsed:
`requires`, `excludes`, `min_round`, `max_round`, `priority`.

**`requires`/`excludes` are NOT free text.** They must be one of these 6 kinds:
`finding`, `challenge`, `confirmation`, `synthesis`, `question`, `answer`.

Common mistakes (all parse silently but never trigger):
- `requires: [audit-transverse]` — custom label, never matches
- `requires: [java-security-concern]` — custom label, never matches
- `match: "securiser endpoint java"` — field dropped, no pattern matching exists
- `priority: high` — must be integer 0–100

**If you need "route on user intent" or "activate on topic"**, that's not Blackboard — use Hierarchical with a coordinator whose system prompt decides `@agent:` delegation based on the request. Blackboard is "all agents reactively contribute to shared state", not "pattern-matched dispatch".

## 9. Hierarchical sub-teams

Orchestration now supports named sub-teams with leads:

```yaml
orchestration:
  pattern: hierarchical
  coordinator: architect
  teams:
    - agents: [direct, reports, here]       # flat
    - lead: test-lead                        # sub-team
      agents: [vm-tester-linux, vm-tester-macos]
```

Use sub-teams when pack has distinct domains or >6 agents. Each sub-team lead coordinates its specialists before reporting back.

## 10. Two complementary usage modes

- **`armadai run <agent>`** — invokes one agent directly, bypasses orchestration. Good for targeted tasks.
- **`armadai shell`** — interactive, triggers orchestration automatically if configured. Good for multi-step workflows.

Pack authors: design agents to be useful in both modes. Don't assume orchestration is always active.

## Checklist for new packs

When creating a pack, verify:
- [ ] Agents use model tiers (`latest:pro` etc.), not hardcoded models
- [ ] If ≥2 agents, an `orchestration:` section is present
- [ ] Sub-teams used when appropriate (>6 agents or distinct domains)
- [ ] Shell config provided if pack has a recommended workflow
- [ ] System prompts are self-contained (work in one-shot and interactive)
- [ ] Tests cover both `armadai run` and `armadai shell` modes
