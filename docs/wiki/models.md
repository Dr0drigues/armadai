# Models

ArmadAI does not run agents, so it has no providers and no API keys: the assistant you link to
picks and calls its own models. An agent carries at most a portable **tier** and, per target, an
optional pinned model id. Everything here works offline: `link` never touches the network.

## Tier and pinned model

In an agent's frontmatter (see [Project format](project-format.md)):

- `tier: fast | pro | max` is the only portable setting;
- `native.<target>.model` pins a concrete model id for one target, and wins over the tier.

## What `link` writes, per target

| Target | Model written |
|---|---|
| `claude` | `native.claude.model`, else the tier as an alias: `fast` → `haiku`, `pro` → `sonnet`, `max` → `opus`; no tier, no `model` key |
| `codex` | `native.codex.model`, else the tier (default `pro`) as an OpenAI model from the table below |
| `opencode` | `native.opencode.model`, else the tier as an Anthropic model from the table below; no tier, no model |
| `copilot` | none: Copilot agent files carry no model (reported `not-compiled`) |

See [Link](link.md) for the rest of each target's output.

## The static tier table

Codex and OpenCode turn a tier into a model with a table built into the binary. It is frozen at
the time it was written, and `link` never reads a model catalogue:

| Vendor | `fast` | `pro` | `max` |
|---|---|---|---|
| Anthropic (OpenCode) | `claude-haiku-4-5-20251001` | `claude-sonnet-4-5-20250929` | `claude-opus-4-6` |
| OpenAI (Codex) | `gpt-4o-mini` | `gpt-4o` | `o3-pro` |

Pin `native.<target>.model` when you need a newer model.

## Deprecated model ids

Canonical agents carry a tier; a model id only appears where an agent pins one, in
`native.<target>.model`. `armadai models check` reports the deprecated ones, and
`armadai models update` replaces them in place (only the id, in the frontmatter: comments and
the body are kept; a replacement that is a tier is reported, not written). `--all` applies to
every project registered by `armadai link`; `armadai models list` shows them. A legacy project
is reported "legacy format, run `armadai migrate`".
