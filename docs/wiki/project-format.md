# Project format

A project is a `.armadai/` directory whose manifest, `.armadai/project.yaml`, **defines** it.
Every project command (`link`, `unlink`, `list`, `inspect`, `validate`, `new`, `graph`) reads this
format only; a legacy project (`.armadai/config.yaml`, `armadai.yaml`, `armadai.yml`) is refused
with "legacy format, run `armadai migrate`" (see [Migrating](#migrating-a-legacy-project)).

```
.armadai/
  project.yaml            # manifest
  agents/<id>.md          # frontmatter + prompt
  templates/<id>.md       # agent templates, rendered into agents/ instances
  skills/<id>/SKILL.md    # cross-tool skill standard, folder copied as is
  flows/<id>.yaml
  automations/<id>.yaml
  commands/<id>.md        # commands the user types (`/<id>`)
  native/<target>/<path>  # files copied as is by link
```

Every file is strict YAML: an unknown key, a duplicate key or a value outside its domain is an
error, except under `native` (opaque) and in a skill's frontmatter (owned by other tools: an
unknown key is a warning). Files are discovered per directory and sorted; nothing is listed in the
manifest. An **id** is the file (or folder) name and matches `[a-z0-9][a-z0-9-]{0,63}`; it is the
only identity of an agent, template, skill, flow, automation or command. The supported targets are `claude`, `codex`,
`copilot` and `opencode`. JSON Schemas of every file are in `schemas/v1/`.

## `project.yaml`

```yaml
schema_version: 1            # integer; the only accepted value is 1
name: my-project             # optional, informative
targets: [claude, opencode]  # default link targets
outputs: { claude: .claude } # optional: output directory per target
coordinator: lead            # optional: agent id whose prompt becomes the root file
policy:                      # optional
  delegation: strict         # off (default) | strict
  free_agents: [explorer, Explore]    # reachable from anywhere: project agent ids, or agents the target provides (Claude Code's Explore)
native: { claude: {} }       # optional, opaque
```

## `agents/<id>.md`

```markdown
---
description: Writes the code            # required, one line
title: Developer                        # optional (display)
tier: pro                               # fast | pro | max
temperature: 0.4                        # optional
tags: [dev]
skills: [rust-conventions]              # skill ids of the project
slots: [code-standards]                 # optional: slots other packs fill (see Slots)
capabilities: { read: allow, write: ask, shell: ask, web: deny, delegate: allow }
native: { claude: { model: claude-sonnet-4-5 } }
---
You are the developer…
```

- No `id` key: the id is the file name without `.md`.
- A missing `capabilities` block, or a missing key in it, means "keep the target's default".
- A concrete model goes in `native.<target>.model`; `tier` is the only portable setting.

## `templates/<id>.md`

An agent template is written once and gives several agents: agents that differ by a few
phrases (two checkers that read, never write and answer with the same verdict) come from one
template and a few lines each. A template is not an agent: it is never linked, and cannot be a
flow step, the `coordinator` or a `free_agents` entry.

```markdown
---
description: "Checks {{subject}}"       # the keys of an agent, all optional
tier: pro
capabilities: { write: deny, shell: deny }
---
You check {{subject}}: you read, you never write.
```

An agent of `agents/` becomes an **instance** of it with `from` and `vars`:

```markdown
---
from: checker                           # a template of templates/
vars: { subject: the frame }            # string values only
skills: [cadre]                         # any agent key: replaces the template's
---
Read the cadre skill first.             # optional, added after the template's prompt
```

The template is rendered at load time, every time: `link`, `inspect`, `graph` and the gate only
see the rendered agent, and the next `link` after a template change rewrites the files of its
instances (no `--force` needed while they were not edited by hand; one edited by hand is left
in place, and its report line names the template as `(from template checker)`). The canonical writer keeps an
instance as an instance (`from`, `vars`, its own keys and prompt); no command flattens it.
`armadai new agent guardian --from checker --var subject="the frame"` writes one (`--var` is
repeatable, the same name given twice is refused), `list` shows `guardian (from checker)` and a
`Templates` section, `inspect guardian` shows the rendered agent and its template.

- **Variables**: `{{name}}` or `{{ name }}`, `name` an id (`[a-z0-9][a-z0-9-]{0,63}`); anything
  else between braces (`{{Bad Name}}`) is plain text. They are replaced in the prompt and in the
  string values of the frontmatter, nested ones and `native` included, never in keys. A template
  is read as a frontmatter on its own before it is rendered, so a variable only stands in a
  string value or under `native`: `tier: "{{t}}"` is refused (`invalid-yaml`, unknown variant).
  The instance's own values and prompt are rendered with the same `vars`.
- **Values** are inserted as they are: a value holding `{{other}}` is not rendered again, and a
  value holding `:` or a line break stays one string (the rendering works on parsed values,
  never on the YAML text).
- **Merge, then render**: an instance key replaces the whole template key, at the top level
  only. With the template above, an instance `capabilities: { write: allow }` gives
  `{ write: allow }` alone (`shell: deny` is gone, not merged). A replaced key does not need the
  variables of the value it replaces: the template's `description` is never rendered for an
  instance that sets its own. The rendered agent must then be valid as any agent (`description`
  required).
- **Prompt**: the instance's prompt, when not empty, follows the template's after exactly one
  blank line.
- **Escaping**: `\{{` writes a literal `{{` and is never a variable (`\{{name}}` gives
  `{{name}}`). A backslash cannot be written right before a rendered variable: `\\{{name}}`
  gives `\{{name}}`.
  In a double-quoted YAML value, YAML itself refuses `\{` (`invalid-yaml`, unknown escape):
  write `"\\{{name}}"` or use single quotes (`'\{{name}}'`); both give `{{name}}`.
- **Diagnostics**: a variable the template uses and `vars` does not set is an error
  (`missing-template-var`); a `vars` entry no `{{…}}` uses is a warning
  (`unused-template-var`), and the agent still loads. An error that comes from the template (an
  unknown key, a wrong type) is reported once, on the template file, not on each instance.
  A template no agent uses is only checked as a file (YAML, keys, types): its other errors
  (an unknown skill) show up with its first instance, reported on the template.

Not in this first version (deferred, not refused): a template using `from` itself
(`nested-template`), logic in templates (conditions, loops, filters, default values), templates
of flows, skills or commands, generating N agents from a list (one instance is one file),
and checking a template no agent uses beyond its file.
The starting points of `armadai new agent --template` ([Templates](templates.md)) are copied,
not linked: they are not project templates.

## `skills/<id>/SKILL.md`

Frontmatter: `name` (equal to the folder name), `description` (required, at most 1024
characters), `license`, `compatibility`, `metadata`. Other keys are kept and reported as
warnings. The folder's other files (scripts, references) are untouched.

`metadata: { armadai-slot: code-standards }` makes the skill fill a slot (see
[Slots](#slots)); several slots are separated by commas (`armadai-slot: "code-standards, security"`).

## `flows/<id>.yaml`

```yaml
description: From code to delivery
start: first-draft                      # optional: the step the flow begins with
steps:                                  # step id -> definition
  first-draft: { agent: developer }
  review:      { agent: reviewer, states: [approved, rejected], on_failure: fixes }
  fixes:       { agent: developer }
  delivery:    { agent: deliverer }
links:
  - { from: first-draft, to: review }   # implied: when done
  - { from: review, to: fixes, when: rejected }
  - { from: fixes, to: review }
  - { from: review, to: delivery, when: approved }
```

- **Linear shortcut**: `sequence: [developer, reviewer, deliverer]` (exclusive with
  `steps`/`links`). An item `{ id: x, agent: y }` repeats an agent under another step id; a
  repeated plain agent gets `-2`, `-3`… It expands to steps and consecutive links.
- Every step has the states `done` and `failed`; `states` adds more. A link's `when` must be a
  state of its `from` step (default `done`).
- `on_failure: <step>` is a `when: failed` link; a step has at most one failure path.
- Loops are allowed; a link from a step to itself is an error.
- Only `.yaml` is loaded: a `.yml` file is ignored with a warning.
- `start: <step>` declares where the flow begins: it is then the only entry step. Without it, an
  **entry step** is one no unconditional link (`when` absent or `done`) leads to; a conditional
  link or an `on_failure` back to it does not count, and the `approved` end of a loop is not an
  entry (see `no-entry-step` below). When an entry found that way has an incoming link of any
  kind, the guess is warned (`ambiguous-entry`): declare `start:`. The `review-loop` template writes it. Without a coordinator, Claude Code's
  [policy gate](policy-gate.md) lets the main session reach the entry steps only.
- The steps are written sorted by id (`armadai new flow` too); their order means nothing, the
  links carry it.

## `automations/<id>.yaml`

```yaml
event: before-tool      # session-start | session-end | before-tool | after-tool | on-prompt | on-stop
run: ./scripts/check.sh # command, not empty
description: …          # optional
native: {}              # optional, opaque
```

## `commands/<id>.md`

A command is a shortcut the user types, `/<id> some text`. It runs in the main conversation, so
it keeps the interaction with the user (questions, validation); a long method belongs in a
skill the command tells the model to load.

```markdown
---
description: Refine an idea into a brief   # required
argument_hint: <idea>                      # optional, Claude Code autocomplete hint
native: { claude: { allowed-tools: Read } } # optional, merged into the target's frontmatter
---
Refine this idea into a brief: $ARGUMENTS
```

- The body is the text sent; `$ARGUMENTS` is replaced by everything typed after the name. It is
  the only portable placeholder: `$<digit>` and `$ARGUMENTS[N]` are an error, since Claude Code
  indexes arguments from 0 and OpenCode from 1 (`$5` in plain text too: write `5 USD`).
- `` !`cmd` `` and a line starting with ```` ```! ```` are an error: both targets would run the
  shell when the command is typed. An author who wants that writes it in `native`, at their risk.
- No `agent`, `context` or `subagent` key is generated: a command running in a subagent loses
  the interaction with the user. `native` can still set them.
- An id equal to a skill id is an error (Claude Code would run the skill and ignore the command);
  an id equal to a built-in command of Claude Code or OpenCode (`init`, `review`, `help`…, list in
  `crates/armadai-model/src/check.rs`) is a warning, since one hides the other.
- Only `.md` files directly under `commands/` are loaded: a sub-folder is ignored with a warning
  (Claude Code would name `commands/a/b.md` `/a:b`, OpenCode `/a/b`).

## Slots

A slot lets a pack be completed by another one (code standards, infrastructure docs, security
guides) without rewriting its agents. An agent declares what it consumes, `slots:
[code-standards]`; a skill declares what it provides, `metadata: { armadai-slot: code-standards
}`; `link` gives the agent every skill of the project that fills one of its slots: preloaded
under Claude Code, prescribed under OpenCode. A slot never restricts the agent's other skills:
under OpenCode only declared `skills:` produce a `permission.skill` rule (see
[Link](link.md#opencode)).

- The agent's skills are its `skills:` in declared order, then the skills of its slots sorted by
  id, each once.
- A slot only holds skills: it cannot bring agents nor change a flow.
- A slot no skill fills is not an error (the pack works with no standard plugged in): `link`
  reports `agent <id> slot <s>: not-compiled (no skill fills it)`. A skill filling a slot no agent
  declares is not a diagnostic either (a standards pack may be installed before the team).
- `armadai list` shows a `Slots:` section with the slots agents declare and the skills filling
  each (`code-standards: a-std, b-std`, or `(none)`); a slot only skills name does not appear.
  In `armadai inspect <agent>`, `Skills:` is the declared list, the skills coming from slots are
  on the `Slots:` lines.

## `native`

A map `{ claude|codex|copilot|opencode: <free YAML> }` accepted on the manifest, agents,
commands and automations. Only its keys are checked (they must be supported targets); its content is
"native, not checked".

## Creating a project

```bash
armadai init --project                  # an empty project
armadai new skill rust-conventions --description "Rust conventions of this project"
armadai new agent dev --template dev-test --stack rust --skills rust-conventions
armadai new agent reviewer --template dev-review --stack rust
armadai new agent guardian --from checker --var subject="the frame"   # templates/checker.md
armadai new flow review --template review-loop --agents dev,reviewer
armadai new automation fmt --event after-tool --run "cargo fmt"
armadai new command brief --description "Refine an idea into a brief"   # body: $ARGUMENTS
armadai validate
```

Or start from a starter pack (`armadai init --pack rust-dev`, see
[Starter packs](starter-packs.md)), and `armadai new` alone for a guided menu (it needs a
terminal).

Each `armadai new` command writes one file, never through a symbolic link and never over an
existing file without `--force`, then validates the project: a creation that would introduce an
error is undone (the folders it created removed, a file `--force` replaced put back), and its
diagnostics are marked "(rolled back)". Ids are refused, never rewritten, when they do not match
`[a-z0-9][a-z0-9-]{0,63}` or name a Windows device (`con`, `nul`, `com1`…). Agent and flow
templates: see [Templates](templates.md). A flow created in a project with neither a coordinator
nor `policy.delegation: strict` is guidance only, and `new flow` says so.

## Seeing a flow: `armadai graph`

```bash
armadai graph review > review.mmd     # one flow
armadai graph                         # every flow
```

`graph` prints Mermaid, and nothing else, on stdout:

```
%% flow: review
flowchart TD
  n_dev["dev"]
  n_reviewer["reviewer"]
  n_dev --> n_reviewer
  n_reviewer -->|"rejected"| n_dev
  n_reviewer -.->|"failed"| n_dev
```

- One node per step, `n_` followed by the step id (other characters than `[A-Za-z0-9_]` become
  `_`); its label is the agent, or `step (agent)` when they differ.
- `done` links are plain arrows, other states are labelled, failure paths (`when: failed`,
  `on_failure`) are dotted.
- Without a flow id, every flow is printed, one diagram each, separated by a blank line: Mermaid
  reads one diagram per document, so render them one at a time.
- An unknown flow lists the flows; an invalid project prints its diagnostics on stderr and exits
  with 1.

## Validation

```bash
armadai validate [path]          # exit code 1 if there is at least one error
armadai validate [path] --json   # the diagnostics as a JSON array on stdout, nothing else
```

A diagnostic prints as `file:line:column: error[code]: message`, followed by `hint: …` when a fix
is obvious. In JSON each one has `severity`, `code`, `file`, `path`, `line`, `column`, `message`
and `hint`. Positions in an agent's frontmatter are lines of the file. A semantic position (an
unknown agent, a missing step…) points at the first whole-word occurrence of the faulty name in
the file. `--json` only applies to new-format projects; on a legacy project it is ignored with a
warning on stderr.

### Diagnostic codes

| Code | Severity | Meaning | Fix |
|---|---|---|---|
| `missing-manifest` | error | `.armadai/project.yaml` not found | Create it with `schema_version: 1`, or run `armadai migrate` (library API only: the CLI picks the new mode only when `project.yaml` exists) |
| `unsupported-schema-version` | error | `schema_version` is not `1` | Set `schema_version: 1` |
| `invalid-yaml` | error | Syntax error, unknown or duplicate key, wrong type or value, empty or unreadable file | Fix the YAML at the given line |
| `missing-frontmatter` | error | An agent file does not start with `---` | Add a `---` frontmatter with at least `description` |
| `unclosed-frontmatter` | error | The frontmatter has no closing `---` | Add the closing `---` line |
| `ignored-file` | warning | A `.yml` file in `flows/` or `automations/` is not loaded | Rename it to `.yaml` |
| `sequence-and-steps` | error | A flow has both `sequence` and `steps`/`links` | Keep one form |
| `duplicate-step` | error | A `sequence` defines the same step id twice | Use `{ id: …, agent: … }` for the repeat |
| `bad-id` | error | A file, folder or step id, a `vars` name or a slot name of an agent's `slots`, does not match `[a-z0-9][a-z0-9-]{0,63}` | Rename it in kebab-case |
| `unknown-target` | error | A target in `targets`, `outputs` or a `native` key is not supported | Use `claude`, `codex`, `copilot` or `opencode` |
| `bad-output` | error | An `outputs` value is absolute, climbs with `..`, is the project root, or points into `.armadai/` or `.git/` | Use a folder inside the project such as `.claude` (`link --output` can point elsewhere for one run) |
| `unknown-agent` | error | A step or `coordinator` names no agent of the project | Use an existing agent id (the hint suggests a close one) |
| `unknown-free-agent` | warning | A `free_agents` entry names no agent of the project | None needed for an agent the target provides itself (Claude Code: `Explore`, `Plan`, `general-purpose`); otherwise fix the id (the hint suggests a close one) |
| `unknown-skill` | error | An agent lists a skill the project does not have | Add the skill or fix the id |
| `empty-description` | error | An agent's `description` is empty | Write a one-line description |
| `invalid-temperature` | error | `temperature` is not a finite number | Use a number such as `0.4` |
| `skill-name-mismatch` | error | A skill's `name` differs from its folder | Set `name` to the folder name |
| `skill-missing-description` | error | A skill has no or an empty `description` | Add one |
| `skill-description-too-long` | error | A skill's `description` exceeds 1024 characters | Shorten it |
| `unknown-skill-key` | warning | A skill's frontmatter has a key outside the standard | None needed if another tool uses it |
| `empty-flow` | error | A flow has no step | Add steps or delete the file |
| `duplicate-step-state` | error | `states` repeats a state or redeclares `done`/`failed` | Remove the duplicate |
| `unknown-step` | error | A link's `from`/`to`, an `on_failure` or `start` names no step of the flow | Use an existing step id |
| `unknown-state` | error | A link's `when` is not a state of its `from` step | Add it to `states`, or use `done`/`failed` |
| `self-link` | error | A link, or `on_failure`, goes from a step to itself | Point it at another step |
| `duplicate-link` | error | The same link is declared twice | Remove one |
| `duplicate-failure-path` | error | A step has more than one failure path | Keep either `on_failure` or one `when: failed` link |
| `no-entry-step` | warning | No step is an entry, so the flow can never start (without `start`). An entry step has no incoming *unconditional* link (`when` absent or `done`): a conditional link (`when: rejected`, `when: failed`) or an `on_failure` leading back to it does not count. Among such steps, one that another reaches without reaching it back (the `approved` end of a review loop) is not an entry | Declare `start: <step>`, or make one link into the first step conditional |
| `ambiguous-entry` | warning | The flow declares no `start` and a step the guess above takes as an entry has an incoming link (of any kind): where the flow begins is only a guess (an entry with no incoming link at all is not warned) | Declare `start: <step>` to say where the flow begins |
| `unreachable-step` | warning | A step cannot be reached from any entry step (every link and `on_failure` counts here) | Link it, or remove it |
| `empty-command` | error | An automation's `run` is empty | Give the command to run |
| `unknown-template` | error | An agent's `from` names no template of `templates/` | Use an existing template id (the hint suggests a close one) |
| `missing-template-var` | error | The template uses `{{x}}` and the instance's `vars` does not set `x` | Add `x` to `vars` |
| `unused-template-var` | warning | A `vars` entry is used by no `{{…}}` of the rendered agent (a template key the instance replaces does not count); the agent still loads | Remove it, or use it |
| `vars-without-template` | error | An agent has `vars` but no `from` | Add `from: <template>`, or remove `vars` |
| `nested-template` | error | A template uses `from` (a template does not inherit, not in this version) | Copy the keys you need into it |
| `template-agent-collision` | error | A template and an agent have the same id | Rename one of them |
| `bad-slot` | error | A skill's `armadai-slot` is empty, has an empty name (two commas in a row, a comma at an end) or a name that is not a valid id | Name the slots, separated by single commas: `armadai-slot: code-standards, security` |
| `command-empty-description` | error | A command's `description` is empty | Write a one-line description |
| `command-empty-argument-hint` | error | A command's `argument_hint` is empty | Remove it, or describe the expected input, such as `<idea>` |
| `command-empty-body` | error | A command's body is empty | Write the text the command sends, citing `$ARGUMENTS` |
| `command-positional-argument` | error | The body holds `$<digit>` or `$ARGUMENTS[`: Claude Code and OpenCode read them differently | Only `$ARGUMENTS` is portable: rephrase the text (an amount: `5 USD`) |
| `command-shell-expansion` | error | The body holds `` !` `` or a line starting with ```` ```! ````: a shell run when the command is typed | Remove it: ArmadAI never compiles shell execution at command time |
| `command-shadows-skill` | error | A command has the id of a skill | Rename one of them (Claude Code would run the skill and ignore the command) |
| `command-shadows-builtin` | warning | A command has the name of a built-in command of Claude Code or OpenCode | Pick another name: one of them hides the other |

## Migrating a legacy project

```bash
armadai migrate --dry-run          # convert and validate, write nothing, print the report
armadai migrate                    # migrate the current directory
armadai migrate --path DIR         # migrate another project
armadai migrate --force            # redo it once project.yaml exists (restore the legacy files first)
```

`migrate` reads the legacy project with the legacy loaders (the config, the `.md` agents it lists
under `agents:`, declared agents of `agents.yaml` with their fragments and `defaults`, prompts,
skills), then:

1. refuses if `.armadai/project.yaml` exists, unless `--force`;
2. converts it and validates the result in a scratch directory: a conversion that would not
   validate fails here, before anything is touched (`--dry-run` stops here);
3. copies the legacy files to `.armadai/.migrated/<unix seconds>/`, keeping their relative paths;
4. removes the legacy content of `.armadai/` (`config.yaml`, `agents.yaml`, `agents/`, `prompts/`,
   `skills/`) and the config file it read (a root `armadai.yaml`/`armadai.yml` is removed only when
   it is the one read). Files elsewhere — agents in a root `agents/`, another config file — are
   left in place and reported;
5. writes the new format and validates it again.

A failure after step 3 says where the backup is. The report printed on stdout has three sections:
**Written** (the new files), **Dropped** (every legacy field with no counterpart, with its reason)
and **Warnings**. An agent `.md` file the config does not list is not converted: it stays only in
the backup, with a warning ("not converted, kept only in the backup").

What the conversion does:

| Legacy | New |
|---|---|
| Agent `.md`: H1, `## System Prompt`, `## Instructions`, `## Output Format`, `## Context` | `id` = slug of the H1 (common accents folded; collision → `-2`, `-3`, reported; empty → `agent-N`); body = the sections composed as `link` did; `title` = H1 |
| Any other text of the `.md` (unknown `##` sections, text between the H1 and the first `##`…) | appended verbatim to the body, reported as "kept verbatim" |
| Description | the declared one (declared agents), else the first non-empty line of the prompt (reported) |
| `temperature` | kept only when it is not 0.7 |
| `model: latest:fast\|pro\|max` (and `low`/`medium`/`high`) | `tier`; `latest:auto` is dropped |
| Concrete model | `native.claude.model` (provider anthropic/claude) or `native.codex.model` (openai/codex); otherwise dropped |
| `tags` | `tags` |
| `provider`, `command`, `args`, `max_tokens`, `timeout`, `stacks`, `scope`, `model_fallback`, `cost_limit`, `rate_limit`, `context_window`, `mode`, `orchestration`, `## Triggers`, `## Ring Config`, `## Pipeline`, unknown `## Metadata` keys | **dropped**, listed per agent |
| Project `prompts:` | skills (`apply_to` dropped) |
| Project `skills:` | `skills/<id>/` (folder copied; `tools` and `version` dropped) |
| `link.target`, `link.overrides.<target>.output`, `link.coordinator` | `targets`, `outputs`, `coordinator` (an unsupported target such as `gemini` is dropped) |
| `orchestration.coordinator` + `teams` | a flow `orchestration`: coordinator → team lead → members (members without a lead under the coordinator); an unknown agent is omitted with a warning |
| `orchestration.policy`, `free_agents` | `policy.delegation`, `policy.free_agents` (a name that is not a project agent, such as `Explore`, is kept as it is: it may be an agent the target provides) |
| `orchestration.enabled`, `pattern`, `routes`, limits; `defaults`, `routing`, `sources` | **dropped**, listed |
| `registries:` (project-level registry sources) | **dropped**, one line per source, then a warning per registry type with the entries to paste where they can still be read (below) |

Agents of the user library that the project references are copied into the project.

**Project registries.** The new format has no `registries:` section, and nothing reads registry
sources from a migrated (or new) project: only the legacy config files did. `migrate` lists each
source it drops under **Dropped**, as `registries.<type>: "<url>"` where `<type>` is the registry
type (`skills`, `starters`, `agents`, `models`), followed by its delivery `kind:` (`git` or
`archive`) when one was declared. Then, for the `skills` and `starters` types, one **Warning** per
type gives the entries to paste into your user file `registries.yaml` (the path printed is the real
one: `$ARMADAI_CONFIG_DIR`, else `$XDG_CONFIG_HOME/armadai`, else `~/.config/armadai`), which
`armadai skills sync|search|info` and `armadai init --pack` read. URLs are written as YAML
double-quoted strings: control characters, U+FFFE/U+FFFF, bidirectional marks, line separators and
the BOM are escaped as `\uXXXX`, so a pasted entry reads back as the same URL.

- **Starters**: copy each entry whole. A `kind:` the URL does not imply (an archive served at
  `…/dl?id=1`) is lost with the URL alone, and the source is then cloned with git.
- **Skills**: skill sources are read as URLs only; a declared `kind:` is not read (it was not
  before either), so the entries to paste carry the URL alone. Only GitHub sources are fetched
  (`owner/repo` or `https://github.com/owner/repo`); `skills sync` refuses any other URL (a GitLab
  or self-hosted one) with "Use owner/repo or a GitHub URL", as it did in a legacy project.

The warning gives the entries as a flow list (`[{ url: … }, …]`). If the list already exists in
block form, add each entry as one `- { url: …, kind: … }` item. A project with many sources gets
one long warning line per type. The lists are top-level keys, without a `registries:` wrapper (a
wrapped file is read as empty):

```yaml
skills:
  - url: https://example.com/team/skills.git
starters:
  - url: https://example.com/team/starters.tar.gz
    kind: archive
```

`agents` and `models` registry sources are read by no command; they are dropped with that reason.

Known limits, also stated in every report:

- **YAML comments are not preserved**: the files are rewritten from their meaning. They remain in
  the backup.
- **Unknown keys of the legacy config file** (`config.yaml`, `armadai.yaml`) are not detected:
  the legacy loader ignores them, so `migrate` cannot see them. Review the backup. (`agents.yaml`
  is strict: an unknown key there stops the migration.)
- Packs (`pack.yaml`) are not projects and are not migrated.
