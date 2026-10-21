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

ArmadAI is a declarative compiler for AI agent teams. You describe your agents (one Markdown
file each, in `.armadai/`), the skills they share, the flows between them and which agent
coordinates the others; `armadai link` then writes the native configuration files of the
assistant you use:

```
armadai link --target claude      # .claude/agents/*.md        (+ a block in the root CLAUDE.md for a coordinator)
armadai link --target codex       # .codex/agents/*.toml, .codex/config.toml, .codex/AGENTS.md
armadai link --target opencode    # .opencode/agents/*.md, opencode.json, .opencode/armadai-team.md
armadai link --target copilot     # .github/agents/*.agent.md  (+ .github/copilot-instructions.md for a coordinator)
```

The project's skills are copied alongside, under `skills/` in the same directory.

ArmadAI does not run agents: the assistant does. There is no execution engine, no TUI and no
web UI, and `link` never touches the network.

### Key Features

- **One source, four targets** — the same definitions compile to Claude Code, Codex, OpenCode
  and GitHub Copilot.
- **Markdown-based agents** — one `.md` file = one agent (YAML frontmatter, prompt as the
  body), human-readable and git-friendly.
- **Skills and flows** — skills following the `SKILL.md` standard; flows saying who hands work
  to whom, drawn as Mermaid by `armadai graph`.
- **Reversible** — `link` records what it wrote in a manifest, and `unlink` removes exactly that.
- **Policy gate** — for Claude Code, a `PreToolUse` hook that enforces the declared delegation
  topology.
- **Guided creation** — `armadai new agent|flow|automation|skill` (or a guided menu), agent
  and flow templates, native starter packs; every creation is validated, and undone if it would
  break the project.

## Quick Start

### Install

```bash
# One-liner (downloads the latest release binary)
curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/master/install.sh | bash
```

Options: `INSTALL_DIR=~/.local/bin` (default), `VERSION=v1.x.y` (default: latest).
The installer checks the binary against the release's `SHA256SUMS` and refuses a release that
publishes none. The latest published release (`v1.0.0-rc.5`) predates both the declarative
refocus and those checksums: until the next one, [build from source](docs/wiki/getting-started.md#from-source).

```bash
armadai init --pack rust-dev                # a project (.armadai/) from a starter pack
armadai new agent planner --template planning
armadai new flow ship --agents planner,code-reviewer,test-writer
armadai graph ship                          # the flow as Mermaid
armadai link --target claude --dry-run      # preview the files and the conformance report
armadai link --target claude                # write them
```

See [Getting Started](docs/wiki/getting-started.md) for building from source, the project
layout and starter packs.

## Usage

Running `armadai` without a subcommand prints the help.

| Command | Description |
|---|---|
| `armadai link [--target t…] [--dry-run] [--json]` | Compile the project into native files, with a conformance report |
| `armadai unlink [--target t…] [--dry-run]` | Remove what `link` wrote, and only that |
| `armadai list [--tags t]` | List the project's agents, skills, flows, automations |
| `armadai inspect <agent>` | Show one agent of the project |
| `armadai validate [path]` | Validate the project (or a starter pack) |
| `armadai migrate` | Convert a legacy project to `.armadai/project.yaml` |
| `armadai new agent\|flow\|automation\|skill <id>` | Create an element of the project (agent templates, flow templates) |
| `armadai graph [flow]` | Print a flow as Mermaid |
| `armadai init [--project] [--pack <name>]` | Initialize the user configuration, or a project (empty or from a starter pack) |
| `armadai skills list/show/sync/search/add/info` | Manage and discover skills |
| `armadai models check/update/list` | Check and rewrite deprecated model ids, list linked projects |
| `armadai config starters-dir list/add/remove` | Custom starter pack directories |
| `armadai update` | Self-update to the latest release |
| `armadai completion <shell>` | Generate shell completions |

## Project Format

A project is a `.armadai/` folder: `project.yaml` (targets, coordinator, policy), one Markdown
file per agent with a YAML frontmatter, skills, flows and automations.

```markdown
---
description: Reviews code for bugs, security and style
tier: pro
tags: [dev, review]
capabilities:
  shell: deny
---
You are an expert code reviewer...
```

See [Project format](docs/wiki/project-format.md) for every file and diagnostic, and
[Link](docs/wiki/link.md) for what each target receives.

## Starter Packs

Start a project from a curated bundle of agents and skills:

```bash
armadai init --pack rust-dev              # Rust essentials (3 agents + conventions skill)
armadai init --pack fullstack             # tests, review and docs chained by a review loop (3 agents)
armadai init --pack ./my-pack             # your own pack (a folder with pack.yaml, format: 1)
```

See [Starter Packs](docs/wiki/starter-packs.md) for the full list and their contents.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the technical documentation.

```
.armadai/ (project.yaml, agents, skills, flows, automations, native)
        │
        ▼
armadai (native binary): load → validate → one plan per target → conformance report
        │
        ▼
native files for Claude Code · Codex · OpenCode · Copilot  (+ .armadai/link-manifest.yaml)
```

What the declarative refocus removed, and what comes next, is specified in
[the step 0 spec](docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md).

## Documentation

Full documentation lives in [`docs/wiki/`](docs/wiki/):

- [Getting Started](docs/wiki/getting-started.md)
- [Project format](docs/wiki/project-format.md)
- [Models](docs/wiki/models.md)
- [Link](docs/wiki/link.md)
- [Policy gate](docs/wiki/policy-gate.md)
- [Templates](docs/wiki/templates.md), [Starter packs](docs/wiki/starter-packs.md)
- Legacy format, read only by `migrate`: [Agent format](docs/wiki/agent-format.md),
  [Declarative agents](docs/wiki/declarative-agents.md),
  [Providers and models](docs/wiki/providers.md), [Skills & Prompts](docs/wiki/skills-prompts.md)

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

`CHANGELOG.md` is written by hand (its `Unreleased` section), and the GitHub release notes are taken
from it: before pushing a tag `vX.Y.Z`, rename `## Unreleased` to `## vX.Y.Z (YYYY-MM-DD)` and commit.
`release.yml` publishes that section (`.github/scripts/release-notes.sh`) and fails when it is missing
or empty; a manual dry run previews the `Unreleased` section instead.

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
