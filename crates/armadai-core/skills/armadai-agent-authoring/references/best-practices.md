# Agent authoring best practices

## The description decides when the agent is used

The target reads `description` to choose a sub-agent. Write when to use it, specifically:

- good: `Reviews Rust source for logic bugs, idioms and error handling`
- good: `Owns releases: readiness, version, changelog, tags; use only when cutting a release`
- bad: `You are a helpful assistant` (the first sentence of a prompt is rarely a description)

## Prompt structure

Role + scope + method + output:

```markdown
You are a [ROLE] specialized in [DOMAIN].

Your scope:
- [RESPONSIBILITY 1]
- [RESPONSIBILITY 2]
You do not [OUT OF SCOPE].

## Instructions
1. [STEP 1]
2. [STEP 2]

## Output Format
[FORMAT, with an example when it is not obvious]
```

- Be specific about the role: "senior security auditor", not "assistant".
- State the boundaries: what the agent does not do.
- Use the imperative: "Analyze the code".
- No empty section.

## Tier and temperature

| Tier | Use |
|---|---|
| `fast` | Lookups, transformations, cost-sensitive work |
| `pro` | The default: analysis, generation, review |
| `max` | Complex reasoning, architecture, critical reviews |

Temperature: 0.1-0.3 for review and audit (consistency), 0.4-0.6 for planning and
documentation, higher for exploration.

## Capabilities

Restrict what an agent may do when its role calls for it: a reviewer with `write: deny`, an
auditor with `shell: ask`. Leave a capability out to keep the target's default; `armadai link`
reports how each target renders it (or cannot).

## Coordinator and specialists

- A **specialist** is narrow and deep: one domain, one output.
- A **coordinator** names its specialists by id, says how it splits the work and does not do it
  itself. It delegates with the target's own tool (the Agent tool in Claude Code); never write
  the removed `@agent: task` text.
- Declare who hands work to whom in a flow (`armadai new flow <id> --template fan-out
  --agents lead,a,b`), not only in prose: `armadai graph` draws it, Claude Code's gate enforces it
  under `policy.delegation: strict`.

## Skills instead of long prompts

Knowledge several agents share (conventions, a reference) is a skill
(`.armadai/skills/<id>/SKILL.md`) listed in each agent's `skills:`, not text copied in every
prompt.

## Anti-patterns

1. A generic prompt ("be helpful") or a description copied from the prompt's first sentence.
2. Too many responsibilities: split into specialists.
3. No output format: the agent guesses.
4. A high temperature for a review.
5. A coordinator doing the work itself.
