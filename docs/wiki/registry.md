# Registry

ArmadAI integrates with [awesome-copilot](https://github.com/github/awesome-copilot) as a community registry of pre-made agents, prompts, and skills.

## Commands

### Sync the registry

```bash
armadai registry sync
```

Clones or updates the awesome-copilot repository to your local cache at `~/.config/armadai/registry/awesome-copilot/`.

### Search the catalog

```bash
armadai registry search "test"
armadai registry search "security" --type agents
```

Searches agents, prompts, and skills by name, description, or tags.

### List available items

```bash
armadai registry list
armadai registry list --type agents
armadai registry list --type prompts
```

### Show details

```bash
armadai registry info <name>
```

### Add to your library

```bash
armadai registry add <name>
```

Copies the agent to `~/.config/armadai/agents/` in ArmadAI format. The awesome-copilot format is automatically converted.

## Format Conversion

The registry converts awesome-copilot `.agent.md` files to ArmadAI format:

| awesome-copilot | ArmadAI |
|---|---|
| `name:` frontmatter | `# Name` (H1 heading) |
| `description:` frontmatter | Tags/description in `## Metadata` |
| `model:` frontmatter | `- model:` in `## Metadata` |
| `tools:` frontmatter | Informational (not mapped) |
| Markdown body | `## System Prompt` + `## Instructions` |

## Using Registry Agents in Projects

Reference registry agents in `armadai.yaml`:

```yaml
agents:
  - registry:principal-engineer
  - registry:test-specialist
  - code-reviewer              # From user library
  - path: ./agents/custom.md   # Project-local
```

## Cache Location

```
~/.config/armadai/registry/
└── awesome-copilot/
    ├── agents/
    ├── prompts/
    ├── instructions/
    ├── skills/
    └── last-sync.json
```

## Offline Usage

The registry works offline after the initial sync. Run `armadai registry sync` when you want to fetch updates.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Starter Packs](starter-packs.md) — curated agent bundles
- [Link Command](link.md) — generating config for AI CLIs
