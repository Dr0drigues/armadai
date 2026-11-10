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
| `opencode` | `native.opencode.model`, else the tier through `model_tiers` of your `~/.config/armadai/config.yaml` when it maps it; otherwise no model (the agent inherits the session's) |
| `copilot` | none: Copilot agent files carry no model (reported `not-compiled`) |

See [Link](link.md) for the rest of each target's output.

## The static tier table

The table is built into the binary (`crates/armadai-core/data/models.json`), and `link` never
reads a model catalogue. Today only Codex turns a tier into a model with it (through the
`openai` row); Claude writes aliases and OpenCode reads your `model_tiers`. A vendor that has no
`max` model answers its `pro` one (Google).

<!-- tier-table:start -->
Table as of 2026-11-10.

| Vendor | `fast` | `pro` | `max` |
|---|---|---|---|
| `anthropic` | `claude-haiku-5-5` | `claude-sonnet-5-5` | `claude-opus-5-5` |
| `google` | `gemini-3.6-flash` | `gemini-3.1-pro-preview` | `gemini-3.1-pro-preview` |
| `openai` | `gpt-6-luna` | `gpt-6.1-sol` | `gpt-6-astra` |
<!-- tier-table:end -->

Sources, read on 2026-11-10 (the ids are copied from these pages, not from memory):

- OpenAI: <https://developers.openai.com/api/docs/models> (`gpt-6-luna`, `gpt-6.1-sol`,
  `gpt-6-astra`) and <https://developers.openai.com/api/docs/deprecations>;
- Anthropic: <https://platform.claude.com/docs/en/about-claude/models/overview>;
- Google: <https://aistudio.google.com/docs/models> and
  <https://aistudio.google.com/docs/deprecations>. `gemini-3.1-pro-preview` is a preview, the only
  Pro the page lists.

The same table carries the aliases `armadai models check` uses: the replacements those
deprecation pages name. The table is refreshed from those pages (`armadai models sync` is planned
for that); until then, pin `native.<target>.model` when you need a newer model.

## Deprecated model ids

Canonical agents carry a tier; a model id only appears where an agent pins one, in
`native.<target>.model`. `armadai models check` reports the deprecated ones, and
`armadai models update` replaces them in place (only the id, in the frontmatter: comments and
the body are kept; a replacement that is a tier is reported, not written). `--all` applies to
every project registered by `armadai link`; `armadai models list` shows them. A legacy project
is reported "legacy format, run `armadai migrate`".
