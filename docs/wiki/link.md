# Link

`armadai link` compiles the project (`.armadai/project.yaml` and its folders, see
[Project format](project-format.md)) into the native files of each target, and prints a
**conformance report**: for every declared element and target, whether it was `compiled`,
`approximated` (with the reason) or `not-compiled` (with the reason). Nothing is degraded in
silence. `link` never reaches the network.

A legacy project (`.armadai/config.yaml`, `armadai.yaml`, `armadai.yml`) is refused with
"legacy format, run `armadai migrate`".

## Usage

```bash
armadai link                         # targets of project.yaml, else claude
armadai link --target claude codex   # several targets
armadai link --dry-run               # the plan and the report, nothing written
armadai link --json                  # the report alone, as JSON, on stdout
armadai link --agents reviewer       # only these agents' files (CLAUDE.md and hooks unaffected)
armadai link --output tools/claude   # one target, another output folder
armadai link --force                 # overwrite files link did not write
```

| Option | Meaning |
|---|---|
| `--target` | Targets (`claude`, `codex`, `copilot`, `opencode`); default: `targets` of project.yaml, else `claude` |
| `--output` | Output folder of a single target (default: `outputs.<target>`, else `.claude`, `.codex`, `.github`, `.opencode`). It may leave the project for one run, never into `.armadai/`, `.git/` or onto the project root |
| `--agents` | Write only these agents' files; each one left out gets a `skipped (--agents)` line. Never removes the other agents' files |
| `--dry-run` | Print the files, the stale files a real run would remove, and the report; write nothing |
| `--json` | Print the report as a JSON array of `{target, element, status, reason}` and nothing else |
| `--force` | Overwrite a file that exists and was not written by link |

## Pipeline

1. Load and validate the project: any error stops before the first write.
2. One plan per target: whole files, the root `CLAUDE.md` block, the settings hooks.
3. `.armadai/native/<target>/<path>` files are copied as is to `<project>/<path>` (`--output`
   does not move them). A native file colliding with a generated one, a link leaving the
   sources, or a path into `.armadai/` or `.git/` is an error and nothing is written.
4. Files shared with you (`CLAUDE.md`, `settings.json`) are checked before any write: a symbolic
   link, invalid JSON or an unterminated ArmadAI block is an error.
5. Writes, then the report.

Every target is planned and checked before the first write.

## Claude Code

| Declared | Produced |
|---|---|
| agent `<id>` | `.claude/agents/<id>.md`: `name`, `description`, `model` (tier `fast`/`pro`/`max` -> `haiku`/`sonnet`/`opus`, or `native.claude.model`; omitted without a tier), `disallowedTools` for `deny` capabilities, `skills`, then `native.claude` keys |
| skill `<id>` | `.claude/skills/<id>/`, every file byte for byte (executable bit kept) |
| coordinator | the root `CLAUDE.md`: the coordinator's prompt and a generated `## Delegation` section (agents, flows, steps, states, transitions), between `<!-- armadai:begin -->` and `<!-- armadai:end -->`. An existing `CLAUDE.md` keeps its content; the block is appended or replaced in place |
| automation | `.claude/settings.json` hooks: `session-start` -> `SessionStart`, `session-end` -> `SessionEnd`, `before-tool` -> `PreToolUse`, `after-tool` -> `PostToolUse`, `on-prompt` -> `UserPromptSubmit`, `on-stop` -> `Stop`; `native.claude.matcher` is the group matcher, other `native.claude` keys go into the handler |
| `policy.delegation: strict` | a `PreToolUse` hook on `Agent|Task` running `armadai __claude-policy-gate` (see [Policy gate](policy-gate.md)) |
| `native.claude` of project.yaml | top-level keys of `settings.json` |

