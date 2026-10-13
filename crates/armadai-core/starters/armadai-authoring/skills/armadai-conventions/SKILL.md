---
name: armadai-conventions
description: Naming and quality conventions for ArmadAI content (agents, skills, flows, automations, starter packs) in the canonical format
---

# ArmadAI conventions

## Where things live

A project is a `.armadai/` folder at the root of a repository:

```
.armadai/
  project.yaml             # schema_version: 1, targets, coordinator, policy
  agents/<id>.md           # one agent: YAML frontmatter, then the prompt
  skills/<id>/SKILL.md     # one skill per folder (+ references/, scripts/, assets/)
  flows/<id>.yaml          # who hands work to whom
  automations/<id>.yaml    # a command run on an event (becomes a hook)
```

`armadai link` compiles it into the native files of each target (Claude Code, Codex, OpenCode,
Copilot). ArmadAI does not run agents.

## Ids

- Every id is the file or folder name: `[a-z0-9][a-z0-9-]{0,63}` (kebab-case, no dots, no
  uppercase). `armadai new` refuses anything else.
- A skill's `name` equals its folder name.
- Step ids of a flow follow the same rule; a step id may differ from its agent id.

## Tags

Lowercase, common vocabulary: a role (`coordinator`, `reviewer`, `writer`), a domain
(`security`, `testing`, `documentation`), a technology (`rust`, `typescript`), an activity
(`review`, `analysis`).

## Quality bar

- An agent's `description` says **when to use it**: the target reads it to pick a sub-agent. One
  sentence, specific ("Reviews Rust source for logic bugs and idioms"), never "helpful assistant".
- The prompt (the body) states the role, what the agent does AND does not do, its steps and its
  output format. No empty sections.
- A coordinator names its specialists by id and delegates with the target's own tool (the Agent
  tool in Claude Code); declare the topology in `flows/`, not in prose.
- Prefer a tier (`fast`, `pro`, `max`) to a concrete model.
- `armadai validate` passes with no error and no warning before anything is shared.
