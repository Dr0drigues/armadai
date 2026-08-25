# Link Command

The `armadai link` command generates native configuration files for your preferred AI CLI tool from your ArmadAI agent definitions.

## Usage

```bash
armadai link <target>           # Generate config for a specific CLI
armadai link <target> --dry-run # Preview: writes no files and no manifest
armadai link <target> --force   # Overwrite existing files

armadai unlink <target>           # Remove what link wrote, and only that
armadai unlink <target> --dry-run # Preview the removals, same guards, same exit code
```

## Supported Targets

| CLI | Target | Generated Files |
|---|---|---|
| Claude Code | `claude` | `CLAUDE.md` + `.claude/commands/*.md` |
| GitHub Copilot | `copilot` | `.github/copilot-instructions.md` + `.github/agents/*.agent.md` |
| Gemini CLI | `gemini` | `GEMINI.md` |
| Codex | `codex` | `.codex/AGENTS.md` + `.codex/config.toml` + `.codex/agents/*.toml` |
| opencode | `opencode` | `.opencode/instructions.md` + `.opencode/agents/*.md` |

More targets (Cursor, Aider, Windsurf, Cline) may be added later.

## How It Works

1. **Load** — Reads `armadai.yaml` from the project root
2. **Resolve** — Resolves agent references (user library, registry, local paths)
3. **Transform** — Converts ArmadAI agents to the target CLI's native format
4. **Write** — Generates files in the appropriate directories

## Examples

### Claude Code

```bash
armadai link claude
```

Generates:
- `CLAUDE.md` — Project instructions with all agent system prompts and conventions
- `.claude/commands/<agent>.md` — One slash command per agent

### GitHub Copilot

```bash
armadai link copilot
```

Generates:
- `.github/copilot-instructions.md` — Global instructions
- `.github/agents/<agent>.agent.md` — One agent file per agent (with YAML frontmatter)

### Gemini CLI

```bash
armadai link gemini
```

Generates:
- `GEMINI.md` — Project instructions for Gemini CLI

### Codex

```bash
armadai link codex
```

Generates:
- `.codex/AGENTS.md` — Project instructions listing available agents
- `.codex/config.toml` — Agent registry config
- `.codex/agents/<agent>.toml` — One agent file per agent

### opencode

```bash
armadai link opencode
```

Generates:
- `.opencode/instructions.md` — Project instructions listing available agents (only generated when the project has a coordinator agent)
- `.opencode/agents/<agent>.md` — One agent file per agent

## Conflict Detection

By default, `armadai link` warns before overwriting existing files. Use `--force` to skip confirmation, or `--dry-run` to preview first. A pre-existing file whose content differs from what would be generated is left untouched and recorded as `skipped` in the link manifest (below), which is what keeps `armadai unlink` from ever removing it.

## The Link Manifest

`armadai link` records what it wrote in `.armadai/link-manifest.yaml`, grouped by target: the target's output root, the directories `link` itself created, and one entry per file with its `produced_by`, its `outcome` (`created` or `skipped`), and — for `created` — the `sha256:` digest of the bytes written.

`armadai unlink` reads that manifest and applies the inverse of each recorded outcome, so it removes exactly what `link` produced and nothing else — including files whose agent has since been removed from the project config, which is the case no amount of regeneration can recover.

The manifest is data on disk, so `unlink` treats it as untrusted input: it re-derives the target's output directory from the project config and refuses the manifest wholesale if the two disagree, and it refuses any individual entry or recorded directory that resolves outside the target's own root, onto that root itself, or that corresponds to no file `link` recorded creating.

`--dry-run` writes no manifest. When no usable manifest exists — a fresh clone, a deleted `.armadai/`, or a project linked before the manifest existed — `unlink` says so and falls back to a content-match guard: it regenerates what `link` would write today and removes a file only when its bytes still match byte for byte. In that degraded mode a file linked with different options (an explicit `--model`, or an interactive prompt answer) is kept rather than reclaimed; re-run `link` to write a manifest.

## Unlink Output

Each file `unlink` considers is reported as one of:

| Outcome | Meaning |
|---|---|
| `deleted` | recorded as `created` and still byte-for-byte what `link` wrote |
| `kept (hand-written — link recorded it as skipped)` | `link` found it already there and left it alone |
| `kept (content differs from what link wrote)` | edited since linking; removing it would be data loss. Without a manifest the fallback says only `kept (content differs)` — it compares against freshly generated content, not a recorded digest, so it cannot claim the file ever *was* what `link` wrote |
| `kept (cannot verify …)` | unreadable, or a digest algorithm this build doesn't recognise |
| `kept (broken symlink …)` | the link is on disk but its target is gone, so its content can't be checked. It is **not** "already absent" |
| `already absent` | nothing at that path at all |

Refusals — a manifest item outside its trusted root, one naming the target's own root, or one matching no recorded file — go to **stderr** in both the real pass and `--dry-run`, beside the summary line that points at them, while the per-file outcomes and the closing `Unlinked '<target>': …` counts stay on stdout. Each refusal names *which side* is at fault: the manifest's own text ("the manifest may be corrupt or forged"), or the filesystem ("something on the filesystem does, most likely a symlink along the path") under a manifest that is intact. The two call for different follow-up. Note what `unlink` deliberately does *not* claim: **when** a filesystem cause appeared. It compares the recorded text against the filesystem as it is now, and keeps no record of how the filesystem looked when `link` ran, so it can say which side puts a path where it is — never that anything changed since.

**A refused item makes `unlink` exit non-zero**, and `--dry-run` mirrors that: it applies the same guards, prints the same refusals, and exits non-zero wherever a real run would — a preview that cannot fail is not a preview. Only one thing it deliberately does not mirror: whether a recorded directory is still empty on disk. The real pass checks that *after* deleting the files inside it, so the preview reports directories as "eligible for cleanup" — the number that passed its guards — rather than "recorded" (which would deny a record it holds for a directory it refuses) or "will be removed".

## Agent-to-Format Mapping

| ArmadAI Field | Claude Code | Copilot | Gemini |
|---|---|---|---|
| Agent name (H1) | Command name | `name:` frontmatter | Section heading |
| System Prompt | Markdown body | Markdown body | Markdown body |
| Instructions | Appended to body | Appended to body | Appended to body |
| Tags | Free text | `description:` | Free text |
| Model | N/A | `model:` frontmatter | N/A |

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Skills & Prompts](skills-prompts.md) — composable prompts and skills
