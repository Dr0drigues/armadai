# Providers

swarm-festai supports three types of providers for executing agents, plus **unified tool names** that auto-detect the best backend.

## Unified Tool Names (Recommended)

Use a tool name directly as the provider. swarm auto-detects whether the CLI tool is installed and falls back to the API if not.

| Provider | CLI tool | API fallback | Default CLI args |
|---|---|---|---|
| `claude` | `claude` | Anthropic API | `-p --output-format text` |
| `gemini` | `gemini` | Google API | `-p` |
| `gpt` | `gpt` | OpenAI API | (none) |
| `aider` | `aider` | OpenAI API | `--message` |

### Example

```markdown
## Metadata
- provider: claude
- model: claude-sonnet-4-5-20250929
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

## API Providers

Direct HTTP calls to LLM APIs. Use these when you want explicit API control.

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
- model: gpt-4o
- temperature: 0.7
```

Available models: `gpt-4o`, `gpt-4o-mini`, `o1`

### Google

```markdown
## Metadata
- provider: google
- model: gemini-2.0-flash
- temperature: 0.7
```

## CLI Provider

Execute any command-line tool as an agent. The input is passed as the last argument to the command, and stdout is captured as the output.

```markdown
## Metadata
- provider: cli
- command: claude
- args: ["-p", "--model", "sonnet", "--output-format", "json"]
- timeout: 300
```

### Examples

**Custom script:**
```markdown
- provider: cli
- command: ./scripts/my-tool.sh
- args: ["--format", "json"]
- timeout: 60
```

> **Note:** For standard tools (claude, gemini, gpt, aider), prefer using the unified tool name (`provider: claude`) instead of `provider: cli` + `command: claude`. The unified names auto-detect CLI availability and provide sensible defaults.

## Proxy Provider

Route requests through an OpenAI-compatible proxy like LiteLLM or OpenRouter.

```markdown
## Metadata
- provider: proxy
- model: anthropic/claude-sonnet-4-5-20250929
```

### Setting up LiteLLM

Start the proxy with Docker Compose:

```bash
swarm up
```

This starts LiteLLM on port 4000 (configured in `docker-compose.yml`).

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
swarm config secrets init

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
swarm config secrets rotate
```

This decrypts current secrets, generates a new age key, re-encrypts with the new key, and backs up the old key.

### Check provider status

```bash
swarm config providers
```

Shows configured providers, secrets status (encrypted/unencrypted/missing), and environment variable status.

## Provider Configuration

Global provider settings are in `config/providers.yaml`:

```yaml
providers:
  anthropic:
    base_url: https://api.anthropic.com
    default_model: claude-sonnet-4-5-20250929
  openai:
    base_url: https://api.openai.com/v1
    default_model: gpt-4o
  proxy:
    base_url: http://localhost:4000
```
