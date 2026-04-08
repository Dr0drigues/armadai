# Orchestration Guide

## Introduction

Orchestration is the art of making multiple AI agents work together on a single task. Instead of relying on one generalist agent, you can assemble a team of specialists — each with focused expertise — that collaborate to produce better results.

Why use orchestration? Complex tasks often benefit from specialization, multiple perspectives, or systematic decomposition. A code review is stronger when security, performance, and architecture experts each contribute their angle. A system design is more complete when frontend, backend, and DevOps specialists work in parallel. A large project is more manageable when a coordinator breaks it down and delegates to team leads.

ArmadAI supports four orchestration patterns: **Direct** (single agent, no orchestration), **Blackboard** (parallel independent work), **Ring** (sequential review with consensus voting), and **Hierarchical** (coordinator-led delegation with multi-level teams). Each pattern fits different collaboration styles and task structures.

## Patterns

### Direct

**What it does:** Runs a single agent without any orchestration. This is the default behavior when you run `armadai run agent-name`.

**How it works:**
1. Load the agent definition
2. Send the user's task to the agent
3. Return the agent's response

**Best for:**
- Simple, single-purpose tasks
- Exploratory work with one specialized agent
- Situations where collaboration overhead isn't justified

**Minimal config:**
```yaml
orchestration:
  enabled: true
  pattern: direct
```

**Key parameters:**

| Parameter | Default | Description |
|-----------|---------|-------------|
| pattern   | direct  | Orchestration pattern |

**Note:** You don't typically need to configure Direct mode explicitly — just run `armadai run agent-name` without orchestration flags.

### Blackboard

**What it does:** Multiple agents work in parallel rounds, reading from and writing to a shared board. Agents make observations, challenge each other's findings, and synthesize insights until they converge or reach a stopping condition.

**How it works:**
1. Initialize an empty shared board
2. Each round, all eligible agents receive a snapshot of the board
3. Agents contribute in parallel (findings, challenges, confirmations, synthesis)
4. Contributions are applied to the board as deltas
5. The engine checks for convergence (consensus, stability, or divergence)
6. Repeat until halted (consensus reached, max rounds, or budget exhausted)

**Best for:**
- Parallel analysis from multiple domains (frontend + backend + DevOps)
- Independent subtasks with low overlap (API design, database schema, CI pipeline)
- Brainstorming sessions where agents contribute diverse perspectives
- Tasks where agents can work simultaneously without tight sequencing

**Minimal config:**
```yaml
orchestration:
  enabled: true
  pattern: blackboard
  agents:
    - frontend-dev
    - backend-dev
    - devops
  max_rounds: 5
  token_budget: 50000
```

**Key parameters:**

| Parameter              | Default | Description |
|------------------------|---------|-------------|
| agents                 | []      | List of agents participating in the blackboard |
| max_rounds             | 5       | Maximum number of rounds before halting |
| consensus_threshold    | 0.75    | Ratio of confirmations needed for consensus |
| divergence_threshold   | 0.60    | Challenge ratio that triggers divergence halt |
| token_budget           | 50000   | Maximum tokens across all agents |
| agent_timeout_secs     | 60      | Timeout per agent per round |
| convergence_rounds     | 1       | Consecutive stable rounds before halting |

**Agent actions:** Agents respond with structured actions: **Finding** (new observation), **Challenge** (disagree with an entry), **Confirmation** (agree with an entry), **Synthesis** (combine multiple entries), **Question** (ask for clarification), **Answer** (respond to a question).

**Triggers:** Use the `## Triggers` section in agent definitions to control when agents participate (e.g., only after findings exist, skip after synthesis, minimum/maximum round).

### Ring

**What it does:** A token passes sequentially through agents in order. Each agent reads prior contributions, adds their own response, and the token continues. After multiple laps, agents vote on final positions and the system resolves consensus or majority.

**How it works:**
1. **Circulation phase:** A token circulates through agents in sequence
2. Each agent reads all prior contributions and adds their own (propose, enrich, contest, endorse, synthesize, or pass)
3. Multiple laps allow agents to react and refine their positions
4. **Voting phase:** Each agent states a final position with a confidence score
5. **Resolution phase:** Votes are grouped by similarity, weighted, and resolved to Consensus, Majority, or NoConsensus

