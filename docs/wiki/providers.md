# Providers and models

ArmadAI no longer executes agents, so it has no execution providers: no API client, no CLI
relay, no proxy. What remains of "providers" is the `provider:` and `model:` fields of an agent,
and how `armadai link` turns them into the model id written into each target's files. Everything
here works offline: `link` never touches the network.

## The `provider` field

`provider` is required for an agent to load (in its `## Metadata`, or in `defaults` of
`.armadai/agents.yaml`). For `link`, it does one thing: it names the vendor whose model catalogue
resolves a `latest:*` placeholder, on the targets that resolve per agent (see below).

| `provider:` value | Model catalogue (vendor) |
|---|---|
| `claude`, `anthropic` | Anthropic |
| `gemini`, `google` | Google |
| `gpt`, `aider`, `codex`, `openai` | OpenAI |
| anything else (`cli`, `copilot`, `opencode`, `proxy`, …) | none — the built-in Anthropic defaults are used |

`gemini` here is a model vendor name, not a link target: Gemini CLI is no longer a target of
`armadai link`.

The interactive wizard of `armadai new -i` uses a second, older tool-to-vendor table
(`armadai-providers`' `factory.rs`) to pick which catalogue to list. It differs on one name:
`codex` maps to no vendor there.

## The `model` field and the `latest:*` placeholders

| Placeholder | Tier |
|---|---|
| `latest`, `latest:pro` (alias `latest:medium`) | Balanced (default) |
| `latest:fast` (alias `latest:low`) | Fast / cheap |
| `latest:max` (alias `latest:high`) | Most capable |

`latest:auto` was resolved per call by the removed execution engine; `link` treats it like any
other non-placeholder string (see below).

## What `link` writes, per target

| Target | Model written | How it is chosen |
|---|---|---|
| `claude` | `model:` in `.claude/agents/*.md` | Always an **Anthropic** model: a `latest:*` tier is resolved for Anthropic; any other value (a concrete id, `latest:auto`, nothing) becomes Anthropic's Pro tier |
| `codex` | `model` in `.codex/agents/*.toml` | Always an **OpenAI** model, by the same rule |
| `opencode` | `model:` in `.opencode/agents/*.md` | `--model <id>`: that id for every agent. Otherwise, in an interactive terminal, `link` asks for a vendor and a model (listed from the local catalogue cache, or typed in). Otherwise, each agent's `latest:*` tier is resolved with the vendor of its own `provider:`; any other value is written unchanged |
| `copilot` | none | Same choice as `opencode`, but the generated `.agent.md` files carry no model |

Measured on a project with four agents and no catalogue cache:

| Agent (`provider` / `model`) | claude | codex | opencode (non-interactive) |
|---|---|---|---|
| `cli` / `latest:fast` | `claude-haiku-4-5-20251001` | `gpt-4o-mini` | `claude-haiku-4-5-20251001` |
| `gemini` / `latest:max` | `claude-opus-4-6` | `o3-pro` | `gemini-2.5-pro` |
| `openai` / `gpt-4.1` | `claude-sonnet-4-5-20250929` | `gpt-4o` | `gpt-4.1` |
| `claude` / `latest:auto` | `claude-sonnet-4-5-20250929` | `gpt-4o` | `latest:auto` |

## How a tier becomes a model id

The resolution reads the local models.dev catalogue cache (`~/.config/armadai/models-cache.json`)
when it exists, and the built-in fallback table otherwise.

**Nothing refreshes the cache any more.** Earlier versions downloaded it; the current binary only
reads a cache left by them. Without one, the built-in table is used, and its content is frozen at
the time it was written:

| Vendor | `latest:fast` | `latest:pro` | `latest:max` |
|---|---|---|---|
| Anthropic | `claude-haiku-4-5-20251001` | `claude-sonnet-4-5-20250929` | `claude-opus-4-6` |
| OpenAI | `gpt-4o-mini` | `gpt-4o` | `o3-pro` |
| Google | `gemini-2.5-flash` | `gemini-2.5-pro` | `gemini-2.5-pro` (Google has no tier above Pro) |

Use an explicit model id (on the targets that keep it) when you need a newer model. Updating this
table is tracked in the
[step 0 spec](https://github.com/Dr0drigues/armadai/blob/master/docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md)
(section 8).

When a cache is present, the best match for the tier is chosen by four keys, then a tie-break:

1. **Generation** — the numbers in the id, compared as numbers (`gpt-10` is above `gpt-9`).
2. **Priced** — an entry the catalogue does not price never wins over a priced one.
3. **Base of the generation** — within one generation, the unsuffixed (shorter) id is taken as the
   base (`gpt-5.6` over `gpt-5.6-luna`). One known limit: a successor named by appending a letter
   inside one generation defeats this (`gpt-4` is shorter than `gpt-4o`).
4. **Price** — `latest:fast` and `latest:pro` take the cheapest of what is left, `latest:max` the
   dearest.

The id itself is the final tie-break. A dated snapshot or a `preview` id never wins while a
released one exists. OpenAI's o-series is a reasoning line and claims no tier, except
`o3-pro`/`o1-pro`, which are Max like the rest of the `-pro` line.

## Deprecated model ids

`armadai models check` reports agents whose `model` or `model_fallback` names a deprecated id, and
`armadai models update` rewrites them in place (in `.md` files and in `.armadai/agents.yaml`).
`--all` applies to every project registered by `armadai link`; `armadai models list` shows them.

## Secrets

`armadai config secrets init|rotate` still manages a SOPS + age encrypted
`config/providers.sops.yaml`, and `armadai config providers` shows the provider and secrets
status. No other command reads these API keys since execution was removed.
