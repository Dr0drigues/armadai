# Getting Started

## Installation

### Quick install (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash
```

This downloads the latest release binary for your platform (Linux/macOS, x86_64/aarch64) and installs it to `~/.local/bin/`.

**Options** (via environment variables):

| Variable | Default | Description |
|---|---|---|
| `INSTALL_DIR` | `~/.local/bin` | Where to install the binary |
| `VERSION` | latest | Specific version to install (e.g. `v0.1.0`) |

```bash
# Custom install directory
INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash

# Specific version
VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/Dr0drigues/armadai/develop/install.sh | bash
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
cargo install --path .
```

### Prerequisites

| Tool | Required | Purpose |
|---|---|---|
| Rust 1.86+ | Yes | Build from source |
| Docker | No | Infrastructure services (SurrealDB, LiteLLM) |
| SOPS + age | No | Encrypted secret management |

## First Agent

Create your first agent from a template:

```bash
armadai new my-assistant --template basic --description "general-purpose coding assistant"
```

This creates `agents/my-assistant.md`. Open it and customize the system prompt to your needs.

## Starter Packs

Instead of creating agents one by one, install a curated pack:

```bash
# Rust development essentials (code-reviewer, test-writer, debug + conventions prompt)
armadai init --pack rust-dev

# Full stack web development (6 agents)
armadai init --pack fullstack

# ArmadAI authoring team (4 agents + skills)
armadai init --pack armadai-authoring

# Install pack + create project config in one step
armadai init --pack rust-dev --project
```

Available packs: `rust-dev`, `fullstack`, `code-analysis-rust`, `code-analysis-web`, `armadai-authoring`, `pirate-crew`. See [Starter Packs](starter-packs.md) for details.

## Verify Setup

```bash
# List all available agents
armadai list

# Validate all agent configurations
armadai validate

# Inspect a specific agent
armadai inspect my-assistant
```

## Configure Providers

Before running agents, configure your API keys:

```bash
# Quick setup (unencrypted, gitignored)
cat > config/providers.secret.yaml << 'EOF'
providers:
  anthropic:
    api_key: sk-ant-your-key-here
EOF
```

For production use, see the [Providers](providers.md) page for SOPS + age encrypted configuration.

## Run an Agent

```bash
armadai run my-assistant "Explain the builder pattern in Rust"
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

## TUI Dashboard

Browse and manage your agent fleet visually:

```bash
armadai tui
```

Use `Tab`/`Shift+Tab` to switch views (Agents, Prompts, Skills, Starters, History, Costs), `j`/`k` to navigate, `Enter` to view details, `i` to init a project from the Starters tab, `:` to open the command palette, and `q` to quit.

## Web UI

For a browser-based dashboard:

```bash
armadai web              # http://localhost:3000
armadai web --port 8080  # custom port
```

Browse agents, prompts, skills and starters. View execution history and track costs. Skill detail views show reference file contents in collapsible sections. Starter detail pages include a "Download config.yaml" button.

## Community Registry

Browse and import agents from the community:

```bash
# Sync the registry
armadai registry sync

# Search for agents
armadai registry search "security review"

# Import an agent
armadai registry add official/security
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
├── config.yaml     # Project configuration (agents, prompts, skills, link targets)
├── agents/         # Project-local agent definitions (.md files)
├── prompts/        # Project-local prompt fragments (.md files)
├── skills/         # Project-local skills (directories with SKILL.md)
└── starters/       # Project-local starter packs
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

ArmadAI provides a JSON Schema for `armadai.yaml` that enables autocompletion and real-time validation in your editor.

**VS Code** (with [YAML extension by Red Hat](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml)) and **IntelliJ** — add this comment as the first line of your `armadai.yaml`:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Dr0drigues/armadai/develop/schemas/armadai.schema.json
```

Once added, your editor will provide:

- Autocompletion for all configuration keys (`agents`, `prompts`, `skills`, `sources`, `link`)
- Inline documentation and descriptions for each field
- Validation of required fields and allowed values (e.g. link targets: `claude`, `copilot`, `gemini`, `opencode`)
- Type checking for agent, prompt, and skill references

## Next Steps

- [Agent Format](agent-format.md) — full reference for agent Markdown files
- [Providers](providers.md) — configure API, CLI, and proxy providers
- [Templates](templates.md) — available templates and how to create your own
- [Starter Packs](starter-packs.md) — curated agent bundles for quick setup
- [Link Command](link.md) — generating native configs for AI CLI tools
- [Skills & Prompts](skills-prompts.md) — composable prompt fragments and skills
- [Registry](registry.md) — browsing and importing community agents