**Best for:**
- Code review workflows (security, performance, architecture reviewers)
- Decision-making tasks requiring multiple expert perspectives
- Iterative refinement where later agents build on earlier contributions
- Tasks where you need explicit consensus or dissent tracking

**Minimal config:**
```yaml
orchestration:
  enabled: true
  pattern: ring
  agents:
    - security-reviewer
    - performance-reviewer
    - lead-reviewer
  max_laps: 3
  consensus_threshold: 0.80
```

**Key parameters:**

| Parameter             | Default | Description |
|-----------------------|---------|-------------|
| agents                | []      | List of agents in ring order |
| max_laps              | 3       | Maximum circulation laps |
| consensus_threshold   | 0.80    | Vote ratio required for consensus |
| majority_threshold    | 0.60    | Vote ratio required for majority (if not consensus) |
| similarity_threshold  | 0.85    | Jaccard threshold for grouping similar positions |
| token_budget          | 40000   | Maximum tokens across all laps |
| agent_timeout_secs    | 90      | Timeout per agent per lap |

**Agent actions:** Agents respond with structured actions: **Propose** (introduce idea), **Enrich** (build on prior contribution), **Contest** (argue against), **Endorse** (support), **Synthesize** (combine insights), **Pass** (nothing to add).

**Ring config:** Use the `## Ring Config` section in agent definitions to set roles (initiator, specialist, challenger, synthesizer), position in the ring, and vote weights.

**Outcomes:** Ring resolves to one of three outcomes:
- **Consensus:** One position exceeds `consensus_threshold` (default 0.80)
- **Majority:** One position exceeds `majority_threshold` (default 0.60) but not consensus; dissenting positions included
- **NoConsensus:** No position reaches majority; all positions reported

### Hierarchical

**What it does:** A coordinator receives the user's task, analyzes it, and delegates subtasks to leads or agents using `@agent-name: task` syntax. Leads can further delegate to their team members. Results flow back up for synthesis.

**How it works:**
1. User sends a task to the **coordinator** (top-level orchestrator)
2. Coordinator analyzes and delegates via `@agent-name: task description` directives
3. Leads receive delegations and can delegate further to their team agents
4. Agents in the same team can communicate laterally (`@peer: question`)
5. Results flow back up the hierarchy
6. Coordinator synthesizes a final answer from all results

**Best for:**
- Large, complex projects with natural team structure (frontend team, backend team, DevOps)
- Tasks requiring decomposition into hierarchical subtasks
- Scenarios where a project lead or architect should coordinate specialists
- Multi-level delegation (coordinator → leads → agents)

**Minimal config:**
```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: architect
  teams:
    - lead: backend-lead
      agents:
        - api-dev
        - db-dev
    - lead: frontend-lead
      agents:
        - ui-dev
    - agents:
        - devops
        - security
  max_depth: 4
  max_iterations: 30
  timeout: 180
```

**Key parameters:**

| Parameter      | Default | Description |
|----------------|---------|-------------|
| coordinator    | (required) | Name of the top-level coordinator agent |
| teams          | []      | List of teams (each with optional lead and list of agents) |
| max_depth      | 5       | Maximum delegation depth |
| max_iterations | 50      | Maximum total LLM invocations |
| timeout        | 300     | Global timeout in seconds |

**Agent roles:**
- **Coordinator:** Top-level orchestrator, delegates to leads or direct agents
- **Lead:** Team sub-coordinator, delegates to team agents
- **Agent:** Specialist worker, can communicate laterally with peers or escalate to lead
- **Direct agent:** Agent without a lead, reports directly to coordinator

**Delegation protocol:** The engine automatically injects an `## Orchestration Protocol` block into each agent's system prompt, describing their role, available team members, and the `@agent-name: message` delegation syntax.

**Safety limits:** Configure `max_depth`, `max_iterations`, and `timeout` to prevent runaway delegation.

