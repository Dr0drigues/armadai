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
- model: claude-sonnet-4-5-20250929
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
- model: claude-sonnet-4-5-20250929
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
- model: claude-opus-4-6
- model_fallback: [claude-sonnet-4-5-20250929, claude-haiku-4-5-20251001]
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

## Common Anti-Patterns

1. **Overly generic system prompt** — "You are a helpful assistant" gives no focus
2. **Too many responsibilities** — split into multiple specialists instead
3. **Missing output format** — the agent guesses, producing inconsistent results
4. **Temperature mismatch** — using 0.7 for code review (should be 0.2-0.3)
5. **No tags** — makes the agent hard to discover and categorize
