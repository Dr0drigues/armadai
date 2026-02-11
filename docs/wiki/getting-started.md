# Getting Started

## Installation

### From source

```bash
git clone https://github.com/Dr0drigues/swarm-festai.git
cd swarm-festai
cargo build --release
```

The binary is at `target/release/swarm`. Add it to your `PATH`:

```bash
# Option 1: symlink
ln -s $(pwd)/target/release/swarm ~/.local/bin/swarm

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
swarm new my-assistant --template basic --description "general-purpose coding assistant"
```

This creates `agents/my-assistant.md`. Open it and customize the system prompt to your needs.

## Verify Setup

```bash
# List all available agents
swarm list

# Validate all agent configurations
swarm validate

# Inspect a specific agent
swarm inspect my-assistant
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
swarm run my-assistant "Explain the builder pattern in Rust"
```

## Shell Completion

Set up auto-completion for your shell:

```bash
# Bash
swarm completion bash > ~/.local/share/bash-completion/completions/swarm

# Zsh
swarm completion zsh > ~/.zfunc/_swarm

# Fish
swarm completion fish > ~/.config/fish/completions/swarm.fish
```

## TUI Dashboard

Browse and manage your agent fleet visually:

```bash
swarm tui
```

Use `Tab`/`Shift+Tab` to switch views, `j`/`k` to navigate, `Enter` to view agent details, `:` to open the command palette, and `q` to quit.

## Web UI

For a browser-based dashboard:

```bash
swarm web              # http://localhost:3000
swarm web --port 8080  # custom port
```

Browse agents, view execution history, and track costs from your browser. Requires the `web` feature flag (not enabled by default):

```bash
cargo build --release --features web
```

## Next Steps

- [Agent Format](agent-format.md) — full reference for agent Markdown files
- [Providers](providers.md) — configure API, CLI, and proxy providers
- [Templates](templates.md) — available templates and how to create your own
