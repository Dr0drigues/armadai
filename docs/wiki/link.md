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
4. Files shared with you (`CLAUDE.md`, `settings.json`, `opencode.json`) are checked before any
   write: a symbolic link, invalid JSON or an unterminated ArmadAI block is an error.
5. Writes, then the report.

Every target is planned and checked before the first write.

## Claude Code

| Declared | Produced |
|---|---|
| agent `<id>` | `.claude/agents/<id>.md`: `name`, `description`, `model` (tier `fast`/`pro`/`max` -> `haiku`/`sonnet`/`opus`, or `native.claude.model`; omitted without a tier), `disallowedTools` for `deny` capabilities, `skills`, then `native.claude` keys |
| skill `<id>` | `.claude/skills/<id>/`, every file byte for byte (executable bit kept) |
| `slots:` of an agent | the skills filling them added to its `skills` (preloaded); report `agent <id> slot <s>: compiled (<skills>)`, or `not-compiled (no skill fills it)`. For the coordinator (main session, no preloading) a filled slot is `approximated`, like its `skills:` |
| command `<id>` | `.claude/commands/<id>.md`: `description`, `argument-hint` (from `argument_hint`), then `native.claude` keys (a key colliding with a generated one is an error); the body as is. Report `command <id>: compiled` |
| coordinator | the root `CLAUDE.md`: the coordinator's prompt and a generated `## Delegation` section (agents, flows, steps, states, transitions), between `<!-- armadai:begin -->` and `<!-- armadai:end -->`. An existing `CLAUDE.md` keeps its content; the block is appended or replaced in place |
| automation | `.claude/settings.json` hooks: `session-start` -> `SessionStart`, `session-end` -> `SessionEnd`, `before-tool` -> `PreToolUse`, `after-tool` -> `PostToolUse`, `on-prompt` -> `UserPromptSubmit`, `on-stop` -> `Stop`; `native.claude.matcher` is the group matcher, other `native.claude` keys go into the handler |
| `policy.delegation: strict` | a `PreToolUse` hook on `Agent|Task` running `armadai __claude-policy-gate` (see [Policy gate](policy-gate.md)) |
| a flow | with the flows, four async hooks running `armadai __record-flow-event` (`SubagentStart`, `SubagentStop`, `PreToolUse` on `SubagentHandback`, `UserPromptExpansion` (a typed command, its name only)) and, for an agent holding a step with `states:` or a failure path, a closing `STATE: <state>` instruction in its file (OpenCode agents get it too); report `flow recording: compiled`. `.armadai/.gitignore` gets `state/`. See [Flow status](flow-status.md) |
| `native.claude` of project.yaml | top-level keys of `settings.json` |

**Commands use `.claude/commands/`**, the format Claude Code documents as merged into skills
whose files "keep working" (skills page, read on 2026-10-21): a legacy format that may be
removed one day; the fallback would be a skill with `disable-model-invocation: true`. A skill and
a command of the same name both give `/<name>` and the skill wins, hence the
`command-shadows-skill` error. Commands depend on no agent: `link --agents x` writes them all.

The order of the keys of a generated subagent frontmatter is not a contract: `model` may move
from one version to the next (the YAML is equivalent).

