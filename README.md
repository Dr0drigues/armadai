# ArmadAI

AI agent fleet orchestrator — define, manage and run specialized agents from Markdown files.

[![CI](https://github.com/Dr0drigues/swarm-festai/actions/workflows/ci.yml/badge.svg)](https://github.com/Dr0drigues/swarm-festai/actions/workflows/ci.yml)
[![Security Audit](https://github.com/Dr0drigues/swarm-festai/actions/workflows/audit.yml/badge.svg)](https://github.com/Dr0drigues/swarm-festai/actions/workflows/audit.yml)

## Overview

ArmadAI lets you build a fleet of specialized AI agents, each configured with a simple Markdown file. It works with any LLM provider (Claude, GPT, Gemini) and any CLI tool (Claude Code, aider, etc.) through a unified interface.

```
armadai run code-reviewer "Review this PR for security issues"
armadai run --pipe code-reviewer test-writer "src/main.rs"
armadai tui
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
# One-liner (downloads the latest release binary)
curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash
```

Options: `INSTALL_DIR=~/.local/bin` (default), `VERSION=v0.1.0` (default: latest).

```bash
# Custom install directory
INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash

# Specific version
VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash
```

### Install from source

```bash
git clone https://github.com/Dr0drigues/armadai.git
cd armadai
cargo build --release
```

The binary is at `target/release/armadai`.

### Configure providers

```bash
# Option A: Encrypted secrets (recommended)
armadai config secrets init       # Generates age key + .sops.yaml
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
armadai config providers
```

### Create your first agent

```bash
armadai new --template basic my-assistant
```

This creates `agents/my-assistant.md` — edit it to customize the system prompt, model, and behavior.

### Run an agent

```bash
armadai run my-assistant "Explain how async/await works in Rust"
```

## Usage

| Command | Description | Status |
|---|---|---|
| `armadai list [--tags t] [--stack s]` | List available agents | Done |
| `armadai new --template <tpl> <name>` | Create an agent from a template | Done |
| `armadai inspect <agent>` | Show parsed agent config | Done |
| `armadai validate [agent]` | Dry-run validation (no API calls) | Done |
| `armadai run <agent> [input]` | Run an agent | Done |
| `armadai run --pipe <a> <b> [input]` | Chain agents in a pipeline | Done |
| `armadai history [--agent a]` | View execution history | Done |
| `armadai history --replay <id>` | Replay a past execution | Planned |
| `armadai costs [--agent a] [--from d]` | View cost tracking | Done |
| `armadai config providers` | Show provider configs and secrets status | Done |
| `armadai config secrets init` | Initialize SOPS + age encryption | Done |
| `armadai config secrets rotate` | Rotate age encryption key | Done |
| `armadai init [--force] [--project]` | Initialize ArmadAI configuration | Done |
| `armadai init --pack <name>` | Install a starter pack (rust-dev, fullstack) | Done |
| `armadai fleet create/link/list/show` | Manage agent fleets | Done |
| `armadai link --target <t> [--dry-run]` | Generate native AI assistant configs | Done |
| `armadai registry sync/search/list/add` | Browse and import community agents | Done |
| `armadai prompts list/show` | Manage composable prompts | Done |
| `armadai skills list/show` | Manage composable skills | Done |
| `armadai update` | Self-update to latest release | Done |
| `armadai tui` | Launch the TUI dashboard | Done |
| `armadai web [--port N]` | Launch the web UI | Done |
| `armadai completion <shell>` | Generate shell completions | Done |
| `armadai up / down` | Start/stop infra (Docker Compose) | Done |

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

Use unified tool names — ArmadAI auto-detects CLI tool vs API:

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
armadai new my-reviewer --template dev-review --stack rust
armadai new my-tool --template cli-generic
```

## Starter Packs

Install curated bundles of agents and prompts:

```bash
armadai init --pack rust-dev      # Code reviewer, test writer, debug agent + Rust conventions
armadai init --pack fullstack     # Full stack of 6 agents for web development
```

Available packs:

| Pack | Agents | Description |
|---|---|---|
| `rust-dev` | code-reviewer, test-writer, debug | Rust development essentials + conventions prompt |
| `fullstack` | code-reviewer, test-writer, doc-generator, planning-agent, security-reviewer, tech-debt-analyzer | Full stack web development |

## Shell Completion

Generate completion scripts for your shell:

```bash
# Bash
armadai completion bash > ~/.local/share/bash-completion/completions/armadai

# Zsh
armadai completion zsh > ~/.zfunc/_armadai

# Fish
armadai completion fish > ~/.config/fish/completions/armadai.fish
```

## TUI Dashboard

Launch with `armadai tui`. The dashboard provides fleet management views:

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

Launch with `armadai web` (default port 3000):

```bash
armadai web              # http://localhost:3000
armadai web --port 8080  # custom port
```

The web UI provides a read-only dashboard for your agent fleet:

- **Agents** — browse all loaded agents, click to view full configuration
- **History** — execution history with tokens, costs, and duration
- **Costs** — aggregated cost summary per agent

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed technical documentation.

```
HOST MACHINE
├── armadai (native binary)
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
- [Starter Packs](docs/wiki/starter-packs.md) — curated agent bundles
- [Registry](docs/wiki/registry.md) — browsing and importing community agents

## License

MIT
