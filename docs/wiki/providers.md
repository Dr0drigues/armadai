# Providers

ArmadAI supports three types of providers for executing agents — **API** (direct HTTP), **CLI** (run a command-line tool) and **proxy** (any OpenAI-compatible server) — plus **unified tool names** that auto-detect the best backend.

## Unified Tool Names (Recommended)

Use a tool name directly as the provider. ArmadAI auto-detects whether the CLI tool is installed and falls back to the API if not.

| Provider | Command `armadai run` spawns | API fallback when the CLI is missing | `armadai link --target` |
|---|---|---|---|
| `claude` | `claude -p --output-format stream-json --verbose` | Anthropic API | yes |
| `gemini` | `gemini -o stream-json -p` | Google API | yes |
| `codex` | `codex exec --json` | none — CLI only | yes |
| `copilot` | `copilot --output-format json -p` | none — CLI only | yes |
| `opencode` | `opencode run --format json` | none — CLI only | yes |
| `gpt` | none — API only | OpenAI API | no |
| `aider` | `aider --message` | OpenAI API | no |

The prompt is always appended as the last argument, which is why any flag that
*takes* the prompt as its value (`-p`) comes last.

### Runnable and linkable are two different things

They are easy to confuse, and confusing them is what produced
[#369](https://github.com/Dr0drigues/armadai/issues/369):

- **runnable** — `armadai run` accepts the name in an agent's `provider:` and
  spawns (or calls) something for it. That is the first column above.
- **linkable** — `armadai link --target <name>` generates that tool's *own*
  native config from your ArmadAI agents, so the tool sees them as its own
  sub-agents. That is the last column.

Every link target is runnable. The reverse does not hold: `gpt` and `aider`
run but have no linker, because neither has an ArmadAI-shaped agent config to
generate.

### CLI-only providers

`codex`, `copilot` and `opencode` are **CLI-only**: their vendors expose the
agent behind the command-line tool, not behind an API an ArmadAI agent's
plain-text exchange could call. When the binary is not on `PATH`, ArmadAI says
so instead of falling back to an unrelated API:

```
Error: Provider 'codex' runs the `codex` CLI, which was not found on PATH, and
it has no API backend to fall back to. Install it, or point this agent at the
executable with `command: /full/path/to/codex`.
```

### API-only providers

`gpt` is the mirror image: it names **no** CLI, and goes straight to the
OpenAI API. It used to probe `PATH` for a binary called `gpt` — and on macOS
it always found one, because `/usr/sbin/gpt` is the system GUID-partition-table
tool that ships with the OS. Every `provider: gpt` agent on macOS therefore
ran a disk utility and failed with `gpt: unknown command: …`, never reaching
the OpenAI fallback ([#402](https://github.com/Dr0drigues/armadai/issues/402)).

If you do have a CLI you want `provider: gpt` to spawn, say so explicitly —
an agent's own `command:` still wins over the API for every unified name:

```markdown
- provider: gpt
- command: /opt/homebrew/bin/my-gpt-cli
- args: ["--prompt"]
```

### Any other tool

A provider name only has to be known for ArmadAI to supply defaults. Any other
command-line tool runs through `provider: cli` (see [CLI
Provider](#cli-provider)), which spawns exactly what `command:` and `args:`
say and nothing else — so a tool that needs a subcommand or a flag to answer
non-interactively needs that flag written out in `args:`.

### Example

```markdown
## Metadata
- provider: claude
- model: latest:pro
- timeout: 120
- tags: [dev, review]
```

If `claude` CLI is installed, the agent runs via CLI. Otherwise, it uses the Anthropic API (requires `ANTHROPIC_API_KEY`).

You can override the CLI args:

```markdown
## Metadata
- provider: claude
- args: [-p, --model, opus, --output-format, json]
- timeout: 600
```

> **`timeout` bounds inactivity, not total duration.** When the CLI tool is
> used (as opposed to the API fallback), `timeout` is the longest gap
> allowed between two lines of subprocess output — it resets every time the
> CLI produces something, including a native sub-agent delegation. A call
> that keeps streaming output can run well past `timeout` seconds in total;
> one that goes fully silent for that long is killed. See [CLI
> Provider](#cli-provider) below for the full explanation.

## Model Selection

When using `armadai new -i` (interactive wizard), the model selection step fetches available models from the [models.dev](https://models.dev) registry with enriched metadata (context window size, input/output cost per MTok). Results are cached locally for 24 hours. If the registry is unreachable, the wizard falls back to the static model list in `providers.yaml`.

```
? Model
> Claude Sonnet 4.5 — 200K ctx — $3.00/$15.00
  Claude Haiku 4.5 — 200K ctx — $0.80/$4.00
  Claude Opus 4 — 200K ctx — $15.00/$75.00
  (custom)
```

## API Providers

Direct HTTP calls to LLM APIs. Use these when you want explicit API control.

> **Tier placeholders work on every path.** `latest` / `latest:fast` /
> `latest:pro` / `latest:max` are resolved to a concrete model id everywhere
> a model is needed: when ArmadAI *generates a native CLI config*
> (`armadai link`), in `armadai shell`, and on the `armadai run` path — for a
> single agent, for a `--pipe` chain, for `--orchestrate`, and for `--resume`
> alike.
>
> **Which vendor's catalog is read** comes from the agent's own `provider:`,
> mapped to the vendor that names its models: `gemini` → Google, `claude` →
> Anthropic, `gpt`/`aider`/`codex` → OpenAI. The concrete id is then the best
> match for the tier in the cached [models.dev](https://models.dev) catalog,
> or a built-in table when the cache is absent. All three commands read the
> same mapping — until #398 `shell` was the only one that had it, and `run`
> resolved a `provider: gemini` agent's `latest:pro` to a *Claude* model.
>
> A provider with no vendor of its own — `provider: cli`, the CLI-only tools
> (`copilot`, `opencode`), and `provider: proxy` — keeps the placeholder
> instead. For a relayed CLI the model is not sent at all (the tool picks its
> own), and for a gateway the placeholder is the more useful string: an
> administrator can route `latest:max` through a house alias, which a
> concrete id chosen here would override with no way to opt out.
>
> **Which model in the tier** is decided by two keys, in order. First the
> **generation** — the numbers in the id, compared as numbers: `latest:pro`
> asks for the *latest* model of the Pro tier, and generation `10` is above
> generation `4.6` even though the string `gpt-10` sorts below `gpt-9`. Then
> the **price**, in the direction the tier promises: `latest:fast` ("cheap
> and fast") and `latest:pro` ("balanced") take the cheapest of that
> generation, `latest:max` ("maximum capability") the dearest. A model the
> catalog does not price never wins over one it does.
>
> Until [#404](https://github.com/Dr0drigues/armadai/issues/404) there was
> one key and it was the alphabet, described in the code as "the highest
> version". It is not the same thing: `latest:fast` on OpenAI answered
> `o4-mini` — a reasoning model at $1.10/$4.40 per Mtok against the tier's
> own example `gpt-4o-mini` at $0.15/$0.60 — because `o` sorts after `g`.
>
> **A vendor may not distinguish every tier.** Google publishes no line above
> `pro`, so `latest:max` on Google *is* `latest:pro`, deliberately and
> explicitly — and it now resolves through the catalog like any other tier
> rather than freezing on a built-in id.
>
> **OpenAI's o-series** (`o1`, `o3`, `o4-mini`, …) is a reasoning line rather
> than a rung on the chat price ladder these three tiers describe, so it
> claims no tier — except `o3-pro`/`o1-pro`, which are Max like the rest of
> the `-pro` line.
>
> `latest:auto` is the one placeholder resolved *per call* rather than up
> front: its tier is chosen from the run's own input by the router
> (configured under `routing:` in `armadai.yaml`).
>
> This was not always true — until #376 the `run` path sent the static tiers
> verbatim, so a placeholder that worked under `armadai link` produced a
> "model not found" under `armadai run`.

### Anthropic

```markdown
## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.3
- max_tokens: 4096
```

Available models: `claude-opus-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`

### OpenAI

```markdown
## Metadata
- provider: openai
- model: gpt-4o-mini
- temperature: 0.7
```

Available models: `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini`,
`gpt-4.1-nano`, `o1`, `o1-mini`, `o3-mini` — plus anything else your endpoint
serves. That is the same list `metadata().models` advertises and the same one
the cost table prices; a test keeps the two from drifting apart.

Requires `OPENAI_API_KEY` (see [Secret Management](#secret-management)). Both
`complete()` and streaming are supported.

The base URL defaults to `https://api.openai.com/v1` and can be redirected —
see [OpenAI-compatible servers](#openai-compatible-servers) below, which is the
same code path.

> **Reasoning models (`o1`, `o3`).** They reject `max_tokens` and any
> `temperature` other than `1`. ArmadAI only sends `max_tokens` when the agent
> declares one, so leave it out for those models. `temperature` **is** always
> sent, so set `temperature: 1.0` explicitly.
>
> The asymmetry is a property of the domain type, not an oversight:
> `max_tokens` is an `Option` in an agent's metadata, so "the author did not
> ask for one" is representable and the field can simply be left off the wire.
> `temperature` is a plain number defaulting to `0.7`, so there is no value
> that means "unset" — omitting it would require guessing which `0.7` was
> deliberate. Dropping it for model ids that look like reasoning models was
> considered and rejected: a gateway can serve anything under any name, and
> silently discarding an author's `temperature: 0.2` is worse than the clear
> `400` OpenAI returns, which names the parameter and the value.

### Google

```markdown
## Metadata
- provider: google
- model: gemini-2.5-pro
- temperature: 0.7
```

## Model Fallback

Declare fallback models for automatic retry when the primary model is unavailable (404, "model not found"). All fallbacks must use the same provider.

```markdown
## Metadata
- provider: google
- model: gemini-2.5-pro
- model_fallback: [gemini-2.5-flash, gemini-2.0-flash]
```

If the primary model returns a "model not found" error, ArmadAI automatically retries with each fallback in order. Non-model errors (auth, rate limit) are not retried.

The plural alias `model_fallbacks` is also accepted.

## CLI Provider

Execute any command-line tool as an agent. The input is passed as the last argument to the command, and stdout is captured as the output.

```markdown
## Metadata
- provider: cli
- command: claude
- args: ["-p", "--model", "sonnet", "--output-format", "json"]
- timeout: 300
```

`timeout` here is an **inactivity** timeout, not a total-duration one: it is
the longest gap allowed between two consecutive lines the command writes to
stdout, and it is rearmed on every line — a delegation event, a token
delta, or (for a CLI with no structured output) just the next chunk of
text. A command that keeps producing output can run well past `timeout`
seconds in total. A command that goes fully silent for `timeout` seconds
(a hung process, a deadlocked script) is killed — that failure mode is
exactly what `timeout` still protects against, so a script wrapped by
`provider: cli` should still print *something* periodically if it expects
to run long. There is also a generous absolute backstop (2 hours) so a
process that never goes silent — but never finishes either — still ends
eventually.

### Examples

**Custom script:**
```markdown
- provider: cli
- command: ./scripts/my-tool.sh
- args: ["--format", "json"]
- timeout: 60
```

> **Note:** For the tools listed under [Unified Tool Names](#unified-tool-names-recommended)
> — claude, gemini, codex, copilot, opencode, gpt, aider — prefer the unified
> name (`provider: codex`) over `provider: cli` + `command: codex`. They are not
> equivalent: the unified name auto-detects CLI availability (for the six that
> have a CLI), falls back to the API where one exists, and supplies the tool's
> non-interactive argv, whereas
> `provider: cli` with no `args:` passes the prompt alone — and `codex` with a
> bare positional opens its interactive UI, while `opencode` reads it as a
> project path.

## OpenAI-compatible servers

`POST {base_url}/chat/completions` (plus SSE for streaming) is spoken by far
more than OpenAI itself. `provider: proxy` is exactly the `openai` provider
with a user-supplied base URL and an **optional** API key, so a single
implementation reaches:

| Kind | Examples | Base URL shape |
|---|---|---|
| Gateways | LiteLLM, OpenRouter, Groq, Together, Fireworks | `https://openrouter.ai/api/v1` |
| Vendor endpoints | DeepSeek, Mistral | `https://api.deepseek.com/v1` |
| Local runtimes | Ollama, vLLM, LM Studio, llama.cpp | `http://localhost:11434/v1` |

```markdown
## Metadata
- provider: proxy
- model: openai/gpt-4o-mini
- temperature: 0.7
```

### Where the base URL comes from

Resolved in this order, first match wins:

1. `PROXY_BASE_URL` (environment variable)
2. `providers.proxy.base_url` in `providers.yaml`
3. `http://localhost:4000/v1` (the default LiteLLM port)

All four API providers follow the same two-step override — the environment
variable first, then `providers.<name>.base_url` in `providers.yaml`:
`OPENAI_BASE_URL`, `PROXY_BASE_URL`, `ANTHROPIC_BASE_URL`, `GOOGLE_BASE_URL`.
`armadai init` writes a `base_url` for all four in that file, so a value put
there is honoured whichever provider it belongs to. A blank value is not a
configuration and is ignored, in either source.

### Authentication is optional

`PROXY_API_KEY` (or a `proxy:` entry in the secrets file) is sent as
`Authorization: Bearer …`. If neither is set, **no `Authorization` header is
sent at all** — which is what a keyless local gateway or an Ollama server
expects. An empty `Bearer ` would be rejected by several servers, so it is
never sent.

### Example: a hosted gateway (OpenRouter)

```bash
export PROXY_BASE_URL=https://openrouter.ai/api/v1
export PROXY_API_KEY=sk-or-v1-…
```

```markdown
## Metadata
- provider: proxy
- model: anthropic/claude-sonnet-4.5
```

### Example: a local runtime (Ollama)

Ollama exposes an OpenAI-compatible surface and needs no key:

```bash
ollama serve            # listens on 11434
export PROXY_BASE_URL=http://localhost:11434/v1
```

```markdown
## Metadata
- provider: proxy
- model: llama3.1
- temperature: 0.7
```

The same shape works for vLLM (`http://localhost:8000/v1`), LM Studio
(`http://localhost:1234/v1`) and llama.cpp's server.

### Example: LiteLLM via Docker Compose

`docker-compose.yml` ships a LiteLLM service, but it sits behind the `proxy`
compose profile and mounts a config file you provide, so plain `armadai up`
does **not** start it:

```bash
# write your own config/litellm.yaml first (model_list: …)
docker compose --profile proxy up -d litellm
export PROXY_BASE_URL=http://localhost:4000/v1
```

### What is not supported

- **Azure OpenAI.** It diverges from the dialect on three axes: an `api-key`
  header instead of `Authorization: Bearer`, deployment-scoped paths
  (`/openai/deployments/{deployment}/chat/completions`) and a mandatory
  `api-version` query parameter. ArmadAI does not handle it, and pointing
  `base_url` at an Azure endpoint will fail. Reach Azure through a gateway
  (LiteLLM) that presents an OpenAI-shaped front instead.
- **Tool/function calling, structured outputs, images, audio.** An ArmadAI
  agent exchanges plain text, so these have nothing to map onto.
- **Cost for unknown models.** Only OpenAI's own model ids are priced
  (`gpt-4o`, `gpt-4o-mini`, the `gpt-4.1` family, `o1`, `o1-mini`, `o3-mini`,
  with any vendor prefix such as `openai/` stripped). Anything else reports
  `$0.00` in `armadai costs` rather than an invented figure. A response that
  carries no `usage` block at all — common on gateways and local runtimes —
  reports zero tokens and zero cost; the call itself still succeeds.

  One consequence is worth knowing before you rely on a ceiling: an
  orchestrated run's `token_budget` and `cost_limit` are enforced from those
  same numbers, so against an endpoint that never reports `usage` they never
  trigger, and each nested delegation is handed the full ceiling rather than
  what is left of it. ArmadAI says so once per run — a `budget_usage_unreported`
  warning on `--json`, and a log line otherwise — rather than letting the limit
  look enforced.

### Errors that arrive with a `200`

Not every failure on this path comes with a failing status code. Ollama and
LiteLLM in pass-through mode answer `200` with `{"error": {...}}` in the body,
and once a *stream* has started the status line is already gone — OpenAI's own
documented behaviour for anything that fails mid-stream is to send the error as
an SSE frame. ArmadAI treats both as errors rather than as an empty answer:

- a non-streaming `200` whose body carries an error envelope and no `choices`
  fails with the server's own message;
- an error frame mid-stream ends the stream with that message, instead of
  quietly delivering the tokens received so far as if they were the whole
  answer.

The reason this needs saying is that every field of an OpenAI-shaped response
is optional here (`usage`, `model`, even `choices`, because real servers omit
them), so an error envelope would otherwise parse cleanly into a successful,
empty, free run.

### Rate limits and retries

Every HTTP provider, this one included, goes through the same retry policy:
`429`, `503` and `529` are retried with exponential backoff (up to 3 retries),
honouring the server's `Retry-After` header when it sends one in
delta-seconds form. Other statuses — `400`, `401`, `404` — are terminal and
surfaced immediately with the server's own message.

An agent can also throttle itself before it hits the wire with the
`rate_limit` metadata field (`"10/min"`), and a per-provider ceiling can be
set under `rate_limits` in `config.yaml`.

## Secret Management

API keys can be provided in three ways (checked in order):

1. **Environment variables** — `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GOOGLE_API_KEY`
2. **Encrypted file** — `config/providers.sops.yaml` (SOPS + age)
3. **Plain file** — `config/providers.secret.yaml` (gitignored)

### Quick setup (environment variables)

```bash
export ANTHROPIC_API_KEY=sk-ant-your-key
export OPENAI_API_KEY=sk-your-key
```

### Quick setup (plain file, gitignored)

```yaml
# config/providers.secret.yaml
providers:
  anthropic:
    api_key: sk-ant-your-key
  openai:
    api_key: sk-your-key
  google:
    api_key: AIza-your-key
```

### Production setup (SOPS + age)

Prerequisites: [SOPS](https://github.com/getsops/sops) and [age](https://github.com/FiloSottile/age).

```bash
# Initialize encryption (generates age key + .sops.yaml + template)
armadai config secrets init

# Set the key file in your shell profile
export SOPS_AGE_KEY_FILE=config/age-key.txt

# Edit encrypted secrets
sops config/providers.sops.yaml
```

The `init` command:
1. Generates an age key pair at `config/age-key.txt`
2. Creates `.sops.yaml` with the public key
3. Creates and encrypts a template `config/providers.sops.yaml`

### Key rotation

```bash
armadai config secrets rotate
```

This decrypts current secrets, generates a new age key, re-encrypts with the new key, and backs up the old key.

### Check provider status

```bash
armadai config providers
```

Shows configured providers, secrets status (encrypted/unencrypted/missing), and environment variable status.

## Provider Configuration

Global provider settings live in `providers.yaml` (`~/.config/armadai/providers.yaml`,
falling back to a project-local `config/providers.yaml`):

```yaml
providers:
  openai:
    base_url: https://api.openai.com/v1
    models:
      - gpt-4o
      - gpt-4o-mini
  proxy:
    base_url: http://localhost:11434/v1
    models: []
```

Two keys are read, and only these two:

| Key | Read by |
|---|---|
| `base_url` | provider construction, for `openai` and `proxy` only (see [above](#where-the-base-url-comes-from)) |
| `models` | the `armadai new -i` wizard, as the fallback model list when models.dev is unreachable |

There is no `default_model` key — the model comes from the agent's `## Metadata`
(or `defaults.model` in `config.yaml`).