## Decision Matrix

### Comparison Table

| Criteria              | Direct | Blackboard | Ring  | Hierarchical |
|-----------------------|--------|------------|-------|--------------|
| Number of agents      | 1      | 2-5        | 2-5   | 3-20+        |
| Task independence     | N/A    | High       | Low   | Mixed        |
| Need for consensus    | No     | No         | Yes   | No           |
| Need for coordination | No     | No         | No    | Yes          |
| Depth of decomposition| None   | Flat       | Flat  | Multi-level  |
| Cost (relative)       | $      | $$         | $$$   | $$-$$$$      |
| Latency               | Low    | Medium     | High  | Medium-High  |

### Decision Flowchart

```
Start → How many agents do you need?
  
  ├─ 1 agent → Direct
  │            (or just run `armadai run agent-name`)
  
  └─ 2+ agents → Are the subtasks independent?
      
      ├─ Yes, low overlap → Blackboard
      │  Example: frontend + backend + DevOps work in parallel
      
      └─ No → Do you need review/consensus or coordination?
          
          ├─ Need review/consensus → Ring
          │  Example: multiple experts review code and vote
          
          └─ Need coordination/delegation → Hierarchical
             Example: architect delegates to team leads
```

### Quick Decision Questions

1. **Is it a simple task for one specialist?** → **Direct**
2. **Can multiple experts work on separate parts simultaneously?** → **Blackboard**
3. **Do you need multiple reviewers to reach consensus?** → **Ring**
4. **Is there a team structure with leads coordinating specialists?** → **Hierarchical**

## Quick Start Recipes

### Recipe 1: Multiple experts review code

**Use case:** You want security, performance, and architecture experts to review a code change, discuss issues, and reach consensus on whether to approve.

**Pattern:** Ring

**Complete config:**
```yaml
# armadai.yaml
agents:
  - name: security-reviewer
  - name: performance-reviewer
  - name: architecture-reviewer

orchestration:
  enabled: true
  pattern: ring
  agents:
    - security-reviewer
    - performance-reviewer
    - architecture-reviewer
  max_laps: 3
  consensus_threshold: 0.75
```

**Run:**
```bash
armadai run security-reviewer "Review this PR diff for security, performance, and architecture issues: [paste diff]"
```

The three reviewers circulate through laps, building on each other's observations, then vote on final approval.

---

### Recipe 2: Parallel analysis of different domains

**Use case:** You're designing a new feature and want frontend, backend, and DevOps specialists to independently analyze their domain and contribute findings.

**Pattern:** Blackboard

**Complete config:**
```yaml
# armadai.yaml
agents:
  - name: frontend-dev
  - name: backend-dev
  - name: devops

orchestration:
  enabled: true
  pattern: blackboard
  agents:
    - frontend-dev
    - backend-dev
    - devops
  max_rounds: 5
  token_budget: 50000
```

**Run:**
```bash
armadai run frontend-dev "Design a user profile page with real-time updates"
```

All three agents work in parallel rounds, contributing findings, challenging assumptions, and synthesizing a complete design.

---

### Recipe 3: Team lead coordinates specialists

**Use case:** You have a project that needs an architect to coordinate a backend team (API dev, database dev) and a frontend team (UI dev).

**Pattern:** Hierarchical

**Complete config:**
```yaml
# armadai.yaml
agents:
  - name: architect
  - name: backend-lead
  - name: api-dev
  - name: db-dev
  - name: frontend-lead
  - name: ui-dev

orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: architect
  teams:
    - lead: backend-lead
      agents:
        - api-dev
        - db-dev
    - lead: frontend-lead
      agents:
        - ui-dev
  max_depth: 3
  max_iterations: 30
```

**Run:**
```bash
armadai run architect "Design a real-time chat application"
```

The architect analyzes the task, delegates backend design to `backend-lead`, frontend design to `frontend-lead`, and synthesizes their results into a complete architecture.

---

### Recipe 4: Just one agent

**Use case:** You have a single specialized agent and don't need orchestration.

**Pattern:** Direct (or just run without orchestration)

