# Agent file format

An agent lives at `.armadai/agents/<id>.md` of a project (a folder holding
`.armadai/project.yaml`). The file name is the id: `[a-z0-9][a-z0-9-]{0,63}`.

```markdown
---
description: Reviews Rust source for logic bugs, idioms and error handling
title: Rust Reviewer
tier: pro
temperature: 0.3
tags: [rust, review]
skills: [rust-conventions]
capabilities:
  read: allow
  write: deny
native:
  claude:
    color: blue
---
You are a senior Rust reviewer. ...

## Instructions
...

## Output Format
...
```

## Frontmatter

Any key not listed here is an error (`armadai validate` names it).

| Key | Required | Value |
|---|---|---|
| `description` | yes | When to use the agent: the target picks a sub-agent by it. Not empty. |
| `title` | no | Display name (Claude Code names a sub-agent by its id: reported, not compiled) |
| `tier` | no | `fast`, `pro` or `max`; `armadai link` resolves it to a model per target |
| `temperature` | no | A finite number, for targets that have the setting (Claude Code has none: reported, not compiled) |
| `tags` | no | List of lowercase words (informational) |
| `skills` | no | Ids of skills of the project (`.armadai/skills/<id>/`) |
| `capabilities` | no | `read`, `write`, `shell`, `web`, `delegate`: each `allow`, `ask` or `deny`; leave one out to keep the target's default |
| `native` | no | Per-target keys passed as is: `claude`, `codex`, `copilot`, `opencode` |

## Body

Everything after the closing `---` is the prompt, Markdown, sent as is. Sections such as
`## Instructions` or `## Output Format` are a convention for the reader and the model; none is
required and none has a special meaning.

## The coordinator

`project.yaml` may name one agent `coordinator`. For Claude Code it becomes the main session (a
block in the root `CLAUDE.md`, with the list of the other agents and the flows), not a sub-agent.

## Checks

`armadai validate` reports, with file, line and column: an unknown key, an empty
`description`, a non-finite `temperature`, a skill the project lacks (`unknown-skill`), an
unknown target under `native`, an invalid id (`bad-id`).

## Commands

```bash
armadai new agent <id> --template <t> [--description D] [--stack S] [--tier T] [--skills a,b]
armadai new agent -i         # guided
armadai validate
armadai inspect <id>
armadai link
```

A legacy agent file (`# Name`, `## Metadata`, `## System Prompt`) is converted once by
`armadai migrate`; nothing else reads that format.
