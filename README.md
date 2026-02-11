# swarm-festai

AI agent fleet orchestrator — define, manage and run specialized agents from Markdown files.

[![CI](https://github.com/Dr0drigues/swarm-festai/actions/workflows/ci.yml/badge.svg)](https://github.com/Dr0drigues/swarm-festai/actions/workflows/ci.yml)
[![Security Audit](https://github.com/Dr0drigues/swarm-festai/actions/workflows/audit.yml/badge.svg)](https://github.com/Dr0drigues/swarm-festai/actions/workflows/audit.yml)

## Overview

swarm-festai lets you build a fleet of specialized AI agents, each configured with a simple Markdown file. It works with any LLM provider (Claude, GPT, Gemini) and any CLI tool (Claude Code, aider, etc.) through a unified interface.

```
swarm run code-reviewer "Review this PR for security issues"
swarm run --pipe code-reviewer test-writer "src/main.rs"
swarm tui
```

### Key Features

- **Markdown-based agents** — one `.md` file = one agent. Human-readable, git-friendly.
- **Multi-provider** — unified tool names (`claude`, `gemini`, `gpt`, `aider`) auto-detect CLI vs API; explicit API/CLI/proxy modes also supported
- **Pipeline mode** — chain agents sequentially (output A becomes input B)
- **TUI dashboard** — fleet management with agent browser, detail view, history, costs, and command palette
- **Shell completion** — auto-complete for bash, zsh, fish, powershell, elvish
- **Cost tracking** — per-agent, per-run cost monitoring stored in SurrealDB

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) (1.86+)
- [Docker](https://docs.docker.com/get-docker/) (optional, for SurrealDB server / LiteLLM proxy)
- [SOPS](https://github.com/getsops/sops) + [age](https://github.com/FiloSottile/age) (optional, for secret management)

### Install

```bash
git clone https://github.com/Dr0drigues/swarm-festai.git
cd swarm-festai
cargo build --release
```

The binary is at `target/release/swarm`.

### Configure providers

```bash
# Option A: Encrypted secrets (recommended)
swarm config secrets init       # Generates age key + .sops.yaml
sops config/providers.sops.yaml # Edit encrypted API keys

# Option B: Environment variables
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...

# Option C: Plain file (quick testing, gitignored)
cat > config/providers.secret.yaml << 'EOF'
providers:
  anthropic:
    api_key: sk-ant-...
  openai:
    api_key: sk-...
EOF

# Check provider status
swarm config providers
```

### Create your first agent

```bash
swarm new --template basic my-assistant
```

This creates `agents/my-assistant.md` — edit it to customize the system prompt, model, and behavior.

### Run an agent

```bash
swarm run my-assistant "Explain how async/await works in Rust"
```

## Usage

| Command | Description | Status |
|---|---|---|
| `swarm list [--tags t] [--stack s]` | List available agents | Done |
| `swarm new --template <tpl> <name>` | Create an agent from a template | Done |
| `swarm inspect <agent>` | Show parsed agent config | Done |
| `swarm validate [agent]` | Dry-run validation (no API calls) | Done |
| `swarm run <agent> [input]` | Run an agent | Done |
| `swarm run --pipe <a> <b> [input]` | Chain agents in a pipeline | Done |
| `swarm history [--agent a]` | View execution history | Done |
| `swarm history --replay <id>` | Replay a past execution | Planned |
| `swarm costs [--agent a] [--from d]` | View cost tracking | Done |
| `swarm config providers` | Show provider configs and secrets status | Done |
| `swarm config secrets init` | Initialize SOPS + age encryption | Done |
| `swarm config secrets rotate` | Rotate age encryption key | Done |
| `swarm tui` | Launch the TUI dashboard | Done |
| `swarm web [--port N]` | Launch the web UI | Done |
| `swarm completion <shell>` | Generate shell completions | Done |
| `swarm up / down` | Start/stop infra (Docker Compose) | Done |

## Agent Format

Each agent is a Markdown file in `agents/`:

```markdown
# Code Reviewer

## Metadata
- provider: claude
- model: claude-sonnet-4-5-20250929
- temperature: 0.3
- max_tokens: 4096
- tags: [dev, review, quality]
- stacks: [rust, typescript, java]

## System Prompt

You are an expert code reviewer...

## Instructions

1. Understand the context of the change
2. Identify bugs, security issues, performance problems
3. Provide constructive feedback

## Output Format

Structured review: bugs, security, performance, style.
```

### Provider names

Use unified tool names — swarm auto-detects CLI tool vs API:

| Provider | CLI tool detected | CLI not found |
|---|---|---|
| `claude` | Uses `claude` CLI | Falls back to Anthropic API |
| `gemini` | Uses `gemini` CLI | Falls back to Google API |
| `gpt` | Uses `gpt` CLI | Falls back to OpenAI API |
| `aider` | Uses `aider` CLI | Falls back to OpenAI API |

You can also use explicit providers: `anthropic`, `openai`, `google`, `cli`, `proxy`.

### Available sections

| Section | Required | Description |
|---|---|---|
| `# Title` (H1) | Yes | Agent name |
| `## Metadata` | Yes | Provider, model, temperature, tags, stacks, etc. |
| `## System Prompt` | Yes | System prompt sent to the model |
| `## Instructions` | No | Step-by-step execution instructions |
| `## Output Format` | No | Expected output format description |
| `## Pipeline` | No | List of agents to chain after this one |
| `## Context` | No | Additional context injected at runtime |

### Using explicit CLI provider

For custom scripts or tools not in the known list:

```markdown
# Custom Tool Agent

## Metadata
- provider: cli
- command: ./scripts/my-tool.sh
- args: ["--format", "json"]
- timeout: 60

## System Prompt

You are a versatile development assistant.
```

## Templates

| Template | Description |
|---|---|
| `basic` | General-purpose agent |
| `dev-review` | Code review specialist |
| `dev-test` | Test generation specialist |
| `cli-generic` | Wrapper for any CLI tool |
| `planning` | Sprint/project planning agent |
| `security-review` | Security audit specialist |
| `debug` | Debugging assistant |
| `tech-debt` | Technical debt analyzer |
| `tdd-red` | TDD red phase (write failing tests) |
| `tdd-green` | TDD green phase (make tests pass) |
| `tdd-refactor` | TDD refactor phase |
| `tech-writer` | Documentation writer |

Create an agent from a template:

```bash
swarm new my-reviewer --template dev-review --stack rust
swarm new my-tool --template cli-generic
```

## Shell Completion

Generate completion scripts for your shell:

```bash
# Bash
swarm completion bash > ~/.local/share/bash-completion/completions/swarm

# Zsh
swarm completion zsh > ~/.zfunc/_swarm

# Fish
swarm completion fish > ~/.config/fish/completions/swarm.fish
```

## TUI Dashboard

Launch with `swarm tui`. The dashboard provides fleet management views:

| Tab | Description |
|---|---|
| **Agents** | Browse all loaded agents with provider, model, and tags |
| **Detail** | View selected agent's full configuration (metadata, prompt, instructions) |
| **History** | Execution history with tokens, costs, and duration |
| **Costs** | Aggregated cost summary per agent |

### Keyboard shortcuts

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Switch tabs |
| `1-4` | Jump to tab directly |
| `j/k` or arrows | Navigate lists |
| `Enter` | View agent detail |
| `:` or `Ctrl+P` | Open command palette |
| `r` | Refresh data |
| `q` / `Esc` | Quit |

## Web UI

Launch with `swarm web` (default port 3000):

```bash
swarm web              # http://localhost:3000
swarm web --port 8080  # custom port
```

The web UI provides a read-only dashboard for your agent fleet:

- **Agents** — browse all loaded agents, click to view full configuration
- **History** — execution history with tokens, costs, and duration
- **Costs** — aggregated cost summary per agent

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed technical documentation.

```
HOST MACHINE
├── swarm (native binary)
│   ├── CLI + TUI + Web UI
│   ├── Providers (API / CLI / Proxy)
│   ├── SurrealDB (embedded)
│   └── SOPS + age secrets
│
└── docker-compose (optional)
    ├── surrealdb   :8000
    └── litellm     :4000
```

### Cargo Feature Flags

Heavy dependencies are gated behind optional feature flags for faster compilation:

| Feature | Default | Description |
|---|---|---|
| `tui` | Yes | TUI dashboard (ratatui + crossterm) |
| `web` | Yes | Web UI dashboard (axum + tower-http) |
| `storage` | Yes* | SurrealDB with in-memory backend |
| `storage-rocksdb` | Yes | SurrealDB with persistent RocksDB backend |
| `providers-api` | Yes | HTTP API providers (Anthropic, OpenAI, Google) |

\* Implied by `storage-rocksdb`.

```bash
# Full build (all features)
cargo build --release

# Lightweight build (no SurrealDB, no TUI)
cargo build --release --no-default-features

# CLI + storage (no TUI)
cargo build --release --no-default-features --features storage
```

## Development

### Setup

```bash
git clone https://github.com/Dr0drigues/swarm-festai.git
cd swarm-festai
git config core.hooksPath .githooks    # Enable commit message validation
```

### Build & Test

```bash
cargo build              # Build
cargo test               # Run tests
cargo clippy             # Lint
cargo fmt                # Format
RUST_LOG=debug cargo run  # Run with debug logs
```

### Git Flow

| Branch | Purpose |
|---|---|
| `master` | Production releases only |
| `develop` | Integration branch (default) |
| `feature/*` | New features (branch from `develop`) |
| `release/*` | Release preparation (branch from `develop`, merge to `master` + `develop`) |
| `hotfix/*` | Emergency fixes (branch from `master`, merge to `master` + `develop`) |

### Commits

This project enforces [Conventional Commits](https://www.conventionalcommits.org/).
A git hook validates messages automatically. Use `cz commit` for an interactive prompt.

```
feat: add agent validation command
fix(parser): handle empty metadata section
docs: update README with new CLI commands
refactor(providers): extract common HTTP logic
```

Changelogs are generated automatically via `cz bump`.

## Wiki

Detailed documentation is available in [`docs/wiki/`](docs/wiki/):

- [Getting Started](docs/wiki/getting-started.md) — installation, first agent, first run
- [Agent Format](docs/wiki/agent-format.md) — complete reference for agent Markdown files
- [Providers](docs/wiki/providers.md) — configuring API, CLI, and proxy providers
- [Templates](docs/wiki/templates.md) — using and creating agent templates

## License

MIT
