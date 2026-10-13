# Skill format

## Folder

```
.armadai/skills/<id>/
  SKILL.md          # required
  references/       # optional: detailed documents, one topic each
  scripts/          # optional: executables (their executable bit is kept by link)
  assets/           # optional: templates, schemas, data
```

The folder name is the id: `[a-z0-9][a-z0-9-]{0,63}`.

## SKILL.md

```markdown
---
name: release-notes
description: Writes release notes from the git log since the last tag; use when preparing a release
---

# Release notes

How to collect the changes, group them and write the notes. See `references/style.md`.
```

| Key | Required | Value |
|---|---|---|
| `name` | no | Equal to the folder name when present (`skill-name-mismatch` otherwise) |
| `description` | yes | What it provides and when to use it, at most 1024 characters |
| `license` | no | A license name or file |
| `compatibility` | no | Free text |
| `metadata` | no | A map of strings (e.g. `version: "1.0"`) |

Any other key (`allowed-tools`, ...) is kept as written and reported as a warning
(`unknown-skill-key`): the format belongs to the targets, ArmadAI does not interpret it.

## Using a skill

An agent lists it: `skills: [release-notes]` in its frontmatter. A skill the project lacks is an
error (`unknown-skill`).

## Commands

```bash
armadai new skill release-notes --description "Writes release notes from the git log"
armadai validate
armadai link
```
