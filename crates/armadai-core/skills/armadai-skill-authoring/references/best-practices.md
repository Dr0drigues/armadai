# Skill best practices

## Skill or agent?

| Need | Write |
|---|---|
| A role that acts (reviews, writes, plans) | an agent |
| Knowledge or a procedure several agents use (conventions, an API, a checklist) | a skill |
| A command to run on an event (format, lint) | an automation |

## Scope

One domain per skill: `rust-conventions`, `release-notes`, `payments-api`. A skill that covers
everything is loaded for everything, and helps nothing.

## The description

The target decides from `description` when to load the skill. Say what it provides and when:
"Shared output format and severity scale of the analysis reports; use when writing a report".

## Organization

- `SKILL.md` is a table of contents: what the skill covers and the list of its references.
- One topic per reference file, with concrete examples.
- Scripts are small, documented, and safe to run twice.

## Naming

kebab-case everywhere: the folder, `name`, the reference files (`api-errors.md`).
