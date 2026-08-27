<p align="center">
  <img src="assets/brand/armadai-wordmark.svg" alt="ArmadAI" width="300">
</p>

<p align="center">
  AI agent orchestrator — define, manage and run specialized agents from Markdown files.
</p>

<p align="center">
  <a href="https://github.com/Dr0drigues/armadai/actions/workflows/ci.yml"><img src="https://github.com/Dr0drigues/armadai/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/Dr0drigues/armadai/actions/workflows/audit.yml"><img src="https://github.com/Dr0drigues/armadai/actions/workflows/audit.yml/badge.svg" alt="Security Audit"></a>
</p>

## Overview

ArmadAI lets you build a team of specialized AI agents, each configured with a simple Markdown file. It works with any LLM provider (Claude, GPT, Gemini) and any CLI tool (Claude Code, aider, etc.) through a unified interface.

```
armadai run code-reviewer "Review this PR for security issues"
armadai run --pipe code-reviewer test-writer "src/main.rs"
armadai tui
```

### Key Features

- **Markdown-based agents** — one `.md` file = one agent. Human-readable, git-friendly.
- **Multi-provider** — unified tool names (`claude`, `gemini`, `codex`, `copilot`, `opencode`, `gpt`, `aider`) auto-detect CLI vs API; explicit API/CLI/proxy modes also supported
- **Multi-pattern orchestration** — Direct (single-shot), Blackboard (parallel shared-state), Ring (sequential consensus), Hierarchical (coordinator → leads → agents)
- **Pipeline mode** — chain agents sequentially (output A becomes input B)
- **TUI & Web dashboards** — agent library management with browser, detail view, history, costs, and command palette
- **Shell completion** — auto-complete for bash, zsh, fish, powershell, elvish
- **Cost tracking** — per-agent, per-run cost monitoring stored in SQLite

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) (1.86+) — only needed to build from source

### Install

```bash
# One-liner (downloads the latest release binary)
curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/master/install.sh | bash
```

Options: `INSTALL_DIR=~/.local/bin` (default), `VERSION=v0.1.0` (default: latest).

```bash
armadai new --template basic my-assistant   # create your first agent
armadai run my-assistant "Explain how async/await works in Rust"
```

See [Getting Started](docs/wiki/getting-started.md) for building from source, configuring providers, and starter packs.

## Usage

| Command | Description | Status |
|---|---|---|
| `armadai list [--tags t] [--stack s]` | List available agents | Done |
| `armadai new --template <tpl> <name>` | Create an agent from a template | Done |
| `armadai inspect <agent>` | Show parsed agent config | Done |
| `armadai validate [path]` | Validate starter pack or project config | Done |
| `armadai run <agent> [input]` | Run an agent | Done |
| `armadai run --pipe <a> <b> [input]` | Chain agents in a pipeline | Done |
| `armadai history [--agent a]` | View execution history | Done |
| `armadai history --replay <id>` | Replay a past execution | Planned |
| `armadai costs [--agent a] [--from d]` | View cost tracking | Done |
| `armadai config providers` | Show provider configs and secrets status | Done |
| `armadai init [--force] [--project]` | Initialize ArmadAI configuration (.armadai/) | Done |
| `armadai init --pack <name>` | Install a starter pack (rust-dev, fullstack, ...) | Done |
| `armadai link --target <t> [--dry-run]` | Generate native AI assistant configs | Done |
| `armadai registry sync/search/list/add` | Browse and import community agents | Done |
| `armadai prompts list/show` | Manage composable prompts | Done |
| `armadai skills list/show/sync/search/add/info` | Manage and discover composable skills | Done |
| `armadai models check/update/list` | Check, update, and list registered models | Done |
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
- model: latest:pro
- temperature: 0.3
- max_tokens: 4096
- tags: [dev, review, quality]
- stacks: [rust, typescript, java]

## System Prompt

You are an expert code reviewer...

## Instructions

1. Understand the context of the change
2. Identify bugs, security issues, performance problems

## Output Format

Structured review: bugs, security, performance, style.
```

See the [Agent Format](docs/wiki/agent-format.md) page for the full section reference and provider configuration options.

## Starter Packs

Install curated bundles of agents, prompts and skills:

```bash
armadai init --pack rust-dev              # Rust essentials (3 agents + conventions prompt)
armadai init --pack fullstack             # Full stack web (6 agents)
armadai init --pack rust-dev --project    # Combined: install pack + create project config
```

See [Starter Packs](docs/wiki/starter-packs.md) for the full list (`rust-dev`, `fullstack`, `code-analysis-rust`, `code-analysis-web`, `armadai-authoring`, `pirate-crew`) and their contents.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed technical documentation.

```
HOST MACHINE
├── armadai (native binary)
│   ├── CLI + TUI + Web UI
│   ├── Orchestration (Direct / Blackboard / Ring / Hierarchical)
│   ├── Providers (API / CLI / Proxy)
│   ├── SQLite (embedded storage)
│   └── SOPS + age secrets
│
└── docker-compose (optional)
    └── litellm     :4000
```

### Cargo Feature Flags

Heavy dependencies are gated behind optional feature flags for faster compilation:

| Feature | Default | Description |
|---|---|---|
| `tui` | Yes | TUI dashboard (ratatui + crossterm) |
| `web` | Yes | Web UI dashboard (axum + tower-http) |
| `storage` | Yes | SQLite persistence (rusqlite bundled) |
| `providers-api` | Yes | HTTP API providers (Anthropic, OpenAI, Google) |

```bash
cargo build --release                                    # Full build (all features)
cargo build --release --no-default-features               # Lightweight build
cargo build --release --no-default-features --features storage  # CLI + storage (no TUI)
```

## Documentation

Full documentation lives in [`docs/wiki/`](docs/wiki/) (and the published site once available):

- [Getting Started](docs/wiki/getting-started.md)
- [Agent format](docs/wiki/agent-format.md)
- [Orchestration guide](docs/wiki/orchestration-guide.md)
- [Providers](docs/wiki/providers.md)
- [Skills & Prompts](docs/wiki/skills-prompts.md)
- [Migration v0 → v1](docs/wiki/migration-v0-to-v1.md)

## Development

### Setup

```bash
git clone https://github.com/Dr0drigues/armadai.git
cd armadai
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
| `master` | Default trunk — all PRs squash-merge here |
| `feature/*` | New features (branch from `master`, PR back to `master`) |
| `release/*` | Release-line stabilization (branch from `master`, merge back to `master` + tag) |
| `hotfix/*` | Emergency fixes (branch from `master`, PR back to `master`) |

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

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0/)

**Free to use for:**
- Personal use, research, experimentation, and testing
- Educational institutions and public research organizations
- Charitable organizations and government institutions
- Hobby projects and personal study

**Commercial use:**
For commercial licensing, please contact the maintainer.

See the [LICENSE](./LICENSE) file for full terms.
