# Starter pack examples

The packs built into ArmadAI (`armadai init --pack <name>`):

| Pack | Shape |
|---|---|
| `rust-dev` | 3 independent agents (review, tests, debug) and a shared `rust-conventions` skill; no coordinator |
| `fullstack` | 3 agents chained by a `review` flow (`review-loop`: tests, review, docs) |
| `code-analysis-rust`, `code-analysis-web` | a lead analyst (`coordinator`) and 4 scoped analysts, an `analysis` fan-out flow, an `analysis-standards` skill |
| `armadai-dev` | a dev lead (`coordinator`), 5 specialists, a `delivery` fan-out and a `release` flow (QA approves, then the release manager) |
| `armadai-authoring` | an authoring lead, builders, a reviewer and a tester; a `pack` flow (plan, build, review loop, test) |
| `pirate-crew` | a themed crew: a captain and 3 crew members |

## A minimal native pack

```
review-pair/
  pack.yaml
  agents/author.md
  agents/reviewer.md
  flows/review.yaml
```

```yaml
# pack.yaml
format: 1
name: review-pair
description: "An author and a reviewer: changes go back until approved"
```

```yaml
# flows/review.yaml  (armadai new flow review --template review-loop --agents author,reviewer)
steps:
  author: { agent: author }
  reviewer: { agent: reviewer, states: [approved, rejected] }
links:
  - { from: author, to: reviewer }
  - { from: reviewer, to: author, when: rejected }
```

No `project.yaml`: the project gets `targets: [claude]`.
