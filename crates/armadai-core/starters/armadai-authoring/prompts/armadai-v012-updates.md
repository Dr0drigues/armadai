---
name: armadai-v012-updates
description: What pack authors must account for in the current, declarative ArmadAI
apply_to:
  - authoring-lead
  - authoring-architect
  - agent-builder
  - starter-builder
---

# Authoring for the declarative ArmadAI

ArmadAI is a declarative compiler: agents, prompts and skills are described once, and `armadai link` compiles them into the native files of Claude Code, Codex, OpenCode and Copilot. ArmadAI does not execute agents: the agents run inside the assistant the pack is linked to. Earlier versions also shipped an execution engine (`armadai run`), an interactive shell with tandem and pipeline modes, and a Workroom panel; they were removed, so do not write packs, prompts or tests that rely on them.

## 1. Model tiers (abstraction)

**Do not hardcode model names in agent metadata.** Use tiers:

- `latest:fast` — cheap, fast (haiku, flash, 4o-mini)
- `latest:pro` — balanced (sonnet, pro, 4o)
- `latest:max` — best (opus, pro-max, o3)

Aliases: `latest:low/medium/high` map respectively.

`armadai link` resolves a tier into a concrete model id per target, offline, from the local model-catalogue cache or the embedded table. The agent's `provider:` names the vendor whose catalogue is read. Tier selection guidelines:
- Coordinator/lead → `latest:pro`
- Specialists doing lookups or transformations → `latest:fast`
- Architects, reviewers, critical reasoning → `latest:pro` or `latest:max`

## 2. Coordinators and sub-teams

A coordinator agent is tagged `coordinator`. `armadai link` writes its prompt into the target's root instructions file (`.claude/CLAUDE.md` and its equivalents) when `link.coordinator` names it.

The delegation topology is declared in the `orchestration:` block of the project config, with optional named sub-teams with leads:

```yaml
orchestration:
  coordinator: architect
  teams:
    - agents: [direct, reports, here]       # flat
    - lead: test-lead                        # sub-team
      agents: [vm-tester-linux, vm-tester-macos]
```

It is a declaration, not an engine: `armadai validate` checks it, and the Claude Code policy gate can enforce it (`policy: strict`). Use sub-teams when the pack has distinct domains or >6 agents.

## 3. Engine-only sections

`## Pipeline`, `## Triggers` and `## Ring Config`, and the metadata keys `command`, `args`, `max_tokens`, `timeout`, `cost_limit`, `rate_limit`, `context_window` and `mode`, are still parsed for compatibility, but they configured the removed execution engine and nothing uses them now. Do not add them to new agents. The `shell:` section of the project config is in the same situation.

## Checklist for new packs

When creating a pack, verify:
- [ ] Agents use model tiers (`latest:pro` etc.), not hardcoded models
- [ ] A coordinator, if any, is tagged `coordinator`
- [ ] Sub-teams used when appropriate (>6 agents or distinct domains)
- [ ] System prompts are self-contained
- [ ] Tests link the pack (`armadai link --target <target>`) and exercise each agent in the linked assistant
