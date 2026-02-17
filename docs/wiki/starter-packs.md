# Starter Packs

Starter packs are curated bundles of agents, prompts and skills that you can install with a single command. They provide ready-to-use agent configurations for common development workflows.

## Usage

```bash
# Install a pack (agents, prompts, skills copied to user library)
armadai init --pack <pack-name>

# Install a pack + create armadai.yaml project config
armadai init --pack <pack-name> --project
```

This copies the pack's agents, prompts and skills into your user library (`~/.config/armadai/agents/`, `~/.config/armadai/prompts/`, `~/.config/armadai/skills/`).

### Init from TUI / Web UI

You can also init a project from the UI:

- **TUI**: navigate to the Starters tab (or Starter Detail), press `i` to write `armadai.yaml` to the current working directory
- **Web UI**: click the "Download armadai.yaml" button on any starter detail page

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

### code-analysis-rust

Rust-focused code analysis team with a lead analyst coordinator.

```bash
armadai init --pack code-analysis-rust
```

| Type | Name | Description |
|---|---|---|
| Agent | `lead-analyst` | Coordinator: dispatches analysis tasks |
| Agent | `rust-reviewer` | Rust code review specialist |
| Agent | `rust-test-analyzer` | Test quality analysis |
| Agent | `rust-doc-writer` | Documentation review |
| Agent | `rust-security` | Security audit specialist |
| Prompt | `analysis-standards` | Code analysis standards and conventions |

### code-analysis-web

Web-focused code analysis team with a lead analyst coordinator.

```bash
armadai init --pack code-analysis-web
```

| Type | Name | Description |
|---|---|---|
| Agent | `lead-analyst` | Coordinator: dispatches analysis tasks |
| Agent | `web-reviewer` | Web code review specialist |
| Agent | `web-test-analyzer` | Test quality analysis |
| Agent | `web-doc-writer` | Documentation review |
| Agent | `web-security` | Security audit specialist |
| Prompt | `analysis-standards` | Code analysis standards and conventions |

### armadai-authoring

ArmadAI content authoring team — create agents, prompts and skills.

```bash
armadai init --pack armadai-authoring
```

| Type | Name | Description |
|---|---|---|
| Agent | `authoring-lead` | Coordinator: manages authoring tasks |
| Agent | `agent-builder` | Creates new agents |
| Agent | `prompt-builder` | Creates composable prompts |
| Agent | `skill-builder` | Creates skills (SKILL.md) |
| Prompt | `armadai-conventions` | ArmadAI authoring conventions |
| Skill | `armadai-agent-authoring` | Built-in skill for agent authoring |
| Skill | `armadai-prompt-authoring` | Built-in skill for prompt authoring |
| Skill | `armadai-skill-authoring` | Built-in skill for skill authoring |

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
├── prompts/            # Prompt definitions (.md files)
│   └── conventions.md
└── skills/             # Skill directories (optional)
    └── my-skill/
        ├── SKILL.md
        └── references/
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
skills:
  - armadai-agent-authoring   # Refers to built-in or bundled skill
```

Skills listed in `pack.yaml` but not bundled in the pack directory (e.g. built-in skills already installed by `armadai init`) are silently skipped during installation.

## Embedded Versioning

Starter packs embedded in the binary are extracted to `~/.config/armadai/starters/` on first use. Each extracted pack contains a `.armadai-version` marker file tracking the binary version at extraction time.

When the binary is updated, packs are automatically re-extracted to ensure they stay in sync. The same mechanism applies to built-in skills.

## Creating Custom Packs

1. Create a directory under `starters/` with your pack name
2. Add a `pack.yaml` manifest listing agents, prompts and skills
3. Add agent `.md` files in `agents/`
4. Add prompt `.md` files in `prompts/` (optional)
5. Add skill directories in `skills/` (optional)
6. Follow the [Agent Format](agent-format.md) for agent files

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Templates](templates.md) — creating agents from templates
- [Registry](registry.md) — community agents
