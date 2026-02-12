# Getting Started

## Installation

### From source

```bash
git clone https://github.com/Dr0drigues/swarm-festai.git
cd swarm-festai
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
```

List available packs with `armadai init --pack nonexistent` (shows available options on error).

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

Use `Tab`/`Shift+Tab` to switch views, `j`/`k` to navigate, `Enter` to view agent details, `:` to open the command palette, and `q` to quit.

## Web UI

For a browser-based dashboard:

```bash
armadai web              # http://localhost:3000
armadai web --port 8080  # custom port
```

Browse agents, view execution history, and track costs from your browser. The `web` feature is enabled by default.

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

## Next Steps

- [Agent Format](agent-format.md) — full reference for agent Markdown files
- [Providers](providers.md) — configure API, CLI, and proxy providers
- [Templates](templates.md) — available templates and how to create your own
- [Starter Packs](starter-packs.md) — curated agent bundles for quick setup
- [Registry](registry.md) — browsing and importing community agents
