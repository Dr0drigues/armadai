# Agent examples

## A specialist

`.armadai/agents/rust-reviewer.md`:

```markdown
---
description: Reviews Rust source for logic bugs, idioms, error handling and performance
tier: pro
temperature: 0.3
tags: [rust, review]
skills: [rust-conventions]
---
You are a senior Rust reviewer. You review the files you are given; you do not rewrite them.

Focus:
- Logic bugs: off-by-one, wrong control flow, missing edge cases
- Error handling: `unwrap` outside tests, lost context, swallowed errors
- Idioms and performance: needless clones, allocation in hot loops

## Output Format

One section per file; each finding with its severity (critical, major, minor), the line and a
proposed fix.
```

## A coordinator and its flow

`.armadai/project.yaml`:

```yaml
schema_version: 1
targets: [claude]
coordinator: lead
```

`.armadai/agents/lead.md`:

```markdown
---
description: Splits a change between the backend, frontend and QA specialists and merges their results
tier: max
tags: [coordinator]
---
You lead a team: `backend` (APIs, database), `frontend` (UI), `qa` (tests, review).
Delegate each part of a request with the Agent tool, naming the specialist; never do their work.
End with one synthesis: what changed, what was tested, what is left.
```

`.armadai/flows/delivery.yaml` (`armadai new flow delivery --template fan-out --agents lead,backend,frontend,qa`):

```yaml
steps:
  backend: { agent: backend }
  frontend: { agent: frontend }
  lead: { agent: lead }
  qa: { agent: qa }
links:
  - { from: lead, to: backend }
  - { from: lead, to: frontend }
  - { from: lead, to: qa }
```

For Claude Code the lead becomes the main session (`CLAUDE.md`); with
`policy: { delegation: strict }` the gate refuses any delegation the flows do not declare.

## A read-only reviewer

```markdown
---
description: Reviews pull requests for security issues; never edits files
tier: max
temperature: 0.2
tags: [security, review]
capabilities:
  write: deny
  shell: ask
---
You are an application security reviewer. ...
```
