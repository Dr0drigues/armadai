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

```bash
cd my-project
armadai init --project          # .armadai/project.yaml, agents/, skills/, flows/, automations/
armadai new agent dev --template dev-test
armadai new agent reviewer --template dev-review
armadai new flow review --template review-loop --agents dev,reviewer
armadai graph review            # the flow as Mermaid
armadai validate                # file:line:column diagnostics, exit 1 on error
armadai link --dry-run          # the files and the conformance report, nothing written
armadai link                    # write them (Claude Code by default)
```

Or start from a starter pack, converted into a project:

```bash
armadai init --pack rust-dev
armadai link --target claude
```

`link` accepts `claude`, `codex`, `opencode` and `copilot`. See [Link](link.md) for what each
target receives and for the conformance report, and for `unlink`, which removes exactly what
`link` wrote. Available packs: `rust-dev`, `fullstack`, `code-analysis-rust`,
`code-analysis-web`, `armadai-authoring`, `armadai-dev`, `pirate-crew` (see
[Starter Packs](starter-packs.md)).

A project in the legacy format (`.armadai/config.yaml`, `armadai.yaml`) is converted once with
`armadai migrate` (see [Project format](project-format.md#migrating-a-legacy-project)); every
other project command refuses it.

## Writing your own agent

An agent is `.armadai/agents/<id>.md`: a YAML frontmatter (`description`, optionally `tier`,
`tags`, `skills`, `capabilities`, `native`) and the prompt as the body. See
[Project format](project-format.md). `armadai new agent <id> --template <tpl>` creates one from a
template (built into the binary; files in `templates/` or `~/.config/armadai/templates/` override
them, see [Templates](templates.md)).

## Verify Setup

```bash
armadai list                 # agents, skills, flows, automations
armadai inspect reviewer     # one agent
armadai validate             # the whole project
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

```
.armadai/
├── project.yaml        # the manifest: targets, outputs, coordinator, policy, native
├── agents/<id>.md      # agents
├── skills/<id>/        # skills (SKILL.md and their files)
├── flows/<id>.yaml     # who hands work to whom
├── automations/<id>.yaml  # hooks
├── native/<target>/    # files copied as is by link
├── .gitignore          # link-manifest.yaml, .migrated/ (machine-local)
└── link-manifest.yaml  # written by `armadai link`: what it wrote, for `unlink`
```

`.armadai/` is versioned, except what `.armadai/.gitignore` lists. Full reference:
[Project format](project-format.md).

## IDE Support

The repository ships JSON Schemas for every file of a project in `schemas/v1/` (generated from
the model, so they cannot drift): `project.schema.json`, `agent.schema.json` (agent
frontmatter), `skill.schema.json`, `flow.schema.json`, `automation.schema.json`.

**VS Code** (with [YAML extension by Red Hat](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml)) and **IntelliJ** — add this comment as the first line of `.armadai/project.yaml`:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Dr0drigues/armadai/master/schemas/v1/project.schema.json
```

## Next Steps

- [Project format](project-format.md) — every file of a project, the diagnostics, migration
- [Link](link.md) — native files for Claude Code, Codex, OpenCode and Copilot, and the report
- [Policy gate](policy-gate.md) — enforcing the declared flows in Claude Code
- [Templates](templates.md) — available templates and how to create your own
- [Starter Packs](starter-packs.md) — curated agent bundles for quick setup
