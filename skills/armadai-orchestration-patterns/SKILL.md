---
name: armadai-orchestration-patterns
description: Reference for ArmadAI's 4 orchestration patterns with decision matrix and YAML examples
version: 1.0.0
tools: []
---

# ArmadAI Orchestration Patterns

ArmadAI supports 4 orchestration patterns for multi-agent workflows. Pick the right one based on your use case.

## Decision matrix

| Criteria | Direct | Blackboard | Ring | Hierarchical |
|----------|--------|------------|------|--------------|
| Number of agents | 1 | 2-5 | 2-5 | 3-20+ |
| Task independence | N/A | High | Low | Mixed |
| Need for consensus | No | No | Yes | No |
| Need for coordination | No | No | No | Yes |
| Depth of decomposition | None | Flat | Flat | Multi-level |
| Cost (relative) | $ | $$ | $$$ | $$–$$$$ |
| Latency | Low | Medium | High | Medium–High |

## Decision flow

```
Start → How many agents?
  ├─ 1 agent → Direct
  └─ 2+ agents → Subtasks independent?
      ├─ Yes, low overlap → Blackboard
      └─ No → Need review/consensus or coordination?
          ├─ Consensus → Ring
          └─ Coordination/delegation → Hierarchical
```

## Direct

Single agent, no orchestration. Use for simple tasks.

```yaml
agents:
  - name: my-agent
orchestration:
  enabled: true
  pattern: direct
```

## Blackboard

Agents work in parallel on shared state. Use when specialists analyze different domains.

```yaml
orchestration:
  enabled: true
  pattern: blackboard
  agents: [frontend-dev, backend-dev, devops]
  max_rounds: 5
  consensus_threshold: 0.75
  token_budget: 50000
```

Best for: frontend + backend + devops analysis, brainstorming, parallel domain analysis.

### `## Triggers` section (per-agent)

In Blackboard, each agent can declare **when it reacts**. Added as a `## Triggers` section in the agent's `.md`.

```markdown
## Triggers
- requires: [finding]       # kinds that MUST be present on the board
- excludes: [synthesis]     # kinds that PREVENT activation
- min_round: 1
- max_round: 4
- priority: 75              # integer 0–100, higher = earlier
```

**Accepted fields (only these — the parser ignores others):**
`requires`, `excludes`, `min_round`, `max_round`, `priority`.

**⚠ Kinds are a closed enum — free strings don't match.** Valid values for `requires`/`excludes`:

| Kind | Meaning |
|------|---------|
| `finding` | a domain observation |
| `challenge` | disputes another entry |
| `confirmation` | backs another entry |
| `synthesis` | closes a thread |
| `question` | open question |
| `answer` | answer to a question |

Custom labels like `[java-security]`, `[audit-transverse]`, `[platodin-java-expert]` **parse but never match** — they are silently dropped at runtime.

**What NOT to put in `## Triggers`:** `match:`, `patterns:`, `keywords:` (pattern-matching on user input is not a Blackboard feature — use Hierarchical with a coordinator for that).

### How to route on user intent
Blackboard does not route by prompt text. If you need "route by topic", use **Hierarchical**: the coordinator's system prompt decides which `@agent:` to call based on the request. Blackboard is for "all agents react to shared state in parallel."

## Ring

Sequential token-passing with voting. Use when agents must agree (code review).

```yaml
orchestration:
  enabled: true
  pattern: ring
  agents: [security-reviewer, performance-reviewer, architecture-reviewer]
  max_laps: 3
  consensus_threshold: 0.75
```

Best for: code review, decision validation, quality gates.
Tip: use odd number of reviewers for clearer votes.

## Hierarchical

Coordinator delegates to agents, optionally via team leads. Use for complex tasks.

### Flat hierarchy (coordinator + direct agents)

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: dev-lead
  teams:
    - agents: [shell-expert, dotfiles-expert, container-expert]
  max_depth: 3
```

### With sub-teams (coordinator + leads + specialists)

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: architect
  teams:
    - agents: [cloud-expert, ops-expert]   # direct reports
    - lead: java-lead                       # sub-team
      agents: [java-architect, java-security, java-test]
    - lead: node-lead                       # sub-team
      agents: [node-architect, node-graphql]
  max_depth: 5
  token_budget: 100000
```

When to use sub-teams:
- More than ~6 specialists → group by domain
- Distinct domains (e.g., testing team separate from dev team)
- A domain needs its own coordination before synthesis

## Cost control

Apply budgets to prevent runaway costs:

```yaml
orchestration:
  token_budget: 100000     # max total tokens — halt gracefully when exceeded
  cost_limit: 5.0          # max USD
```

On halt, the engine returns partial results with a notice — not an error.

## Cross-references

- `docs/wiki/orchestration.md` — full technical reference
- `docs/wiki/orchestration-guide.md` — user-facing guide with recipes
- `examples/orchestration-patterns/` — working examples per pattern
