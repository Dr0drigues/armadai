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
whom, not an engine. Three things read it:

- `armadai validate` checks that the coordinator, the team leads and the team members are
  declared agents (it ignores `pattern`).
- `armadai link` checks the block's structure before writing, only when `enabled: true` and
  `pattern: hierarchical` (see below).
- The Claude Code policy gate (the `armadai __claude-policy-gate` `PreToolUse` hook) enforces it
  when `policy: strict`. It is specific to Claude Code; nothing equivalent is generated for the
  other targets.

The removed execution engine used to run five patterns (Direct, Blackboard, Ring, Hierarchical,
Auto) with `armadai run --orchestrate`. That command no longer exists, and nothing executes any
pattern now. The engine limits (`max_depth`, `max_iterations`, `max_concurrency`, `timeout`,
`max_rounds`, `max_laps`, `consensus_threshold`, `token_budget`, `cost_limit`, `routes`) are
still parsed, but nothing uses them. `pattern` (root and per team) is still read by `link`'s
check:

`enabled: true` with `pattern: hierarchical` (what `armadai init --pack <name> --project`
writes) makes `armadai link` check the block's structure before writing anything: a coordinator
is present, there is at least one team, no agent or lead appears twice, the coordinator is not
inside a team, and a team with its own `pattern` has a `lead`. With any other `pattern`, `link`
skips that check, and a team-level `pattern` under a non-hierarchical root makes `link` fail.
Only `armadai validate` checks that the coordinator, leads and team members are declared agents.
Nothing executes any pattern.

## Declaring a topology

```yaml
orchestration:
  enabled: true             # with pattern: hierarchical, link checks the structure
  pattern: hierarchical
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

`policy` is independent of `enabled` and `pattern`: the gate reads `policy` alone. Every uncertainty (unreadable config, no
coordinator, `policy: off`) resolves to *allowed*.

## Sub-teams

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
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
| A coordinator that dispatches to specialists | `enabled: true`, `pattern: hierarchical`, `coordinator` + `teams` |
| Many specialists, or distinct domains | the same, with `lead` sub-teams |

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
