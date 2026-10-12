<p align="center">
  <img src="assets/brand/armadai-wordmark.svg" alt="ArmadAI" width="300">
</p>

<p align="center">
  Declarative agent teams — describe them once, compile them for Claude Code, Codex, OpenCode and Copilot.
</p>

<p align="center">
  <a href="https://github.com/Dr0drigues/armadai/actions/workflows/ci.yml"><img src="https://github.com/Dr0drigues/armadai/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/Dr0drigues/armadai/actions/workflows/audit.yml"><img src="https://github.com/Dr0drigues/armadai/actions/workflows/audit.yml/badge.svg" alt="Security Audit"></a>
</p>

## Overview

ArmadAI is a declarative compiler for AI agent teams. You describe your agents (Markdown files
or a YAML declaration), the prompts and skills they share, and which agent coordinates the
others; `armadai link` then writes the native configuration files of the assistant you use:

```
armadai link --target claude      # .claude/agents/*.md        (+ .claude/CLAUDE.md for a coordinator)
armadai link --target codex       # .codex/agents/*.toml, .codex/config.toml, .codex/AGENTS.md
armadai link --target opencode    # .opencode/agents/*.md      (+ .opencode/instructions.md for a coordinator)
armadai link --target copilot     # .github/agents/*.agent.md  (+ .github/copilot-instructions.md for a coordinator)
```

The project's prompts and skills are copied alongside, under `prompts/` and `skills/` in the same
directory.

ArmadAI does not run agents: the assistant does. There is no execution engine, no TUI and no
web UI, and `link` never touches the network.

### Key Features

- **One source, four targets** — the same definitions compile to Claude Code, Codex, OpenCode
  and GitHub Copilot.
- **Markdown-based agents** — one `.md` file = one agent, human-readable and git-friendly; or
  declare them in `.armadai/agents.yaml`.
- **Composable prompts and skills** — reusable prompt fragments, and skills following the
  `SKILL.md` standard.
- **Reversible** — `link` records what it wrote in a manifest, and `unlink` removes exactly that.
- **Policy gate** — for Claude Code, a `PreToolUse` hook that enforces the declared delegation
  topology.
- **Starter packs and templates** — curated bundles and agent templates to start from.

## Quick Start

### Install

```bash
# One-liner (downloads the latest release binary)
curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/master/install.sh | bash
```

Options: `INSTALL_DIR=~/.local/bin` (default), `VERSION=v0.1.0` (default: latest).

```bash
armadai init --pack rust-dev --project      # install a starter pack and declare its agents in .armadai/config.yaml
armadai link --target claude --dry-run      # preview the generated files
armadai link --target claude                # write them
```

See [Getting Started](docs/wiki/getting-started.md) for building from source, the project
layout and starter packs.

## Usage

Running `armadai` without a subcommand prints the help.

| Command | Description |
|---|---|
| `armadai link --target <t> [--dry-run]` | Generate native config files for an assistant |
| `armadai unlink --target <t> [--dry-run]` | Remove what `link` wrote, and only that |
| `armadai list [--tags t] [--stack s]` | List available agents |
| `armadai inspect <agent>` | Show an agent's parsed configuration |
| `armadai validate [path]` | Validate a starter pack or project config |
| `armadai new --template <tpl> <name>` | Create an agent from a template (`-i` for a wizard) |
| `armadai init [--project] [--pack <name>]` | Initialize the user or project configuration, install a starter pack |
| `armadai prompts list/show` | Manage composable prompts |
| `armadai skills list/show/sync/search/add/info` | Manage and discover skills |
| `armadai extract` | Extract agents, prompts and skills with their dependencies |
| `armadai models check/update/list` | Check and rewrite deprecated model ids, list linked projects |
| `armadai config providers/secrets/starters-dir` | Provider status, SOPS + age secrets, custom starter directories |
| `armadai update` | Self-update to the latest release |
| `armadai completion <shell>` | Generate shell completions |

## Agent Format

Each agent is a Markdown file in `agents/` (or `.armadai/agents/`):

```markdown
# Code Reviewer

## Metadata
- provider: claude
- model: latest:pro
- temperature: 0.3
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

See the [Agent Format](docs/wiki/agent-format.md) page for the full section reference.

## Starter Packs

Install curated bundles of agents, prompts and skills:

```bash
armadai init --pack rust-dev              # Rust essentials (3 agents + conventions prompt)
armadai init --pack fullstack             # General-purpose pack (6 agents)
armadai init --pack rust-dev --project    # Install the pack and create the project config
```

See [Starter Packs](docs/wiki/starter-packs.md) for the full list and their contents.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the technical documentation.

```
agent definitions (.md / agents.yaml) + prompts + skills + project config
        │
        ▼
armadai (native binary): parse → validate → one linker per target
        │
        ▼
native files for Claude Code · Codex · OpenCode · Copilot  (+ .armadai/link-manifest.yaml)
```

What the declarative refocus removed, and what comes next, is specified in
[the step 0 spec](docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md).

## Documentation

Full documentation lives in [`docs/wiki/`](docs/wiki/):

- [Getting Started](docs/wiki/getting-started.md)
- [Agent format](docs/wiki/agent-format.md)
- [Declarative agents](docs/wiki/declarative-agents.md)
- [Link](docs/wiki/link.md)
- [Providers and models](docs/wiki/providers.md)
- [Skills & Prompts](docs/wiki/skills-prompts.md)
- [Policy gate](docs/wiki/policy-gate.md)

Documents describing the former execution engine are kept, unmaintained, in
[`docs/archive/`](docs/archive/README.md).

## Development

### Setup

```bash
git clone https://github.com/Dr0drigues/armadai.git
cd armadai
git config core.hooksPath .githooks    # Enable commit message validation
```

Building from source needs a recent stable Rust toolchain; `.mise.toml` pins the one the
project uses (`mise install`).

### Build & Test

There is a single configuration (no optional cargo features), identical to CI:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
cargo build -p armadai-core   # the core builds standalone
RUST_LOG=debug cargo run -- list
```

`mise run ci` runs the same gate.

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
refactor(linker): share the prompt composition
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