Capabilities never produce `tools` (Claude Code's inheritance is kept): `deny` adds the tools to
`disallowedTools` (`read`: Read, Grep, Glob; `write`: Edit, Write, NotebookEdit; `shell`: Bash;
`web`: WebFetch, WebSearch; `delegate`: Agent); `allow` is the default; `ask` is reported
`approximated` (Claude Code asks per project or session, not per agent).

`settings.json` is merged, never overwritten: your keys, hooks and permissions are kept, in their
order; a hook identical to one link would add is left to you and reported. The file is rewritten
only when its value changes, and the JSON layout is then normalised (reported). Flows are
guidance in `CLAUDE.md`; under `strict`, the delegation topology is enforced by the gate.

## Codex, Copilot, OpenCode

Their output is the one of the former linkers, fed from the canonical model: one agent file per
agent, the coordinator in the target's instructions file (`AGENTS.md`,
`copilot-instructions.md`, `instructions.md`), skills copied. Flows, automations, capabilities,
agent skills and the policy are reported `not-compiled` for now. Models are chosen offline, from
the static tier table: `native.<target>.model`, else for Codex the tier (default `pro`) as an
OpenAI model, for OpenCode the tier as an Anthropic model (no tier: no model); Copilot files carry
no model.

## Files link did not write

A file that exists with other bytes is left alone unless `--force`: its report line becomes
`not-compiled (file exists and was not written by link (use --force): <path>)`. A file still
exactly what link wrote last time (per the manifest) is link's own output and is rewritten when
its source changed. A file the previous link wrote and that is no longer produced is removed
while unchanged (`stale … removed`), else left in place and reported.

## The Link Manifest

`armadai link` records what it wrote in `.armadai/link-manifest.yaml`, grouped by target: the
target's output root, the directories `link` itself created, and one entry per file with its
`produced_by`, its `outcome` (`created` or `skipped`), and — for `created` — the `sha256:` digest
of the bytes written.

It also records the files `link` **patched** rather than wrote whole (`patched`): the ArmadAI
block of the root `CLAUDE.md` and the hooks (and `native.claude` keys) it added to
`<output>/settings.json`, as exact values. `armadai unlink` reads that manifest and applies the
inverse of each recorded outcome, so it removes exactly what `link` produced and nothing else —
including files whose agent has since been removed from the project, which is the case no amount
of regeneration can recover — then forgets the target.

The manifest is data on disk, so `unlink` treats it as untrusted input: it re-derives the target's
output directory from the project config and refuses the manifest wholesale if the two disagree,
and it refuses any individual entry or recorded directory that resolves outside the target's own
root, onto that root itself, or that corresponds to no file `link` recorded creating.

`--dry-run` writes no manifest. When no usable manifest exists — a fresh clone, a deleted
`.armadai/`, or a project linked before the manifest existed — `unlink` says so and falls back to a
content-match guard: it recompiles the project and removes a file only when its bytes still match
what `link` would write now, byte for byte. It never touches `settings.json` nor `CLAUDE.md` in
that mode ("hooks/CLAUDE.md block not removed: no manifest"): without the record, nothing there
is provably link's. Re-run `link` to write a manifest.

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


## A project linked to Gemini

Gemini CLI is no longer a target: `link --target gemini` and `unlink --target gemini` are refused
as invalid values. A project linked to Gemini by an earlier version keeps its generated files and
its `gemini:` entry under `targets:` in `.armadai/link-manifest.yaml`. Clean it up by hand:

1. Delete the files listed under `targets: gemini:` that have `outcome: created` (by default
   under `.gemini/`), after checking you have not edited them since. Leave the entries with
   `outcome: skipped`: those are hand-written files `link` found and never wrote. Then remove the
   now-empty directories listed in that block's `created_dirs`.
2. Remove the `gemini:` block under `targets:` of `.armadai/link-manifest.yaml`.
3. If `.armadai/project.yaml` lists `gemini` under `targets`, remove it (the validator reports
   it as `unknown-target`).

Until you do, the `gemini:` entry is left alone: `link` and `unlink` on the other targets keep
working with it in place. Alternatively, run `armadai unlink --target gemini` with the previous
version before upgrading.

## See Also

- [Project format](project-format.md) — the files `link` reads
- [Policy gate](policy-gate.md) — enforcing the declared topology in Claude Code
- [Getting Started](getting-started.md) — installation and first steps