**Complete config:**
```yaml
# armadai.yaml (optional)
agents:
  - name: analyst

orchestration:
  enabled: true
  pattern: direct
```

**Run:**
```bash
# No orchestration needed — just run the agent directly
armadai run analyst "Analyze this dataset for trends"
```

Direct mode is the default. You don't need to configure orchestration at all for single-agent tasks.

## Cost Control

Orchestration multiplies API calls — a 3-agent Blackboard running 5 rounds makes up to 15 LLM invocations. To prevent runaway costs, ArmadAI provides two budget controls:

### Token Budget

Set a maximum number of tokens across all agents and rounds:

```yaml
orchestration:
  token_budget: 100000  # 100k tokens max
```

When the budget is exhausted, orchestration halts gracefully with partial results. The engine returns all contributions collected so far.

### Cost Limit

Set a maximum dollar amount (requires cost tracking enabled):

```yaml
orchestration:
  cost_limit: 5.00  # $5 max
```

The engine estimates API costs per invocation and halts when the limit is reached. Partial results are returned.

### What Happens When Limits Are Reached

When a budget or cost limit is hit:
1. The current round or lap completes (no partial contributions)
2. The engine halts with a `BudgetExhausted` or `CostLimitReached` status
3. All completed contributions are returned in the result
4. The halt reason is logged and visible in history (`armadai history`)

**Best practice:** Start with conservative budgets (e.g., 50k tokens for Blackboard, 40k for Ring) and increase if needed. Monitor costs via `armadai costs`.

## Tips and Gotchas

### 1. Start Simple, Add Orchestration When Needed
Don't over-engineer. If one agent can handle the task well, use Direct mode (or just `armadai run`). Add orchestration when you genuinely need multiple perspectives, parallel work, or hierarchical delegation.

### 2. Blackboard Works Best with 3-5 Agents
Too few agents (2) and you might as well use Ring for tighter collaboration. Too many agents (10+) and the board becomes noisy, making it hard to converge. Sweet spot: 3-5 specialists with distinct domains.

### 3. Ring Consensus Needs Odd Numbers of Agents
When voting for consensus, odd numbers (3, 5, 7) prevent ties. Even numbers (2, 4, 6) can deadlock or require lower majority thresholds. For critical decisions, prefer 3 or 5 reviewers.

### 4. Hierarchical Needs a Well-Defined Coordinator System Prompt
The coordinator drives the entire orchestration. Its system prompt must clearly explain its role: analyze the task, identify subtasks, delegate to appropriate leads/agents, and synthesize results. A vague coordinator leads to poor delegation.

### 5. Use `auto` Pattern to Let the Classifier Decide
If you're not sure which pattern to use, set `pattern: auto`. The engine analyzes the task, agent tags, and project config to select the best fit. Good for dynamic workflows where task types vary.

### 6. Budget Your Costs — Orchestration Multiplies API Calls
A 5-agent Blackboard running 4 rounds = 20 LLM calls. A Hierarchical delegation with 3 levels and 6 agents = 10+ calls. Always set `token_budget` or `cost_limit` to avoid surprise bills. Start conservative.

### 7. Use Agent Triggers to Optimize Blackboard
Not all agents need to participate in every round. Use the `## Triggers` section in agent definitions to activate agents conditionally:
```markdown
## Triggers
- requires: [finding]   # Only activate after findings exist
- max_round: 3          # Stop participating after round 3
- priority: 80          # Run earlier in the round
```

This reduces redundant LLM calls and improves convergence speed.

### 8. Test with Smaller Models First
While developing orchestration configs, test with smaller/faster models (e.g., `claude-haiku-4-20250514`) to iterate quickly. Switch to larger models (e.g., `claude-sonnet-4-5-20250929`) for production runs.

## Further Reading

For detailed technical documentation, including:
- Full parameter reference for all patterns
- Storage schema for orchestration runs
- Advanced agent configuration (triggers, ring roles, vote weights)
- Automatic pattern selection algorithm
- Internal implementation details

See the [Orchestration Reference](orchestration.md).
