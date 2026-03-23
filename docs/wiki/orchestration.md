# Orchestration

ArmadAI supports two non-hierarchical multi-agent orchestration patterns: **Blackboard** and **Ring**. Both patterns allow agents to collaborate without a central coordinator.

## Quick Start

```bash
# Run two agents with Blackboard orchestration
armadai run agent-a --pipe agent-b --orchestrate blackboard

# Run three agents with Ring orchestration
armadai run agent-a --pipe agent-b agent-c --orchestrate ring
```

At least 2 agents are required for orchestration. Without `--orchestrate`, agents run sequentially (standard pipeline behavior).

## Patterns

### Blackboard

Shared-state pattern where agents work in parallel, reading from and writing to a shared board.

**How it works:**
1. Each round, all eligible agents receive a snapshot of the board
2. Agents contribute in parallel (with per-agent timeout)
3. Contributions are applied to the board as deltas
4. The engine checks for convergence (consensus, divergence, stability)
5. Repeats until halted (consensus, max rounds, or budget exhausted)

**Agent actions (via structured LLM prompt):**
- **Finding** — new observation or analysis (default fallback)
- **Challenge** — disagree with a specific entry (requires target index)
- **Confirmation** — agree with a specific entry (requires target index)
- **Synthesis** — combine multiple entries (requires source indices)
- **Question** — ask for clarification
- **Answer** — respond to a question (requires question index)

**Halt conditions:**
- Consensus: high ratio of confirmations (configurable threshold)
- Divergence: persistent challenges after round 3
- Stability: no new entries in a round
- Max rounds reached
- Token budget exhausted
- Majority halt proposal from agents

**Agent activation:** Control when agents participate using the `## Triggers` section:

```markdown
## Triggers
- requires: [finding]      # Only activate after findings exist
- excludes: [synthesis]     # Don't activate if synthesis already done
- min_round: 1              # Skip round 0
- max_round: 4              # Stop after round 4
- priority: 80              # Higher = runs earlier
```

### Ring

Sequential token-passing pattern with explicit voting and consensus resolution.

**How it works:**
1. **Circulation phase:** A token circulates through agents in order, each adding a contribution
2. Multiple laps allow agents to react to previous contributions
3. **Voting phase:** Each agent states a final position with confidence
4. **Resolution phase:** Votes are grouped by similarity and weighted

**Agent actions (via structured LLM prompt):**
- **Propose** — introduce a new idea (default fallback)
- **Enrich** — build on a previous contribution (requires target index)
- **Contest** — argue against a contribution (requires target index)
- **Endorse** — support a contribution (requires target index)
- **Synthesize** — combine insights from the discussion
- **Pass** — nothing to add

**Vote resolution outcomes:**
- **Consensus** — one group exceeds `consensus_threshold` (default 0.80)
- **Majority** — one group exceeds `majority_threshold` (default 0.60) but not consensus, includes dissenting positions
- **NoConsensus** — no group reaches majority, all positions reported

**Position grouping:** Votes with similar wording are grouped together using word-overlap Jaccard similarity (configurable via `similarity_threshold`, default 0.85).

**Weighted voting:** Configure via `## Ring Config`:

```markdown
## Ring Config
- role: challenger           # initiator, specialist, challenger, synthesizer
- position: 1                # order in the ring (0-indexed)
- vote_weight: 2.0           # this agent's vote counts double
```

## Automatic Pattern Selection

When using `--orchestrate auto` (or when the classifier is invoked programmatically), ArmadAI selects the pattern based on:

1. **Agent count:** Single matching agent = Direct (no orchestration)
2. **Keyword hints:** Task words like "review", "audit", "evaluate" suggest Ring; "generate", "build", "create" suggest Blackboard
3. **Domain overlap:** High tag overlap between agents suggests Ring (cross-critique); low overlap suggests Blackboard (parallel independent work)

Agent matching uses bidirectional prefix matching: tag `"review"` matches task word `"reviewing"`, tag `"infra"` matches `"infrastructure"`.

## Configuration

### Per-project (armadai.yaml)

Override orchestration defaults in your project config:

```yaml
defaults:
  orchestration:
    max_rounds: 10              # Blackboard: max rounds before halt
    max_laps: 5                 # Ring: max circulation laps
    consensus_threshold: 0.85   # Required ratio for consensus
    divergence_threshold: 0.60  # Challenge ratio that triggers divergence halt
    majority_threshold: 0.60    # Required ratio for majority outcome
    similarity_threshold: 0.85  # Jaccard threshold for grouping similar positions
    token_budget: 100000        # Max tokens across all agents
    agent_timeout_secs: 120     # Per-agent timeout
    convergence_rounds: 2       # Consecutive convergent rounds before halt
```

### Defaults

| Parameter | Blackboard | Ring |
|---|---|---|
| Max rounds/laps | 5 | 3 |
| Consensus threshold | 0.75 | 0.80 |
| Divergence threshold | 0.60 | — |
| Majority threshold | — | 0.60 |
| Similarity threshold | — | 0.85 |
| Token budget | 50,000 | 40,000 |
| Agent timeout | 60s | 90s |
| Convergence rounds | 1 | — |

## Storage

All orchestration runs are persisted to SQLite alongside regular agent runs:

- `orchestration_runs` — pattern, config, outcome, halt reason
- `board_entries` — Blackboard contributions per round
- `ring_contributions` — Ring contributions per lap
- `ring_votes` — final positions and confidence scores

Query them via the `runs` table (joined on `run_id`).

## Example: Code Review Ring

```markdown
# Security Reviewer

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- tags: [security, review]

## Ring Config
- role: specialist
- vote_weight: 1.5

## System Prompt
You are a security specialist. Focus on vulnerabilities, injection risks, and auth issues.
```

```markdown
# Performance Reviewer

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- tags: [performance, review]

## Ring Config
- role: challenger

## System Prompt
You are a performance specialist. Focus on N+1 queries, memory leaks, and bottlenecks.
```

```markdown
# Lead Reviewer

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- tags: [review, architecture]

## Ring Config
- role: synthesizer
- position: 2

## System Prompt
You are the lead reviewer. Synthesize security and performance findings into actionable recommendations.
```

```bash
armadai run security-reviewer --pipe performance-reviewer lead-reviewer --orchestrate ring "Review this PR diff: ..."
```
