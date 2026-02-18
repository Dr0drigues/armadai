# Starter Pack Best Practices

## Thematic Cohesion

Each pack should serve a single domain or workflow. Users should understand what a pack does from its name and description alone.

**Good**: `rust-dev` (Rust development), `code-analysis-web` (JS/TS code analysis)
**Bad**: `misc-tools` (unclear scope), `agents-v2` (meaningless name)

## Pack Composition

### Include a Coordinator

For packs with 3+ agents, include a **coordinator agent** that dispatches requests to specialists. This gives users a single entry point.

```
lead-analyst → rust-reviewer
             → rust-test-analyzer
             → rust-doc-writer
             → rust-security
```

The coordinator should:
- List all team members and their specialties in its system prompt
- Define clear dispatch rules
- Include a `## Pipeline` section listing downstream agents
- Use a slightly higher temperature (0.4) than specialists (0.2-0.3) for flexible routing

### Include a Shared Prompt

Add a prompt with conventions shared across all agents in the pack. Apply it via `apply_to` to all specialist agents.

This ensures consistent behavior (naming, formatting, quality standards) without duplicating instructions in every agent's system prompt.

### Reference Built-in Skills

When your pack's agents need knowledge provided by existing ArmadAI skills (e.g., `armadai-agent-authoring`), **reference them** in `pack.yaml` rather than bundling copies. Built-in skills are already installed by `armadai init` and referencing avoids duplication.

```yaml
# Good: reference built-in skill
skills: [armadai-agent-authoring, armadai-prompt-authoring]

# Only bundle skills that are custom to your pack
```

## Recommended Pack Size

- **Minimum**: 2 agents + 1 prompt (a small focused team)
- **Sweet spot**: 3-6 agents + 1-2 prompts (clear roles, manageable)
- **Maximum**: 8-10 agents (beyond this, consider splitting into separate packs)

Including too many agents dilutes the pack's focus and makes coordination harder.

## Naming Guidelines

- Pack name should describe the domain: `rust-dev`, `devops-fleet`, `data-pipeline`
- Agent names should describe the role: `code-reviewer`, `test-writer`, `lead-analyst`
- Avoid generic names: prefer `rust-security` over `security-agent`
- Coordinators: use `lead`, `captain`, or domain-specific titles

## Agent Design Within Packs

- Each specialist should have a **focused scope** — one clear responsibility
- Use `scope:` metadata to limit file patterns when relevant (e.g., `[src/**/*.rs]`)
- Use `tags:` consistently across agents for prompt targeting
- Set appropriate temperatures: analytical tasks (0.2-0.3), creative tasks (0.5-0.7)

## Testing Your Pack

Before publishing a starter pack:

1. Run `armadai init --pack <name>` in a clean environment
2. Verify all agents are installed: `armadai list`
3. Test each agent individually: `armadai run <agent-name>`
4. Test the coordinator's dispatch logic with various request types
5. Verify prompts are correctly applied to their target agents
