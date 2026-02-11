# Providers

swarm-festai supports three types of providers for executing agents.

## API Providers

Direct HTTP calls to LLM APIs.

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

**Claude Code:**
```markdown
- provider: cli
- command: claude
- args: ["-p"]
- timeout: 600
```

**aider:**
```markdown
- provider: cli
- command: aider
- args: ["--message"]
- timeout: 300
```

**Custom script:**
```markdown
- provider: cli
- command: ./scripts/my-tool.sh
- args: ["--format", "json"]
- timeout: 60
```

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

API keys are stored in `config/providers.sops.yaml` (encrypted) or `config/providers.secret.yaml` (unencrypted, gitignored).

### Quick setup (unencrypted)

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

```bash
# Initialize encryption
swarm config secrets init

# Edit encrypted file
sops config/providers.sops.yaml
```

See the [SOPS documentation](https://github.com/getsops/sops) for details on the encryption workflow.

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
