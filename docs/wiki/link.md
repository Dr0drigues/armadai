# Link Command

The `armadai link` command compiles your ArmadAI agent definitions into the native configuration
files of an AI assistant. It never touches the network.

## Usage

```bash
armadai link --target <target>            # Generate config for one assistant
armadai link --target <target> --dry-run  # Preview: writes no files and no manifest
armadai link --target <target> --force    # Overwrite existing files

armadai unlink --target <target>            # Remove what link wrote, and only that
armadai unlink --target <target> --dry-run  # Preview the removals, same guards, same exit code
```

Without `--target`, `link` uses `link.target` from the project config. Other options:
`--agents <names…>` (link only some agents), `--coordinator <name>` (see below), `--model <id>`
(see [Providers and models](providers.md)), `--output <dir>` (override the output directory).

## Supported Targets

| Assistant | Target | Generated Files |
|---|---|---|
| Claude Code | `claude` | `.claude/agents/<agent>.md` (+ `.claude/CLAUDE.md` for the coordinator) |
| Codex | `codex` | `.codex/agents/<agent>.toml` + `.codex/config.toml` + `.codex/AGENTS.md` |
| OpenCode | `opencode` | `.opencode/agents/<agent>.md` (+ `.opencode/instructions.md` for the coordinator) |
| GitHub Copilot | `copilot` | `.github/agents/<agent>.agent.md` (+ `.github/copilot-instructions.md` for the coordinator) |

