# Prompt Authoring Best Practices

## One Prompt = One Concern

Each prompt file should address a single, well-defined concern:

- **Good**: `rust-conventions.md` — Rust coding style rules
- **Good**: `error-handling.md` — Error handling standards
- **Bad**: `all-standards.md` — Everything mixed together

This enables mix-and-match composition: different agents get different prompt combinations.

## Conventions vs Context

### Convention Prompts

Convention prompts define **how** to write code — coding standards, naming rules, patterns:

```markdown
---
name: java-conventions
description: Java coding standards
apply_to:
  - "*"
---

- Use `var` for local variables when the type is obvious
- Prefer records over POJOs for data transfer objects
- All public methods must have Javadoc
- Use `Optional` instead of nullable return types
```

### Context Prompts

Context prompts provide **what** — background information about the project or domain:

```markdown
---
name: project-context
description: Project architecture context
apply_to:
  - "*"
---

This is a microservice handling order processing.
- Database: PostgreSQL via JPA
- Message broker: Kafka for async events
- Authentication: OAuth2 via company IAM
```

Keep these separate — conventions are reusable across projects, context is project-specific.

## `apply_to` Patterns

### Targeted Application

Apply conventions only to relevant agents:

```yaml
apply_to:
  - code-reviewer
  - test-writer
```

### Global Application

Use `*` for project-wide conventions:

```yaml
apply_to:
  - "*"
```

### No Auto-Application

Omit `apply_to` for prompts that should only be used when explicitly referenced:

```yaml
# No apply_to — must be explicitly listed in armadai.yaml
name: special-instructions
description: Context for specific analysis tasks
```

## Writing Effective Prompts

1. **Be directive** — Use imperative voice: "Use snake_case" not "You should consider using snake_case"
2. **Be specific** — Include concrete examples, not vague guidelines
3. **Be concise** — One prompt should fit in a few paragraphs. If it's getting long, split into multiple prompts
4. **Use lists** — Bullet points are easier for agents to process than prose paragraphs
5. **Include examples** — Show correct and incorrect patterns when rules are nuanced

## Composition Strategy

Design prompts to compose cleanly:

```yaml
# armadai.yaml
prompts:
  - name: java-conventions      # Language standards
  - name: spring-patterns       # Framework patterns
  - name: api-design            # REST API conventions
  - name: project-context       # Project-specific context
```

Each prompt adds one layer. The agent receives all applicable prompts combined.

## Common Anti-Patterns

1. **Monolithic prompt** — One giant file with all rules. Split by concern instead
2. **Contradictory prompts** — Two prompts giving conflicting instructions to the same agent
3. **Over-targeting** — Using `apply_to: ["*"]` for prompts that only apply to specific agents
4. **Redundant with system prompt** — Duplicating what's already in the agent's system prompt