Capabilities never produce `tools` (Claude Code's inheritance is kept): `deny` adds the tools to
`disallowedTools` (`read`: Read, Grep, Glob; `write`: Edit, Write, NotebookEdit; `shell`: Bash;
`web`: WebFetch, WebSearch; `delegate`: Agent); `allow` is the default; `ask` is reported
`approximated` (Claude Code asks per project or session, not per agent).

`settings.json` is merged, never overwritten: your keys, hooks and permissions are kept, in their
order (`link` and `unlink` never reorder them); a hook identical to one link would add is left to
you and reported, with the way to adopt it: `armadai link --adopt-hooks` takes over the hooks
identical to the ones link adds (on a clone or a worktree, where `.armadai/link-manifest.yaml`,
git-ignored, is missing), and `unlink` then removes them. The option adopts hooks only: a
`permissions` rule identical to one of yours is never claimed. An event array, or a `hooks` object, that was empty in your file and held only adopted hooks is removed by `unlink` with them. The file is rewritten
only when its value changes, and the JSON layout is then normalised (reported). Flows are
guidance in `CLAUDE.md`; under `strict`, the delegation topology is enforced by the gate.

`native.claude.permissions` is merged list by list into the `permissions` of the file (every key
holding a list: `allow`, `ask`, `deny`, `additionalDirectories`, …): your rules are kept, the
project's rules are added after them without duplicates, and `unlink` removes only the rules
link added. Another value (for instance `defaultMode`) keeps the whole-value rule: a different
value of yours is an error that names both values (``key `permissions.defaultMode` is already set
to another value: "plan" (link writes "acceptEdits")``) and nothing is written. Claude Code
evaluates `deny` first, whatever the file ([permissions](https://code.claude.com/docs/en/permissions.md),
checked 2026-10-28): a `deny` rule link adds that your `allow` lists too, character for
character, is reported (`permissions.deny <rule>: compiled (also in your permissions.allow:
Claude Code applies deny)`). A rule whose pattern merely overlaps one of yours
(`Bash(git *)` against `Bash(git push:*)`) is not seen. A binary older than this version
(1.0.0-rc.8 and before) ignores the `permissions` record of the manifest: its `unlink` leaves the
rules link added.

## OpenCode

Target: OpenCode 2.x only (checked against [the agents page](https://opencode.ai/docs/agents/),
[the config page](https://opencode.ai/docs/config/), [the rules page](https://opencode.ai/docs/rules/)
and the 2.x code; 1.x is not supported). OpenCode 2.x reads, in a project: `.opencode/agents/*.md`,
`opencode.json` at the root, `AGENTS.md` (the 2.x code looks for `AGENTS.md` only; the rules
page also mentions `CLAUDE.md`), the files
listed under `instructions` in `opencode.json`, and the skills of `.opencode/skills/`. Link writes
nothing else: `.opencode/instructions.md`, written by the former linker and read by no OpenCode
version, is no longer produced (a previous one is removed as a stale file while unchanged).

| Declared | Produced | Report |
|---|---|---|
| agent `<id>` | `.opencode/agents/<id>.md`: `description`, `mode: subagent`, `model` (see below), `temperature`, `permission`; the body is the prompt, ending with the `STATE: <state>` instruction when the agent holds a step with `states:` or a failure path (see [Flow status](flow-status.md)). No other key: OpenCode sends an unknown frontmatter key to the provider as a model option | `compiled` |
| coordinator | its agent in `mode: primary`, its prompt followed by a `## Delegation` section; `opencode.json`: `default_agent: <id>` (left out when `--agents` skips the coordinator and no previous link wrote its file) | `compiled` |
| team and flows | `.opencode/armadai-team.md` (in the output folder), listed under `instructions` in `opencode.json`: the agents by id and the flows (steps, states, transitions), described neutrally since every agent reads it. Written when there is a coordinator or a flow | `team file: compiled` |
| skill `<id>` | `.opencode/skills/<id>/`, every file byte for byte | `compiled` |
| `skills:` of an agent | `permission.skill`: `"*": deny`, then `<id>: allow` for each; the prompt says to load them with the `skill` tool | `compiled` |
| `slots:` of an agent (coordinator included) | the skills filling them are added to the loading prescription. A slot never restricts: only an agent with declared `skills:` gets `permission.skill` (`"*": deny`, then its skills and its slot skills as `allow`); an agent without `skills:` gets no `permission.skill`, slots or not, and keeps every skill, as under Claude Code where a slot preloads without restricting | `agent <id> slot <s>: compiled (<skills>)`, or `not-compiled (no skill fills it)` |
| command `<id>` | `<output>/commands/<id>.md` (`.opencode/commands/` by default): `description`, then `native.opencode` keys; the body is the template | `compiled`; `approximated (argument_hint has no OpenCode equivalent)` when it has an `argument_hint` |
| flow | guidance in the team file | `approximated (guidance only)`, `compiled` under `strict` |
| `policy.delegation: strict` | `permission.task` of each agent: `"*": deny`, then `allow` for each target the flows permit (same rule as the Claude [policy gate](policy-gate.md)) and the `free_agents`; the main-thread rule on the built-in `build` and `plan` agents (`agent.build.permission.task`, `agent.plan.permission.task` in `opencode.json`); `experimental.subagent_depth` the longest delegation chain of the flows (1 to 8) | `compiled (enforced by OpenCode permissions)`; primary agents of your own outside `.armadai/` are not covered |
| `policy.delegation: off` | no `permission.task` | flows `approximated` |
| capabilities | `deny` / `ask` on the agent's tools: `read` -> `read`, `grep` and `glob`, `write` -> `edit`, `shell` -> `shell`, `web` -> `webfetch` and `websearch`, `delegate` -> `task`; `allow` is OpenCode's default | `compiled` |
| automation | nothing (it would need an OpenCode plugin) | `not-compiled` |
| `native.opencode` of an agent | merged into its frontmatter after the generated keys; a key colliding with a generated one is an error | `compiled` |
| `native.opencode` of project.yaml | merged into `opencode.json` as owned keys | `compiled` |

**Models.** By default an agent gets no `model`: it inherits the model of the OpenCode session
(`model: approximated (inherited from the OpenCode session)` when it has a tier).
`native.opencode.model` (a `provider/model` string) is always written as is. A tier can also be
mapped per machine in your user configuration, `~/.config/armadai/config.yaml`:

```yaml
model_tiers:
  opencode:
    fast: <provider>/<model>     # each value as `opencode models` lists it
    max: <provider>/<model>
```

A tier left out keeps the session's model.

The link output then depends on that file for this key only, and the report says where the model
comes from. A model without a provider (no `/`) is not written and reported. `link` stays
offline: nothing is looked up.

**Under `strict`, `build` and `plan` change.** Switching to OpenCode's own `build` or `plan`
agent (Tab) must not get around the declared topology, so link restricts their delegations too.
Their other settings are left alone, and `unlink` removes the rule.

**`opencode.json` is shared with you.** It is merged, never overwritten: link owns only the keys
and array entries it adds (`default_agent`, its `instructions` entry, `agent.build.permission.task`,
`agent.plan.permission.task`, `experimental.subagent_depth`, the `native.opencode` keys), recorded
in the manifest; your keys and your `instructions` entries are kept, in their order (OpenCode
permission rules are ordered: the last match wins). A key you already set to another value than
the one link wants (for instance `default_agent` or `experimental.subagent_depth`) makes the link
fail before any write (``key `default_agent` is already set to another value: "mine" (link writes "lead")``), as does an owned
value you changed by hand since the last link (``key `…` was changed by hand since the last link``):
remove or restore it, then link again. A key no longer wanted is
removed on the next link, and a file link created is removed once it owns nothing in it
(`removed: link created it and no owned key is left`). An `opencode.jsonc` stops the link before
any write (rename it to `opencode.json`), and `unlink` checks it can undo every shared file
before removing anything (a comment added to `opencode.json` stops it with nothing removed).
`AGENTS.md` and `CLAUDE.md` are never created nor modified: they are yours.

`native.opencode` of project.yaml may not name a key link owns: `default_agent` with a
coordinator, `instructions` with a team file, and, under `strict`, the whole top-level `agent` and
`experimental` keys (they are not merged in depth). A key holding a `.` is always refused. Each
is an error naming the key. A native file `.armadai/native/opencode/opencode.json` collides with the generated
`opencode.json` and is an error too: use `native.opencode` of project.yaml.

## Codex, Copilot

Their output is the one of the former linkers, fed from the canonical model: one agent file per
agent, the coordinator in the target's instructions file (`AGENTS.md`,
`copilot-instructions.md`), skills copied. Flows, automations, capabilities, agent skills, slots and the
policy are reported `not-compiled` for now; each command is `not-compiled (no command support yet)`. Models are chosen offline, from the static tier
table: `native.<target>.model`, else for Codex the tier (default `pro`) as an OpenAI model;
Copilot files carry no model.

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
block of the root `CLAUDE.md`, the hooks (and `native.claude` keys) it added to
`<output>/settings.json`, and the keys and `instructions` entries it owns in `opencode.json`, as
exact values. `armadai unlink` reads that manifest and applies the
inverse of each recorded outcome, so it removes exactly what `link` produced and nothing else —
including files whose agent has since been removed from the project, which is the case no amount
of regeneration can recover — then forgets the target.

The manifest is data on disk, so `unlink` treats it as untrusted input: it re-derives the target's
output directory from the project config and refuses the manifest wholesale if the two disagree,
and it refuses any individual entry or recorded directory that resolves outside the target's own
root, onto that root itself, or that corresponds to no file `link` recorded creating.

`unlink --agents x` removes the files of those agents and, as for the skills, every command (a
command belongs to no agent). Directories link created are removed once empty, except the
target root itself (`.claude/`, `.opencode/`…), which `unlink` never removes: it stays, empty.

`--dry-run` writes no manifest. When no usable manifest exists — a fresh clone, a deleted
`.armadai/`, or a project linked before the manifest existed — `unlink` says so and falls back to a
content-match guard: it recompiles the project and removes a file only when its bytes still match
what `link` would write now, byte for byte. It never touches `settings.json`, `CLAUDE.md` nor `opencode.json` in
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
