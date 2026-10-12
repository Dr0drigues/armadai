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

A coordinator orchestrates a team of specialist agents. Tag it with `coordinator`:

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

## Model Fallbacks

### `model_fallback`

Declare fallback models alongside the primary one:

```markdown
- model: latest:max
- model_fallback: [latest:pro, latest:fast]
```

`armadai models` checks and rewrites `model_fallback` entries like `model`. No kept command tries
them in sequence: that was the removed execution engine.

## Scope Patterns

The `scope` field declares which files an agent focuses on:

```markdown
- scope: [src/**/*.rs, tests/**/*.rs]
- scope: [*.ts, *.tsx]
- scope: [docs/]
```

Use glob patterns. This is informational: it is parsed, but no link target writes it today, and the agent won't automatically filter files.

## Delegation in a Coordinator

The coordinator names its team and how it delegates in its system prompt:

```markdown
## System Prompt

Your team:
| Agent | Role |
|-------|------|
| security-auditor | Security vulnerability scanning |
| test-writer | Test coverage analysis |

To delegate, use: `@agent-name: description of the task`
```

ArmadAI does not execute agents: the delegation happens in the assistant the agents are linked to.
Who may delegate to whom can be declared in the `orchestration:` block of the project config and
enforced in Claude Code by the policy gate (see the `armadai-orchestration-patterns` skill).

## Engine-only Sections

`## Pipeline`, `## Triggers` and `## Ring Config`, and the metadata keys `command`, `args`,
`max_tokens`, `timeout`, `cost_limit`, `rate_limit`, `context_window` and `mode`, configured the
removed execution engine (pipelines, Blackboard, Ring, per-run budgets). They are still parsed for
compatibility, but nothing uses them now: do not add them to new agents.

## Common Anti-Patterns

1. **Overly generic system prompt** — "You are a helpful assistant" gives no focus
2. **Too many responsibilities** — split into multiple specialists instead
3. **Missing output format** — the agent guesses, producing inconsistent results
4. **Temperature mismatch** — using 0.7 for code review (should be 0.2-0.3)
5. **No tags** — makes the agent hard to discover and categorize
