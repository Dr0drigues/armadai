---
name: armadai-skill-authoring
description: Complete reference for authoring skills of an ArmadAI project (.armadai/skills/<id>/SKILL.md, Agent Skills open standard)
metadata:
  version: "2.0"
---

# ArmadAI Skill Authoring

A skill is shared knowledge an agent loads when a task calls for it: a folder
`.armadai/skills/<id>/` with a `SKILL.md` entry point and optional `references/`, `scripts/`
and `assets/`. `armadai link` copies the whole folder to each target. A former ArmadAI
"prompt" (an instruction fragment injected with `apply_to`) is a skill now.

See the `references/` directory:

- **format.md** — the folder, the frontmatter, the checks of `armadai validate`
- **best-practices.md** — scoping, writing a description, organizing references
- **examples.md** — complete skills
