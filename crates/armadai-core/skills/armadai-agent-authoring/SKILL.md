---
name: armadai-agent-authoring
description: Complete reference for authoring ArmadAI agents in the canonical format (.armadai/agents/<id>.md, YAML frontmatter and prompt)
metadata:
  version: "2.0"
---

# ArmadAI Agent Authoring

An ArmadAI agent is one Markdown file, `.armadai/agents/<id>.md`: a strict YAML frontmatter
(its description, tier, tags, skills, capabilities) and the prompt as the body. `armadai link`
compiles it into each target's native sub-agent (Claude Code, Codex, OpenCode, Copilot).

See the `references/` directory:

- **format.md** — the file format, every frontmatter key, the checks of `armadai validate`
- **best-practices.md** — descriptions, prompts, tiers, coordinators and specialists
- **templates.md** — the built-in templates of `armadai new agent`
- **examples.md** — complete agents (a specialist, a coordinator with its flow, a read-only reviewer)
