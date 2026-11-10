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

`armadai models tiers` shows this table.

Sources, read on 2026-11-10 (the ids are copied from these pages, not from memory):

- OpenAI: <https://developers.openai.com/api/docs/models> (`gpt-6-luna`, `gpt-6.1-sol`,
  `gpt-6-astra`), <https://developers.openai.com/api/docs/deprecations>, and
  <https://developers.openai.com/api/docs/models/all>, the only models page that names the
  `gpt-5.6-*` ids the aliases point to;
- Anthropic: <https://platform.claude.com/docs/en/about-claude/models/overview>;
- Google: <https://aistudio.google.com/docs/models> and
  <https://aistudio.google.com/docs/deprecations>. `gemini-3.1-pro-preview` is a preview, the only
  Pro the page lists.

The same table carries the aliases `armadai models check` uses: the replacements those
deprecation pages name. One of them moves a Pro to a Flash, `gemini-2.5-pro` to
`gemini-3.8-flash`, as Google's deprecation page does, and `models check` proposes it. The table is refreshed from those pages at release time; to use a newer model before that, pin
`native.<target>.model`.

## Syncing the models cache

`armadai models sync` downloads the models.dev catalog (`https://models.dev/api.json`, through
`curl`, HTTPS only) and writes it to `<config dir>/models-cache.json` (provider ids, model ids,
names, prices, limits). The write is atomic: on any error (download failed, answer is not a
catalog) the previous cache is left as it was and the error says so. `--source <url>` reads
another HTTPS catalog of the same shape.

`armadai link` never reads this cache and never touches the network: the table above is what it
uses, whatever the cache holds. No `armadai` command reads the cache today: only the core's
resolution function (`resolve_model_for_tier`) and the frozen audit tool do. `armadai models sync`
is the only command of this page that uses the network.

## How the table is refreshed

At release time, `mise run models-refresh` (`scripts/refresh-models.sh`) syncs the models.dev
catalog into a throwaway config dir, re-resolves each tier of the table from it, and prints a
diff against `crates/armadai-core/data/models.json`. It writes nothing in the repository and
needs the network.

The diff is a proposal. A catalog id is not proof that the vendor publishes it for that tier:
check each changed id against the vendor pages listed above, then edit `models.json` (and the
table above) by hand and commit.

## Deprecated model ids

Canonical agents carry a tier; a model id only appears where an agent pins one, in
`native.<target>.model`. `armadai models check` reports the deprecated ones, and
`armadai models update` replaces them in place (only the id, in the frontmatter: comments and
the body are kept; a replacement that is a tier is reported, not written). `--all` applies to
every project registered by `armadai link`; `armadai models list` shows them. A legacy project
is reported "legacy format, run `armadai migrate`".
