---
name: armadai-orchestration-patterns
description: Reference for declaring who hands work to whom in an ArmadAI project (flows, coordinator, policy), as drawn by armadai graph and enforced by the Claude Code gate
metadata:
  version: "3.0"
---

# ArmadAI delegation topology

ArmadAI does not run agents: they run inside the assistant the project is linked to. The
topology is a **declaration** in three places of `.armadai/`:

- `flows/<id>.yaml`: who hands work to whom;
- `coordinator` in `project.yaml`: the agent that is the main session (`CLAUDE.md` for Claude
  Code), not a sub-agent;
- `policy` in `project.yaml`: whether Claude Code enforces the flows.

## Flows

```yaml
description: Write, review until approved, then ship
steps:
  write: { agent: dev }
  review: { agent: reviewer, states: [approved, rejected], on_failure: write }
  ship: { agent: release }
links:
  - { from: write, to: review }
  - { from: review, to: write, when: rejected }
  - { from: review, to: ship, when: approved }
```

- A step names an agent; its id may differ from the agent's.
- `when` is `done` (the default), `failed` or one of the step's `states`.
- `on_failure` (or one `when: failed` link) is the step's single failure path.
- `sequence: [a, b, c]` is a short form of a chain.
- An **entry** step is one no unconditional link leads to (a `when: rejected` or `on_failure` way
  back does not count; the `approved` end of a loop is not an entry). A flow with no entry is
  warned (`no-entry-step`), a step no entry reaches too (`unreachable-step`).

Templates: `armadai new flow <id> --template pipeline|review-loop|plan-build-verify|fan-out
--agents a,b,c`. `armadai graph [flow]` prints a flow as Mermaid.

## Coordinator

```yaml
schema_version: 1
coordinator: dev-lead
```

For Claude Code the coordinator's prompt becomes a block of the root `CLAUDE.md`, followed by the
list of the other agents and a description of every flow. Without a coordinator, the flows are
rendered nowhere (they stay a declaration, and `armadai new flow` says so) unless the policy is
strict.

## Policy

```yaml
policy:
  delegation: strict          # off (default) | strict
  free_agents: [Explore, Plan]
```

Under `strict`, `armadai link --target claude` installs a `PreToolUse` hook (the gate) that
refuses a delegation the flows do not declare:

- the main session: with a coordinator, every agent of a flow step; without one, the agents of
  the entry steps only;
- an agent: the agents of the steps its own steps lead to (links and `on_failure`);
- `free_agents`: from anywhere (project agents, or Claude Code's own `Explore`, `Plan`,
  `general-purpose`);
- the coordinator is never a target: it is no sub-agent.

The gate is specific to Claude Code; the other targets get the flows as guidance only, and the
conformance report of `armadai link` says so.
