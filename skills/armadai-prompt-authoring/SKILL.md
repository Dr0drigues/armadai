---
name: armadai-prompt-authoring
description: Complete reference for authoring ArmadAI composable prompt fragments
version: "1.0"
tools: []
---

# ArmadAI Prompt Authoring

Complete reference for creating and maintaining ArmadAI composable prompt fragments.

Prompts are single Markdown files that provide composable instruction fragments to agents. They support optional YAML frontmatter for metadata and targeting, and can be auto-injected into specific agents via the `apply_to` mechanism.

See the `references/` directory for detailed documentation:

- **format.md** — Prompt file specification and `apply_to` mechanism
- **best-practices.md** — Guidelines for writing effective, composable prompts
- **examples.md** — Real-world prompt patterns and complete examples
