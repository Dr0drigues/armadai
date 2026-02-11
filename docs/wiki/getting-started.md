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

## Next Steps

- [Agent Format](agent-format.md) — full reference for agent Markdown files
- [Providers](providers.md) — configure API, CLI, and proxy providers
- [Templates](templates.md) — available templates and how to create your own
