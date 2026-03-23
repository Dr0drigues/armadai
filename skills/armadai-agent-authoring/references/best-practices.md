# Agent Authoring Best Practices

## System Prompt Structure

A well-structured system prompt follows the **Role + Scope + Methodology + Output** pattern:

```markdown
## System Prompt

You are a [ROLE] specialized in [DOMAIN].

Your scope:
- [RESPONSIBILITY 1]
- [RESPONSIBILITY 2]

Methodology:
1. [STEP 1]
2. [STEP 2]
3. [STEP 3]

Output your findings as [FORMAT].
```

### Key Principles

- **Be specific about the role** — "senior security auditor" is better than "helpful assistant"
- **Define boundaries** — state what the agent should NOT do
- **Use imperative voice** — "Analyze the code" not "You should analyze the code"
- **Include examples** when the expected output format is non-obvious

## Temperature Guidelines

| Temperature | Use Case | Examples |
|---|---|---|
| `0.1 – 0.3` | Deterministic analysis, review, auditing | Code review, security audit, test validation |
| `0.4 – 0.6` | Balanced tasks, structured generation | Planning, documentation, refactoring |
| `0.7` (default) | General-purpose, creative tasks | Code generation, brainstorming |
| `0.8 – 1.2` | Highly creative, exploratory | Naming, architecture exploration |

Rule of thumb: lower temperature for tasks where **consistency** matters, higher for tasks where **diversity** matters.

## Coordinator vs Specialist Pattern

### Coordinator Agent

A coordinator orchestrates a fleet of specialist agents. Tag it with `coordinator`:

```markdown
## Metadata
- provider: anthropic
- model: latest:pro
- tags: [coordinator]
```

The coordinator's system prompt should:
- Describe the overall mission
- Reference which specialists are available
- Define the decision-making process
- NOT do the actual work itself

### Specialist Agent

A specialist focuses on one domain. Keep it narrow and deep:

```markdown
## Metadata
- provider: anthropic
- model: latest:pro
- tags: [review, security]
- stacks: [java, spring]
- scope: [src/**/*.java]
```

## Cost Control

### `cost_limit`

Set a per-run budget to prevent runaway costs:

```markdown
- cost_limit: 0.50
```

### `model_fallback`

Define cheaper fallback models tried in order if the primary model fails or exceeds limits:

```markdown
- model: latest:max
- model_fallback: [latest:pro, latest:fast]
```

The system tries each model in sequence until one succeeds.

## Pipeline Chaining

Use `## Pipeline` to chain agents sequentially. The output of one agent becomes the input of the next:

```markdown
## Pipeline
- test-writer
- code-reviewer
```

Pipeline tips:
- Keep each agent focused on one transformation
- The first agent in the chain receives the user's original input
- Subsequent agents receive the previous agent's output
- Use consistent output formats between chained agents

## Scope Patterns

The `scope` field restricts which files an agent operates on (used by linkers):

```markdown
- scope: [src/**/*.rs, tests/**/*.rs]
- scope: [*.ts, *.tsx]
- scope: [docs/]
```

Use glob patterns. This is informational — the agent won't automatically filter files, but linkers and tools use this metadata.

## Orchestration Sections

ArmadAI supports 4 orchestration patterns. Agents can participate in orchestration by adding optional sections:

### Hierarchical (Coordinator → Specialists)

The coordinator uses `@agent-name: task` delegation protocol in its system prompt:

```markdown
## System Prompt

Your team:
| Agent | Role |
|-------|------|
| security-auditor | Security vulnerability scanning |
| test-writer | Test coverage analysis |

To delegate, use: `@agent-name: description of the task`
```

### Blackboard (Parallel Reactive)

Add a `## Triggers` section to reactive agents:

```markdown
## Triggers
- requires: [initial_analysis]
- excludes: []
- min_round: 1
- max_round: 3
- priority: 5
```

### Ring (Token-Passing Consensus)

Add a `## Ring Config` section:

```markdown
## Ring Config
- role: reviewer
- position: 2
- vote_weight: 1.0
```

Roles: `proposer` (generates initial proposal), `reviewer` (evaluates and refines), `validator` (final approval).

## Common Anti-Patterns

1. **Overly generic system prompt** — "You are a helpful assistant" gives no focus
2. **Too many responsibilities** — split into multiple specialists instead
3. **Missing output format** — the agent guesses, producing inconsistent results
4. **Temperature mismatch** — using 0.7 for code review (should be 0.2-0.3)
5. **No tags** — makes the agent hard to discover and categorize
