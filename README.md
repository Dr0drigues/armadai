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

```
swarm run <agent> [input]           Run an agent
swarm run --pipe <a> <b> [input]    Chain agents in a pipeline
swarm new --template <tpl> <name>   Create an agent from a template
swarm list [--tags t] [--stack s]   List available agents
swarm inspect <agent>               Show parsed agent config
swarm validate [agent]              Dry-run validation (no API calls)
swarm history [--agent a]           View execution history
swarm history --replay <id>         Replay a past execution
swarm costs [--agent a] [--from d]  View cost tracking
swarm config providers              Manage provider configs
swarm config secrets init           Initialize SOPS + age
swarm tui                           Launch the TUI dashboard
swarm up                            Start infra (Docker Compose)
swarm down                          Stop infra
```

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

## Templates

| Template | Description |
|---|---|
| `basic` | General-purpose agent |
| `dev-review` | Code review specialist |
| `dev-test` | Test generation specialist |
| `cli-generic` | Wrapper for any CLI tool |

## Development

```bash
cargo build              # Build
cargo test               # Run tests
cargo clippy             # Lint
cargo fmt                # Format
RUST_LOG=debug cargo run  # Run with debug logs
```

## License

MIT
