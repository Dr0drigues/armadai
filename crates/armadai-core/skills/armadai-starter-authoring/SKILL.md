---
name: armadai-starter-authoring
description: Complete reference for authoring native ArmadAI starter packs (pack.yaml with format 1 and a canonical tree), installed by armadai init --pack
metadata:
  version: "2.0"
---

# ArmadAI Starter Pack Authoring

A starter pack is a ready-made project: a `pack.yaml` with `format: 1` next to a canonical tree
(agents, skills, flows, automations, an optional `project.yaml`). `armadai init --pack <name>`
validates it, then copies it into `.armadai/`.

See the `references/` directory:

- **format.md** — the folder, `pack.yaml`, what is copied, where packs are found
- **best-practices.md** — designing a cohesive team and its flows
- **examples.md** — the built-in packs as patterns
