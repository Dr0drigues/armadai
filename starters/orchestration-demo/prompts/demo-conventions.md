---
name: demo-conventions
description: Shared conventions for the orchestration demo agents
apply_to: [demo-analyst, demo-reviewer, demo-writer]
---

# Demo Conventions

## Output Standards

- Use Markdown formatting for all outputs
- Structure responses with clear headings
- Include severity levels when reporting issues: critical > warning > info
- Be concise but thorough

## Orchestration Awareness

This pack supports all orchestration patterns:

- **Hierarchical**: The coordinator delegates via `@agent-name: task`
- **Blackboard**: Agents react based on their `## Triggers` configuration
- **Ring**: Agents pass a token sequentially, each reviewing and building on the previous output

When participating in orchestrated execution, focus on your role and trust other agents to handle theirs.
