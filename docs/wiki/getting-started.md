# Getting Started

Welcome to ArmadAI — follow the steps below to get up and running.

## Installation

### Quick install (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/master/install.sh | bash
```

This downloads the latest release binary for your platform (Linux/macOS, x86_64/aarch64) and installs it to `~/.local/bin/`.

**Options** (via environment variables):

| Variable | Default | Description |
|---|---|---|
| `INSTALL_DIR` | `~/.local/bin` | Where to install the binary |
| `VERSION` | latest | Specific version to install (e.g. `v0.1.0`) |

```bash
# Custom install directory
INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/master/install.sh | bash

# Specific version
VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/master/install.sh | bash
```

### From source

```bash
git clone https://github.com/Dr0drigues/armadai.git
cd armadai
cargo build --release
```

The binary is at `target/release/armadai`. Add it to your `PATH`:

```bash
# Option 1: symlink
ln -s $(pwd)/target/release/armadai ~/.local/bin/armadai

# Option 2: cargo install
cargo install --path crates/armadai
```

### Prerequisites

| Tool | Required | Purpose |
|---|---|---|
| Rust (recent stable) | To build from source | `.mise.toml` pins the toolchain the project uses |
| SOPS + age | No | Encrypted secret management (`armadai config secrets`) |

Running `armadai` without a subcommand prints the help.

## A first project

The shortest path to generated files is a starter pack:

```bash
cd my-project
armadai init --pack rust-dev --project     # install the pack, declare its agents in .armadai/config.yaml
armadai link --target claude --dry-run     # preview what would be written
armadai link --target claude               # write .claude/agents/*.md
```

`link` accepts `claude`, `codex`, `opencode` and `copilot`. See [Link](link.md) for what each
target receives, and for `unlink`, which removes exactly what `link` wrote.

> `init --pack … --project` currently writes `link.target: anthropic` into the generated config,
> which is not a link target: pass `--target` explicitly, or set `link.target` to one of the four
> targets above.

Available packs: `rust-dev`, `fullstack`, `code-analysis-rust`, `code-analysis-web`,
`armadai-authoring`, `armadai-dev`, `orchestration-demo`, `pirate-crew`. See
[Starter Packs](starter-packs.md) for details.

## Writing your own agent

An agent is a Markdown file (see [Agent Format](agent-format.md)). Put it in `.armadai/agents/`
or `agents/`, then reference it from `.armadai/config.yaml`:

```yaml
agents:
  - name: my-assistant                      # from ~/.config/armadai/agents/
  - path: .armadai/agents/my-assistant.md   # or a project-local file
```

`armadai new <name> --template <tpl>` creates one from a template, in `.armadai/agents/` or
`agents/` when the project has one, otherwise in your user library. It reads templates from
`templates/` in the current directory or from `~/.config/armadai/templates/`, which nothing
installs for you (see [Templates](templates.md)). A project can also declare agents in `.armadai/agents.yaml` — see
[Declarative agents](declarative-agents.md).

## Verify Setup

```bash
# List all available agents
armadai list

# Validate the project config (or a starter pack directory)
armadai validate

# Inspect a specific agent
armadai inspect my-assistant
```

## Shell Completion

Set up auto-completion for your shell:

```bash
# Bash
armadai completion bash > ~/.local/share/bash-completion/completions/armadai

# Zsh
armadai completion zsh > ~/.zfunc/_armadai

# Fish
armadai completion fish > ~/.config/fish/completions/armadai.fish
```

## Discover Skills

Browse and install skills from GitHub repos:

```bash
# Sync remote skill sources
armadai skills sync

# Search for skills
armadai skills search "testing"

# Install a skill
armadai skills add anthropics/skills/webapp-testing

# List installed skills
armadai skills list
```

## Project Structure

ArmadAI uses a `.armadai/` directory as the canonical project configuration location:

```
.armadai/
├── config.yaml         # Project configuration (agents, prompts, skills, link settings)
├── agents/             # Project-local agent definitions (.md files)
├── prompts/            # Project-local prompt fragments (.md files)
├── skills/             # Project-local skills (directories with SKILL.md)
├── starters/           # Project-local starter packs
└── link-manifest.yaml  # Written by `armadai link`: what it created, for `unlink`
```

Create this structure with:

```bash
armadai init --project
```

### Legacy format

The older `armadai.yaml` at the repository root is still supported for backwards compatibility. When ArmadAI detects `armadai.yaml` without a `.armadai/` directory, it prints a migration hint:

```
hint: armadai.yaml detected in /path/to/project. Consider migrating to
.armadai/config.yaml. Run `armadai init --project` to create the new structure.
```

### Resolution order

Resources (agents, prompts, skills) are resolved in this order:

1. `.armadai/{type}/` — project-local (preferred)
2. `{type}/` — legacy root-level directories
3. `~/.config/armadai/{type}/` — user library

## IDE Support

The repository ships a JSON Schema for `armadai.yaml` (`schemas/armadai.schema.json`) that gives autocompletion and validation in editors that support it.

**VS Code** (with [YAML extension by Red Hat](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml)) and **IntelliJ** — add this comment as the first line of your `armadai.yaml`:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Dr0drigues/armadai/master/schemas/armadai.schema.json
```

This schema predates the declarative refocus: it still offers fields of the removed execution
engine and lists `gemini` among the link targets, which `armadai link` no longer accepts. A
versioned schema with a validator is planned (see the
[step 0 spec](https://github.com/Dr0drigues/armadai/blob/master/docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md)); it does not
exist yet.

## Next Steps

- [Agent Format](agent-format.md) — full reference for agent Markdown files
- [Link](link.md) — generating native configs for Claude Code, Codex, OpenCode and Copilot
- [Providers and models](providers.md) — the `provider` and `model` fields as `link` uses them
- [Templates](templates.md) — available templates and how to create your own
- [Starter Packs](starter-packs.md) — curated agent bundles for quick setup
- [Skills & Prompts](skills-prompts.md) — composable prompt fragments and skills
