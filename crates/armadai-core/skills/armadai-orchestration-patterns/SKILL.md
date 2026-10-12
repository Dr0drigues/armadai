---
name: armadai-orchestration-patterns
description: Reference for declaring an ArmadAI delegation topology (the orchestration block of the project config), as checked by armadai validate and enforced by the Claude Code policy gate
version: 2.0.0
tools: []
---

# ArmadAI Orchestration Declarations

ArmadAI does not execute agents: the agents run inside the assistant a project is linked to
(Claude Code, Codex, OpenCode, Copilot). The `orchestration:` block of the project config
(`.armadai/config.yaml` or `armadai.yaml`) is therefore a **declaration** of who delegates to
whom, not an engine. Two things read it:

- `armadai validate` (and `armadai link` when `enabled: true`) checks that it is consistent: the
  coordinator and every team member must be declared agents.
- The Claude Code policy gate (the `armadai __claude-policy-gate` `PreToolUse` hook) enforces it
  when `policy: strict`. It is specific to Claude Code; nothing equivalent is generated for the
  other targets.

The removed execution engine used to run five patterns (Direct, Blackboard, Ring, Hierarchical,
Auto) with `armadai run --orchestrate`. That command no longer exists. The `pattern` field and the
engine limits (`max_depth`, `max_iterations`, `max_concurrency`, `timeout`, `max_rounds`,
`max_laps`, `consensus_threshold`, `token_budget`, `cost_limit`, `routes`, and per-team
`pattern`) are still parsed, but nothing uses them now.

## Declaring a topology

```yaml
orchestration:
  policy: strict            # off | strict — default: off
  coordinator: dev-lead
  teams:
    - agents: [schema-specialist, adapter-specialist, cli-specialist, qa-specialist]
  free_agents: [Explore, Plan]
```

Under `policy: strict`:

- the main thread may reach **only** `coordinator`;
- the coordinator may reach team leads, and the agents of lead-less teams;
- a lead may reach **its own** team;
- a plain specialist does not sub-delegate;
- `free_agents` are reachable from anywhere — assistance agents, outside the topology;
- **anything not declared is refused.**

`policy` is independent of `enabled`: `enabled` only makes `armadai link` validate the block
before writing anything. The gate reads `policy` alone. Every uncertainty (unreadable config, no
coordinator, `policy: off`) resolves to *allowed*.

## Sub-teams

```yaml
orchestration:
  coordinator: architect
  teams:
    - agents: [cloud-expert, ops-expert]   # direct reports of the coordinator
    - lead: java-lead                       # sub-team
      agents: [java-architect, java-security, java-test]
    - lead: node-lead                       # sub-team
      agents: [node-architect, node-graphql]
```

When to use sub-teams:
- More than ~6 specialists → group by domain
- Distinct domains (e.g., testing team separate from dev team)
- A domain needs its own coordination before synthesis

## When to declare a coordinator

| Situation | Declaration |
|-----------|-------------|
| One agent, or independent agents the user calls directly | no coordinator, no `orchestration:` block |
| A coordinator that dispatches to specialists | `coordinator` + `teams` |
| Many specialists, or distinct domains | `coordinator` + `teams` with `lead` sub-teams |

The coordinator agent is tagged `coordinator`. For a pack with such an agent,
`armadai init --pack <name> --project` writes `link.coordinator` and an `orchestration:` block
(coordinator + one team of the other agents). `link.coordinator` is what makes `armadai link`
write the coordinator's prompt into the target's root instructions file
(`.claude/CLAUDE.md` and its equivalents).

## Before switching to `policy: strict`

Turning on `strict` before declaring what you actually use refuses most delegations. Declare every
sub-agent your sessions call — the agents in `.claude/agents/` and the Claude Code built-ins you
rely on — in `teams` or `free_agents` first.

## Cross-references

- `docs/wiki/policy-gate.md` — the policy gate, its hook and its limits
- `docs/wiki/link.md` — the coordinator and the root instructions file
