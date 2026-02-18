# Starter Pack Examples

Real-world patterns from the built-in ArmadAI starter packs.

## Pattern 1: Coordinator + Specialists + Prompt

**Pack: `pirate-crew`** — A fun themed pack demonstrating the full coordinator pattern.

```yaml
name: pirate-crew
description: "Pirate development crew — a captain coordinator and their specialized crew"
agents:
  - capitaine        # Coordinator — dispatches to crew members
  - vigie            # Specialist — code review (lookout)
  - charpentier      # Specialist — code construction (carpenter)
  - cartographe      # Specialist — documentation (cartographer)
prompts:
  - code-nautique    # Shared conventions for the whole crew
```

**Structure:**
```
pirate-crew/
├── pack.yaml
├── agents/
│   ├── capitaine.md
│   ├── vigie.md
│   ├── charpentier.md
│   └── cartographe.md
└── prompts/
    └── code-nautique.md
```

**Key traits:**
- Single coordinator (`capitaine`) with a Pipeline section listing all specialists
- Shared prompt (`code-nautique`) applied to all agents
- Thematic naming for fun while maintaining clear roles
- 4 agents — manageable team size

## Pattern 2: Specialists + Prompt (No Coordinator)

**Pack: `rust-dev`** — A focused pack for Rust development without a coordinator.

```yaml
name: rust-dev
description: Rust development agent pack — code review, test writing, and debugging
agents:
  - code-reviewer
  - test-writer
  - debug
prompts:
  - rust-conventions
```

**Key traits:**
- No coordinator — users invoke specialists directly
- Best for small packs (2-3 agents) where routing is unnecessary
- Each agent is self-contained with clear purpose
- Shared conventions via prompt

## Pattern 3: Coordinator + Pipeline + Skill References

**Pack: `armadai-authoring`** — The meta-pack for creating ArmadAI content.

```yaml
name: armadai-authoring
description: "ArmadAI authoring pack — create agents, prompts, skills, and starters with built-in reference skills"
agents: [authoring-lead, agent-builder, prompt-builder, skill-builder, starter-builder]
prompts: [armadai-conventions]
skills: [armadai-agent-authoring, armadai-prompt-authoring, armadai-skill-authoring, armadai-starter-authoring]
```

**Key traits:**
- Coordinator (`authoring-lead`) with dispatch rules and pipeline
- Skills are **referenced, not bundled** — they are built-in to ArmadAI
- Each builder agent is paired with its corresponding reference skill
- Prompt applied to all builder agents for consistent conventions

## Pattern 4: Large Team with Scoped Specialists

**Pack: `code-analysis-rust`** — A full analysis crew with scoped file patterns.

```yaml
name: code-analysis-rust
description: "Code analysis crew for Rust projects — coordinator with scoped specialists"
agents:
  - lead-analyst
  - rust-reviewer
  - rust-test-analyzer
  - rust-doc-writer
  - rust-security
prompts:
  - analysis-standards
```

**Key traits:**
- 5 agents with a lead coordinator
- Specialists use `scope:` metadata to focus on specific file patterns
- Technology-prefixed names (`rust-reviewer` vs generic `reviewer`)
- Shared analysis standards prompt

## Minimal Pack Template

The simplest valid starter pack:

```yaml
name: my-pack
description: Description of my pack
agents: [my-agent]
```

```
my-pack/
├── pack.yaml
└── agents/
    └── my-agent.md
```

## Full Pack Template

A complete starter pack with all content types:

```yaml
name: my-domain
description: "My domain — coordinator with specialists, conventions, and skills"
agents: [domain-lead, specialist-a, specialist-b, specialist-c]
prompts: [domain-conventions]
skills: [domain-knowledge]
```

```
my-domain/
├── pack.yaml
├── agents/
│   ├── domain-lead.md        # Coordinator with Pipeline
│   ├── specialist-a.md
│   ├── specialist-b.md
│   └── specialist-c.md
├── prompts/
│   └── domain-conventions.md  # apply_to all specialists
└── skills/
    └── domain-knowledge/      # Bundled custom skill
        ├── SKILL.md
        └── references/
            ├── format.md
            └── examples.md
```
