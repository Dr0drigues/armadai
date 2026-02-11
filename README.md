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
- **Multi-provider** — API (Anthropic, OpenAI, Google), CLI tools (claude, aider, any CLI), proxies (LiteLLM, OpenRouter)
- **Hub & spoke orchestration** — a coordinator agent dispatches tasks to specialists
- **Pipeline mode** — chain agents sequentially (output A becomes input B)
- **TUI dashboard** — real-time monitoring with streaming output, history, and cost tracking
- **Cost tracking** — per-agent, per-run cost monitoring stored in SurrealDB
- **SOPS + age encryption** — API keys encrypted at rest, diff-friendly

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
# Initialize secret management
swarm config secrets init

# Edit your API keys (encrypted)
sops config/providers.sops.yaml
```

Or for quick testing, create `config/providers.secret.yaml` (unencrypted, gitignored):

```yaml
providers:
  anthropic:
    api_key: sk-ant-...
  openai:
    api_key: sk-...
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
| `swarm run <agent> [input]` | Run an agent | Planned |
| `swarm run --pipe <a> <b> [input]` | Chain agents in a pipeline | Planned |
| `swarm history [--agent a]` | View execution history | Planned |
| `swarm history --replay <id>` | Replay a past execution | Planned |
| `swarm costs [--agent a] [--from d]` | View cost tracking | Planned |
| `swarm config providers` | Manage provider configs | Planned |
| `swarm config secrets init` | Initialize SOPS + age | Planned |
| `swarm tui` | Launch the TUI dashboard | Planned |
| `swarm up / down` | Start/stop infra (Docker Compose) | Done |

## Agent Format

Each agent is a Markdown file in `agents/`:

```markdown
# Code Reviewer

## Metadata
- provider: anthropic
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

### Using CLI tools as providers

```markdown
# Claude Code Agent

## Metadata
- provider: cli
- command: claude
- args: ["-p", "--model", "sonnet", "--output-format", "json"]
- timeout: 300

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

Create an agent from a template:

```bash
swarm new my-reviewer --template dev-review --stack rust
swarm new my-tool --template cli-generic
```

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed technical documentation.

```
HOST MACHINE
├── swarm (native binary)
│   ├── CLI + TUI
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
| `storage` | Yes* | SurrealDB with in-memory backend |
| `storage-rocksdb` | Yes | SurrealDB with persistent RocksDB backend |

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
