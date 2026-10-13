# Starter Packs

A starter pack is a ready-made project: agents, skills, flows and automations in the
[Project format](project-format.md), next to a `pack.yaml`. `armadai init --pack <name>` validates
it, then copies it into `.armadai/` of the current directory; `armadai link` compiles it for your
assistant.

## Usage

```bash
armadai init --pack rust-dev                 # a built-in pack, by name
armadai init --pack ./path/to/my-pack        # a pack folder
armadai init --pack fullstack --force        # merge into an existing project
armadai validate && armadai link
```

What `init --pack` does with a native pack (`format: 1`):

1. Refuses a folder that already holds `.armadai/project.yaml`, unless `--force`.
2. Refuses a pack holding any symbolic link.
3. Validates the pack in a scratch project: an error stops it, nothing is written (diagnostics
   point into the pack).
4. Copies, by a white list: `project.yaml`, `agents/*.md`, `skills/**`, `flows/*.yaml`,
   `automations/*.yaml`. `pack.yaml`, a README, `.armadai-version`, `link-manifest.yaml`,
   `.migrated/` and `.gitignore` stay behind.
5. Keeps an existing file (one warning each). Under `--force`, an existing `project.yaml` wins
   over the pack's, and the warning says its targets, coordinator and policy are not applied.
6. Writes `project.yaml` (`schema_version: 1`, `targets: [claude]`) when neither has one, and
   `.armadai/.gitignore`.
7. Lists every automation of the pack, with its command: it becomes a hook the target runs.
8. Undoes everything it wrote if the result has an error the project did not have.

A pack without `format` in its `pack.yaml` is the **legacy format** (an `agents:` list,
`prompts/`): it is still converted at install, like `armadai migrate`, whatever its origin, and
`init` says "legacy pack converted". Another `format` value is refused.

## Available Packs

The seven packs below ship embedded in the binary, all native.

| Pack | Agents | Flows | Skills |
|---|---|---|---|
| `rust-dev` | `code-reviewer`, `test-writer`, `debug` | none | `rust-conventions` |
| `fullstack` | `test-writer`, `code-reviewer`, `doc-generator` | `review` (review loop: tests, review, docs) | none |
| `code-analysis-rust` | `lead-analyst` (coordinator), `rust-reviewer`, `rust-test-analyzer`, `rust-doc-writer`, `rust-security` | `analysis` (fan-out) | `analysis-standards` |
| `code-analysis-web` | `lead-analyst` (coordinator), `web-reviewer`, `web-test-analyzer`, `web-doc-writer`, `web-security` | `analysis` (fan-out) | `analysis-standards` |
| `armadai-dev` | `dev-lead` (coordinator), `schema-specialist`, `adapter-specialist`, `cli-specialist`, `qa-specialist`, `release-manager` | `delivery` (fan-out), `release` (QA approves, then the release manager) | `rust-conventions` |
| `armadai-authoring` | `authoring-lead` (coordinator), `authoring-architect`, `agent-builder`, `skill-builder`, `starter-builder`, `authoring-reviewer`, `authoring-tester` | `authoring` (fan-out), `pack` (plan, build, review loop, test) | `armadai-conventions` |
| `pirate-crew` | `capitaine` (coordinator), `vigie`, `charpentier`, `cartographe` | `mission` (fan-out) | `code-nautique` |

`fullstack` has no coordinator, on purpose: a coordinator becomes the main session itself
(`CLAUDE.md` for Claude Code), and making `code-reviewer` that would turn every conversation in
the repository into a code review. Its `review` flow is therefore guidance only (`link` reports
it `not-compiled`): set `policy.delegation: strict` to have Claude Code enforce it (the main
session then starts at `test-writer`), or add a lead agent of your own as `coordinator`.

In the packs, an agent whose step a flow follows by state (`approved`, `rejected`) ends its
answer with that state on a last line: `code-reviewer` (fullstack), `qa-specialist`
(armadai-dev, for a release), `authoring-reviewer`. The authoring skills
(`armadai-agent-authoring`, ...) are not in `armadai-authoring`: they are built into ArmadAI and
installed in `~/.config/armadai/skills/` by `armadai init`; copy one into `.armadai/skills/` to
give it to the agents.

## Pack Format

```
<pack-name>/
├── pack.yaml              # format: 1, name, description
├── project.yaml           # optional: targets, coordinator, policy
├── agents/<id>.md
├── skills/<id>/SKILL.md   # and the rest of each skill folder
├── flows/<id>.yaml
└── automations/<id>.yaml
```

```yaml
# pack.yaml
format: 1
name: rust-dev
description: "Rust development agents: code review, test writing and debugging"
```

`armadai validate <pack-dir>` validates a native pack as the project it installs (the default
`project.yaml` when it ships none). A legacy pack is checked as before (its agents and prompts
exist and parse).

## Embedded Versioning

Starter packs embedded in the binary are extracted to `~/.config/armadai/starters/` on first use,
with a `.armadai-version` marker. When the binary changes, each pack is extracted next to the old
one and swapped in, so nothing of a previous version survives. The marker is never copied into a
project. The same version mechanism applies to built-in skills.

## Creating Custom Packs

1. Build the team in a scratch project: `armadai init --project`, `armadai new agent ...`,
   `armadai new flow ...`, `armadai validate`, `armadai link`.
2. Copy its `.armadai/` content (`project.yaml`, `agents/`, `skills/`, `flows/`,
   `automations/`) into a pack folder and add `pack.yaml` with `format: 1`.
3. Check it: `armadai validate <pack-dir>`, then `armadai init --pack <pack-dir>` in a scratch
   folder.
4. Put it in one of the starter directories below, or share its folder.

The `armadai-authoring` pack and the `armadai-starter-authoring` skill help with each step.

## Custom Starter Directories

### Priority order

1. **Built-in** — embedded in the binary
2. **Project-local** — `.armadai/starters/` in the current project
3. **User library** — `~/.config/armadai/starters/`
4. **Environment variable** — `ARMADAI_STARTERS_DIRS` (colon-separated paths)
5. **Config file** — `starters_dirs` in `~/.config/armadai/config.yaml`

### Environment variable

```bash
export ARMADAI_STARTERS_DIRS=/path/to/starters1:/path/to/starters2
```

### Config file

Add custom directories to `~/.config/armadai/config.yaml`:

```yaml
starters_dirs:
  - /path/to/my-starters
  - /shared/team-starters
```

Or use the CLI:

```bash
armadai config starters-dir add /path/to/my-starters
armadai config starters-dir list
armadai config starters-dir remove /path/to/my-starters
```

### Remote starter registries

When `armadai init --pack <name>` finds no local pack of that name, it syncs the remote starter
registries declared under `registries:` (in `~/.config/armadai/registries.yaml` or the project
config) once, then retries. A pack that exists locally never triggers a sync.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Templates](templates.md) — creating agents from templates
- [Link](link.md) — compiling a pack's agents for your assistant
