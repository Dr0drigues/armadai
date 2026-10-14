# Providers and models (legacy)

> **Legacy format — read only by `armadai migrate`.** This page describes the `provider:` and
> `model:` fields of agents in the format of projects before v1 (`.armadai/config.yaml`,
> `armadai.yaml`). The only command that reads them is `armadai migrate`, which converts such a
> project to the [Project format](project-format.md) (`armadai init --pack` converts a legacy
> starter pack the same way). For how models work in a current project, see [Models](models.md).

## The `provider` field

`provider` was required for a legacy agent to load (in its `## Metadata`, or in `defaults` of
`.armadai/agents.yaml`). It named the vendor whose model catalogue resolved a `latest:*`
placeholder. `armadai migrate` drops it ("the target tool picks its own provider"): a canonical
agent has no provider.

## The `model` field and the `latest:*` placeholders

| Legacy `model:` | After `migrate` |
|---|---|
| `latest`, `latest:pro` (alias `latest:medium`) | `tier: pro` |
| `latest:fast` (alias `latest:low`) | `tier: fast` |
| `latest:max` (alias `latest:high`) | `tier: max` |
| `latest:auto` | dropped (it was resolved per call by the removed execution engine) |
| a concrete model id | `native.claude.model` (provider `anthropic`/`claude`) or `native.codex.model` (`openai`/`codex`); otherwise dropped |

The full conversion table is in [Project format](project-format.md#migrating-a-legacy-project).
