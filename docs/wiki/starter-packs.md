# Starter Packs

Starter packs are curated bundles of agents and prompts that you can install with a single command. They provide ready-to-use agent configurations for common development workflows.

## Usage

```bash
armadai init --pack <pack-name>
```

This copies the pack's agents and prompts into your user library (`~/.config/armadai/agents/` and `~/.config/armadai/prompts/`).

## Available Packs

### rust-dev

Rust development essentials: code review, test writing, debugging, and Rust conventions.

```bash
armadai init --pack rust-dev
```

| Type | Name | Description |
|---|---|---|
| Agent | `code-reviewer` | Rust code review specialist |
| Agent | `test-writer` | Rust test generation |
| Agent | `debug` | Debugging assistant for Rust |
| Prompt | `rust-conventions` | Rust coding conventions and idioms |

### fullstack

Full stack web development with 6 agents covering multiple providers.

```bash
armadai init --pack fullstack
```

| Type | Name | Description |
|---|---|---|
| Agent | `code-reviewer` | General code review (Anthropic API) |
| Agent | `test-writer` | Test generation |
| Agent | `doc-generator` | Documentation writer |
| Agent | `claude-cli-reviewer` | Code review via Claude CLI |
| Agent | `gemini-reviewer` | Code review via Gemini CLI |
| Agent | `echo-reviewer` | Simple echo agent (testing) |

### pirate-crew

A fun demo pack with pirate-themed agents. Showcases the coordinator pattern and custom prompts.

```bash
armadai init --pack pirate-crew
```

| Type | Name | Description |
|---|---|---|
| Agent | `capitaine` | Pirate captain coordinator agent |
| Agent | `cartographe` | Navigation and architecture specialist |
| Agent | `vigie` | Lookout / code review specialist |
| Agent | `charpentier` | Ship builder / implementation agent |
| Prompt | `code-nautique` | Pirate-themed coding conventions |

## Pack Format

Each pack lives in `starters/<pack-name>/` with this structure:

```
starters/<pack-name>/
├── pack.yaml           # Pack manifest
├── agents/             # Agent definitions (.md files)
│   ├── agent-one.md
│   └── agent-two.md
└── prompts/            # Prompt definitions (.md files)
    └── conventions.md
```

### pack.yaml

```yaml
name: rust-dev
description: Rust development agent pack — code review, test writing, and debugging
agents:
  - code-reviewer
  - test-writer
  - debug
prompts:
  - rust-conventions
```

## Creating Custom Packs

1. Create a directory under `starters/` with your pack name
2. Add a `pack.yaml` manifest listing agents and prompts
3. Add agent `.md` files in `agents/`
4. Add prompt `.md` files in `prompts/` (optional)
5. Follow the [Agent Format](agent-format.md) for agent files

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Templates](templates.md) — creating agents from templates
- [Registry](registry.md) — community agents