In each target's directory, `link` also copies the prompts and skills the project lists, under
`prompts/` and `skills/` (see [Skills & Prompts](skills-prompts.md#how-prompts-and-skills-are-linked)).

The exact output of the four targets on a reference project is pinned by a golden test
(`crates/armadai/tests/link_golden.rs`, outputs in `crates/armadai/tests/golden/link/`).

## How It Works

1. **Load** — reads the project config (`.armadai/config.yaml`, or the legacy `armadai.yaml`)
2. **Resolve** — resolves the agents it lists (project-local files, user library, and
   `.armadai/agents.yaml` declarations), plus its prompts and skills
3. **Transform** — converts each agent to the target's native format, resolving its model
4. **Write** — writes the files and records them in `.armadai/link-manifest.yaml`

A project linked this way is also registered for `armadai models check --all`.

### Agent-to-Format Mapping

| ArmadAI field | Claude Code | Codex | OpenCode | Copilot |
|---|---|---|---|---|
| Agent name (H1) | file name + `name:` | file name + `[agents.<name>]` in `config.toml` | file name | file name |
| First line of System Prompt | `description:` | `description` in `config.toml` | `description:` | first line of the file |
| Composed prompt (System Prompt, Instructions, Output Format, Context) | Markdown body | `developer_instructions` | Markdown body | Markdown body |
| Model | `model:` | `model` | `model:` | — |
| Temperature | — | — | `temperature:` (when not `0.7`) | — |

### The Coordinator

`link.coordinator` in the project config (or `--coordinator`) names one agent of the roster: the
one whose prompt becomes the target's root instructions file (`.claude/CLAUDE.md`,
`.opencode/instructions.md`, …) instead of a per-agent file, followed by a team roster. It is
matched against each agent's **H1 title** — the `# Name` heading of its Markdown file — or that
title's slug, case-insensitively. So `coordinator: dev-lead` resolves an agent titled `Dev Lead`,
the same folding the linker uses to name files on disk.

`link.coordinator` is matched against the **title**, *not* against the key used in `agents:` or
in `orchestration.coordinator`, which are a separate namespace. When the two differ — a roster
entry `- name: lead` on a file titled `# Dev Lead` — `coordinator: lead` matches nothing; that is
reported (see below), and the field to correct is the title, not the key.

`unlink` applies that identical rule whenever it has to regenerate (the no-manifest fallback
below). Through the manifest the question never arises: `link`'s own attribution is what `unlink`
reads back.

A `link.coordinator` matching no agent is not an error — the target simply gets no root
instructions file, and `unlink` removes none. It is reported, though, by `link`, `unlink` and
`armadai validate`:

```text
  warn: link.coordinator 'dev-led' matches no agent — no root instructions file (.claude/CLAUDE.md and its equivalents) is written or removed for it.
        It is matched against an agent's H1 title, or that title's slug — not the `agents:` key, which is a separate namespace.
        Titles in this roster: Worker, Dev Lead.
```

The question asked is always *"does this reference name an agent the project declares?"* — never
*"did this particular command find it?"*. `--agents` narrows what gets written, so
`armadai link --agents Worker` writes no root instructions file for a coordinator it was not asked
for; but the configuration is untouched by your filter, so nothing is reported.

`armadai validate` answers the same question, on the same criterion — with one restriction: if
any agent of the roster failed to load, the check stands down instead of guessing, says so, and
lists what stopped it:

```text
WARN  armadai.yaml:link.coordinator: link.coordinator 'ghost' was not checked: part of the roster failed to load, so the titles available here are not the titles `link` would see and "matches no agent" would be a guess.
  Resolve these first, then re-run `armadai validate`:
  - Agent 'ghost' not found in …
```

## Conflict Detection

By default, `armadai link` warns before overwriting existing files. Use `--force` to skip
confirmation, or `--dry-run` to preview first. A pre-existing file whose content differs from what
would be generated is left untouched and recorded as `skipped` in the link manifest (below), which
is what keeps `armadai unlink` from ever removing it.

## The Link Manifest

`armadai link` records what it wrote in `.armadai/link-manifest.yaml`, grouped by target: the
target's output root, the directories `link` itself created, and one entry per file with its
`produced_by`, its `outcome` (`created` or `skipped`), and — for `created` — the `sha256:` digest
of the bytes written.

`armadai unlink` reads that manifest and applies the inverse of each recorded outcome, so it
removes exactly what `link` produced and nothing else — including files whose agent has since been
removed from the project config, which is the case no amount of regeneration can recover.

The manifest is data on disk, so `unlink` treats it as untrusted input: it re-derives the target's
output directory from the project config and refuses the manifest wholesale if the two disagree,
and it refuses any individual entry or recorded directory that resolves outside the target's own
root, onto that root itself, or that corresponds to no file `link` recorded creating.

`--dry-run` writes no manifest. When no usable manifest exists — a fresh clone, a deleted
`.armadai/`, or a project linked before the manifest existed — `unlink` says so and falls back to a
content-match guard: it regenerates what `link` would write today and removes a file only when its
bytes still match byte for byte. In that degraded mode a file linked with different options (an
explicit `--model`, or an interactive answer) is kept rather than reclaimed; re-run `link` to write
a manifest.

## Unlink Output

Each file `unlink` considers is reported as one of:

| Outcome | Meaning |
|---|---|
| `deleted` | recorded as `created` and still byte-for-byte what `link` wrote |
| `kept (hand-written — link recorded it as skipped)` | `link` found it already there and left it alone |
| `kept (content differs from what link wrote)` | edited since linking; removing it would be data loss. Without a manifest the fallback says only `kept (content differs)` — it compares against freshly generated content, not a recorded digest |
| `kept (cannot verify …)` | unreadable, or a digest algorithm this build doesn't recognise |
| `kept (broken symlink …)` | the link is on disk but its target is gone, so its content can't be checked. It is **not** "already absent" |
| `already absent` | nothing at that path at all |

Refusals — a manifest item outside its trusted root, one naming the target's own root, or one
matching no recorded file — go to **stderr** in both the real pass and `--dry-run`, beside the
summary line that points at them, while the per-file outcomes and the closing
`Unlinked '<target>': …` counts stay on stdout. Each refusal names *which side* is at fault: the
manifest's own text ("the manifest may be corrupt or forged"), or the filesystem ("something on
the filesystem does, most likely a symlink along the path") under a manifest that is intact.
`unlink` keeps no record of how the filesystem looked when `link` ran, so it can say which side
puts a path where it is — never that anything changed since.

**A refused item makes `unlink` exit non-zero**, and `--dry-run` mirrors that: it applies the same
guards, prints the same refusals, and exits non-zero wherever a real run would. Only one thing it
deliberately does not mirror: whether a recorded directory is still empty on disk. The real pass
checks that *after* deleting the files inside it, so the preview reports directories as "eligible
for cleanup" rather than "will be removed".

`unlink --with-config` also removes the project config file.

## A project linked to Gemini

Gemini CLI is no longer a target: `link --target gemini` and `unlink --target gemini` are refused
as invalid values. A project linked to Gemini by an earlier version keeps its generated files and
its `gemini:` entry under `targets:` in `.armadai/link-manifest.yaml`. Clean it up by hand:

1. Delete the files listed under `targets: gemini:` that have `outcome: created` (by default
   under `.gemini/`), after checking you have not edited them since. Leave the entries with
   `outcome: skipped`: those are hand-written files `link` found and never wrote. Then remove the
   now-empty directories listed in that block's `created_dirs`.
2. Remove the `gemini:` block under `targets:` of `.armadai/link-manifest.yaml`.
3. If `.armadai/config.yaml` sets `link.target: gemini`, change it to another target: otherwise
   a bare `armadai link` fails with "Unknown link target".

Until you do, the `gemini:` entry is left alone: `link` and `unlink` on the other targets keep
working with it in place. Alternatively, run `armadai unlink --target gemini` with the previous
version before upgrading.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Providers and models](providers.md) — how `link` chooses the model it writes
- [Skills & Prompts](skills-prompts.md) — composable prompts and skills
- [Policy gate](policy-gate.md) — enforcing the declared topology in Claude Code
