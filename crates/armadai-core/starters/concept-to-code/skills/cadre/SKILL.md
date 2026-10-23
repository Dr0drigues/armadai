---
name: cadre
description: "The project frame (.armadai/docs/cadre.md): what is decided (stack, conventions, tools, security, infrastructure), what is still to decide, and the user's dated decisions; with the rule to ask the user before adopting any convention the frame does not fix. Use before writing, reviewing or explaining code, and whenever a convention, a library or a layout must be chosen."
---
# Cadre: the frame of the project

The frame is the set of rules the code follows. It is the user's: the repository fixes part of it, the user decides the rest, and no agent adds to it on its own.

## Where

`.armadai/docs/cadre.md`, a working document of the team. It is not human documentation: the project standards for humans and tools live in the project's standards skill (written by `/init-standards`) and in `docs/technical/`.

## Shape

```markdown
# Cadre

Updated: YYYY-MM-DD

## Decided
- <topic>: <the rule, imperative and checkable>. Source: <file path | standards skill <id> | user, YYYY-MM-DD>

## To decide
- <topic>: <options seen>. Needed by: <step or agent>

## User decisions
- YYYY-MM-DD, <topic>: <decision> (<the user's reason, if given>). Asked by: <agent>
```

Topics, when they apply: language and version, framework, project layout, naming, formatting and linting, tests (tool, location, level), error handling, logging, dependencies (which are allowed, how to add one), secrets, input validation, authentication and authorisation, dependency audit, commits and branches, language of the documentation, build and deployment.

## What counts as decided

- A rule a tool enforces: a formatter or linter configuration, a CI job, a hook. Source: the file.
- A rule of a skill that fills the `code-standards`, `security` or `infra-docs` slot. Source: the skill.
- A decision the user took, recorded under "User decisions".

A habit seen in the code but enforced by nothing is **observed**, not decided: put it under "To decide" with "observed in <files>" and have the user confirm it.

## The rule: ask before adopting a convention

Before writing code that depends on a convention absent from "Decided" (a layout, a naming pattern, a library, an error style, a test tool, a configuration format), stop and have the user decide:

1. State the question, 2 to 4 options with what each implies, your recommendation and why, and what waits on it.
2. In the main conversation, ask it with your question tool. As a subagent, you cannot ask: return it to `lead` under "Questions for the user"; `lead` asks and sends the answer back.
3. Record the answer: a line under "User decisions" with the date and who asked, the rule under "Decided" with the user as source, and the topic removed from "To decide". An agent that cannot write (`coach`) lists it under "To record in the frame" for `lead`.
4. Only then write the code.

An obvious option is still the user's to choose: a short question costs less than a convention nobody agreed to.

Never invent a source, never record as decided what the user did not say.
