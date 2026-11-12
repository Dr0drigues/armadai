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
armadai init --pack rust-dev --link          # then run `armadai link` without asking
armadai validate && armadai link
```

After a pack install, `init` offers to run `armadai link` (the project's targets, no dry run):
`--link` does it without asking, `--no-link` skips it, and with neither a terminal is asked
(default yes) while a non-interactive run does not link. If the link fails, the project stays
created, the error is prefixed `the project was created, but link failed:` and the exit code
is not 0. Both flags require `--pack`.

What `init --pack` does with a native pack (`format: 1`):

1. Refuses a folder that already holds `.armadai/project.yaml`, unless `--force`.
2. Refuses a pack holding any symbolic link.
3. Validates the pack in a scratch project: an error stops it, nothing is written (diagnostics
   point into the pack).
4. Copies, by a white list: `project.yaml`, `agents/*.md`, `templates/*.md`, `commands/*.md`,
   `skills/**`, `flows/*.yaml`, `automations/*.yaml`. `pack.yaml`, a README, `.armadai-version`, `link-manifest.yaml`,
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

## Two modes: copy and reference

| | Copy (`init --pack`) | Reference (`pack add`) |
|---|---|---|
| What the project holds | the pack's files, in `.armadai/` | only your own files, plus `pack:` in `project.yaml` and `.armadai/pack.lock` |
| Who changes the pack's files | you (they are yours) | nobody here: the pack's version lives in the cache, read-only |
| Updating | `armadai pack update` replaces what you did not edit | `armadai pack update` moves the lock |
| Network | at install and at `pack update` | `pack sync`, `pack add`, `pack update` only |

Nothing converts one mode into the other on its own. `pack add` in a project that already has
files is allowed: they become the overlay on the pack.

## Building on a pack by reference

```bash
armadai pack sync                      # refresh every source of registries.yaml (network)
armadai pack add demo                  # the newest version, range ^<that version>
armadai pack add demo@^1.3 --source https://example.com/packs.git
armadai pack status                    # the pack, its lock, the cache (no network)
armadai pack update                    # the best cached version of the range, then link
armadai pack update 1.4.0              # an exact version inside the range
```

`pack add <name>[@<range>] [--source <url>]` builds the project on the pack: it writes the
`pack:` key at the end of `.armadai/project.yaml` (your comments stay; a new file gets
`schema_version: 1`), writes `.armadai/pack.lock`, records the project in the registry of
referents, and runs `armadai link`. `--source` names the source when more than one of your
`registries.yaml` holds the pack. If the project is already invalid, or the pack would make it
invalid, nothing is changed. If the link fails, the lock, the key and the files link had written
are put back.

```
Added pack demo 1.3.0 (0123456789ab): .armadai/project.yaml (version: ^1.3.0) and .armadai/pack.lock
```

**Ranges** (`pack.version`): `latest` (the newest version without a pre-release), an exact version
(`1.2.3` means exactly that one, a pre-release only this way), or a semver range (`^1.3`, `~1.2`,
`1`). Without `@`, `pack add` writes `^<resolved version>`, so `pack update` never jumps a major
version. A range no cached version matches is an error that names the cached versions and says to
run `armadai pack sync`, or to change `pack.version`.

**The lock.** `.armadai/pack.lock` (versioned) records the version, its tag and its commit (an
archive source: its digest). The commit is what counts: `link` reads the snapshot of that commit
and nothing else, so a tag that moves changes nothing. Only `pack update` advances the lock;
`pack update <version>` must stay inside the range. `pack sync` never writes it: it warns when
the tag now points elsewhere (`  warn: tag demo-v1.3.0 of pack demo now points to <new12>, .armadai/pack.lock says <old12>: the lock is kept (link uses its commit)`) or no longer exists. When the cache holds a
newer version than the lock, `pack sync`, `list`, `inspect` and `link` print

```
note: pack demo 1.4.0 is in the cache (locked: 1.3.0): run `armadai pack update`
```

A project that references a pack with no lock, or a lock for another pack or source, is refused
by every command that reads the project (`link`, `list`, `inspect`, `graph`, `flow`, `validate`,
`new`, `skills`, `unlink`), with a message that names the cure
(`armadai pack update`).

**The cache.** `pack sync` clones each git source (`--depth 1` first, then the whole history and
every tag, so any version can be extracted) or downloads each archive, and writes
`~/.config/armadai/registry/packs/<key>/versions.yaml`. Each version used is extracted once into
`<key>/<pack>/<commit>/` and never rewritten; its files are read-only. `link` and the other
commands read the cache without the network or `git`. A snapshot missing from the cache is
``pack demo 1.3.0 (0123456789ab) is not in the local cache: run `armadai pack sync` ``.

**The overlay.** The project the commands read is the pack's snapshot with your files laid over
it: a file of yours replaces the pack's file **at the same path**, and your other files are added.
`project.yaml` is merged key by key, recursively: a map is completed, a list gets the pack's items
then yours that it lacks, any other value is yours. The `pack:` key of your file never reaches the
merge. A pack whose own `project.yaml` has a `pack:` key is refused. The merge happens in a
scratch copy: `.armadai/` is never rewritten, and a pack's list is never shortened by yours.
`armadai models` sees only your overlay (it rewrites files and must never touch a snapshot).

**Other projects on the same pack.** `~/.config/armadai/pack-referents.yaml` lists the projects
that reference a pack (a `pack-referents.yaml.lock` file beside it serializes writers and stays).
After a `pack update` that moved the lock, the command lists the others and changes none of them:

```
Other projects referencing pack demo (not changed; run `armadai pack update` in each):
  /path/to/other-project
```

## Pack commands

| Command | Does |
|---|---|
| `armadai pack sync [--source <url>]` | Refresh the sources now (network): the ones of `registries.yaml`, and the project's own, registered or not. Prints `Synced <url>: <n> pack(s), <v> version(s)` per source; a source that fails is a warning and a non-zero exit |
| `armadai pack add <name>[@<range>] [--source <url>]` | Reference mode: see above |
| `armadai pack status` | Locked and newest cached versions, and the projects referencing the pack (no network). In copy mode: the installed version and what is in the cache |
| `armadai pack update [<version>]` | Reference: sync the source, move the lock to the best version of the range (or the one given), link. Copy: apply the new version of the pack to the files (below), link |
| `armadai pack new <name> [--empty \| --from-project] [--description …] [--dir …]` | Create a native pack folder |
| `armadai pack test [<dir>]` | Run the scripts the pack lists under `tests:` (the project's locked snapshot, or a folder; no network access of its own) |

`pack status` in reference mode:

```
Pack demo (reference)
  source:    https://example.com/packs.git
  range:     ^1.3.0
  locked:    1.3.0 (tag demo-v1.3.0, 0123456789ab)
  in cache:  1.3.0 matches the range (newest: 1.4.0)
```

**Updating a copy.** `init --pack` records in the link manifest, per file, the digest of what it
wrote (the `packs:` section). `pack update` then replaces the files you did not edit
(`  updated .armadai/<file>`), adds the pack's new ones (`  added`), removes the ones the pack
dropped and you did not edit (`  removed .armadai/<file> (no longer in the pack)`), and keeps the
rest with a warning each: edited since the install, removed by hand (not added back), not
installed by the pack, dropped by the pack but edited. An edited `project.yaml` is merged: your
keys stay, the pack's are added (`  merged .armadai/project.yaml (your keys kept, the pack's added)`);
if your file's comments, key order or inline forms cannot survive the canonical form, the
warning says it was rewritten. An embedded pack and a folder have no versions: the update
compares their content. If the update would make the project invalid, or the link fails, the files
and the record are put back.

**Creating a pack.**

```bash
armadai pack new demo                       # guided: name, description (needs a terminal)
armadai pack new demo --empty               # no questions
armadai pack new demo --from-project        # this project's .armadai/, as a pack
armadai pack new demo --dir ../packs        # in another parent folder
```

`--empty` writes `pack.yaml` (`format: 1`, the description, `A starter pack` by default), a minimal
`project.yaml`, and `agents/`, `skills/`, `flows/`, `automations/` each holding a `.gitkeep` (the
pack readers ignore it). `--from-project` copies the same white list as an install, and leaves out
`link-manifest.yaml`, `state/`, `pack.lock` and the vars files; a project that references a pack is
refused (its `.armadai/` is an overlay, not a pack). The pack is validated first: an invalid one
writes nothing. An existing folder of that name is refused (`<dir> already exists: choose another
name, or --dir`), and a failed write removes what the command created.

**Testing a pack.** A pack lists scripts in `pack.yaml`, relative paths inside the pack:

```yaml
tests:
  - tests/permissions.sh
  - tests/mr-rules.sh
```

`armadai pack test` runs them with `bash`, in a copy of the pack, with `HOME`, `CLAUDE_CONFIG_DIR`
and `ARMADAI_CONFIG_DIR` pointing at temporary folders; without a folder it takes the snapshot the
project locks from the local cache (`the locked snapshot of <name> <version> is not in the cache;
run armadai pack sync` when it is not there), with one it takes that folder. Each test may run 60
seconds, then it is killed with its child processes. The output is one line per test (`ok`, or `FAIL`
with the exit code and what the script printed) and the counts; the exit code is 1 when a test
fails, and a pack that lists none says so (`pack <name> declares no test (tests: in pack.yaml)`).
Only `pack test` runs a pack's scripts: `link`, `init` and `pack sync` never do.

`pack test` runs the pack's scripts with your permissions, in an isolated copy and with a temporary
`HOME`; it does not isolate the network. Only those three variables are replaced: every other
variable of the caller passes through as it is (`PATH`, `XDG_CONFIG_HOME`, tokens in the
environment), so a script sees them. A script can reach whatever your account can reach, so run
the tests of a pack you have read. A path in `tests:` is used as written (no `~` or `$HOME`
expansion, no trimming); an absolute path, a `..` component, an empty one or one holding a NUL byte
is the validation error `pack-test-path-escapes`, and a symbolic link anywhere in the pack is
refused. A file listed under `tests:` is not reported as `ignored-pack-file`.

## Variables

A pack is written once and used by many projects, so what differs per project or per machine is a
variable: `{{var.name}}` (or `{{ var.name }}`), `name` an id (`[a-z0-9][a-z0-9-]{0,63}`). `link`
replaces it, in the files it compiles:

- in the body (after the frontmatter) of agents, templates, commands and `SKILL.md`;
- in the text values **and the keys** of the `native` block of `project.yaml`;
- in the text values of `vars:` of an agent that is a template instance (`from:`): a value is
  written back as a string, so it is never parsed again as YAML (`a: b # c` stays that text).

Frontmatters (except `vars:` of an instance), `scripts/`, `references/` and `native/<target>/`
files are never substituted.
`\{{` is kept as it is, backslash included, and starts no variable; `{{name}}` without `var.`
belongs to agent templates. A value is inserted as it is, never rendered again.

Three layers give the value, the first that sets it wins:

1. the environment variable `ARMADAI_VAR_<NAME>` (name in capitals, `-` written `_`:
   `home-dir` is `ARMADAI_VAR_HOME_DIR`);
2. `.armadai/vars.local.yaml`, on this machine only (`link` adds it to `.armadai/.gitignore`);
3. `.armadai/vars.yaml`, versioned with the project.

Both files are a flat map (`name: value`); a bad name or bad YAML is an error that names the file.

```yaml
# .armadai/vars.yaml
home-dir: /srv/app
```

A name no layer sets becomes an empty value, and the report says so, as `approximated`:

```
var home-dir: approximated (set by none of ARMADAI_VAR_HOME_DIR, .armadai/vars.local.yaml, .armadai/vars.yaml: replaced by an empty value)
```

In a key of `native`, a variable is a rule selector: if no layer sets it, or a layer sets it to an
empty value, `link` stops before writing anything (a key `"{{var.data-dir}}/**"` would otherwise
become `"/**"`):

```
Error: project.yaml: under native, the key "{{var.data-dir}}/**" uses variable data-dir, which is empty or set by none of ARMADAI_VAR_DATA_DIR, .armadai/vars.local.yaml, .armadai/vars.yaml: a key never takes an empty value
```

Two keys of one object that render the same text stop it too:

```
Error: project.yaml: under native, the keys "{{var.a}}/**" and "/srv/data/**" both render as "/srv/data/**"
```

A value is more lenient: a variable that is empty or not set gives an empty value, and only a
variable that no layer sets (not one set to an empty value) adds the `approximated` line. The same
goes for a text value of `vars:` of an instance.

In an instance, `link` rewrites the `vars:` entry of the frontmatter, written as a block or as an
object on the same line (`vars: {dir: "{{var.d}}"}`). If the frontmatter has no such entry at the
start of a line (for instance a whole frontmatter in flow style, `{from: t, vars: {...}}`), `link`
stops with `agents/<id>.md: vars holds a pack variable but is not a "vars:" block at the start of a
frontmatter line: write it that way`. A diagnostic about a key of `vars:` points at the `vars:`
line, not at the entry.

Not covered yet: a variable in the frontmatter outside `vars:`, in `scripts/`, in `references/` or
in `native/<target>/` files stays as written, with no warning (tracked in
[#547](https://github.com/Dr0drigues/armadai/issues/547)).

The substitution happens at `link` only, on the effective view: the files of `.armadai/` and the
snapshots keep `{{var.name}}`.

## Sources, versions and secrets

A source is a git repository or an archive (see [Remote starter registries](#remote-starter-registries)).
It lists its packs in `armadai-packs.yaml` at its root:

```yaml
format: 1
packs:
  - name: demo
    path: packs/demo          # the folder of the pack, with its pack.yaml
    version: 1.3.0            # the version of its content at the current commit
```

For a git source, the history comes from the tags `<pack>-vX.Y.Z` (`demo-v1.3.0`); the version of
the index at the current commit is offered too when no tag carries it. An archive has no tags: it
needs the index to have versions at all, and keeps in the cache the versions of its earlier syncs
(each identified by a digest). A source without an index lists its packs with no version, so a
range cannot be used with it. A source is cloned with `git clone --depth 1`, then completed with
`git fetch --unshallow --force --prune --prune-tags`: submodules are not initialized, so a pack
cannot rely on one.

`.armadai/project.yaml`, `.armadai/pack.lock` and `pack-referents.yaml` are versioned or shared:
a source that carries a secret (`user:pass@`, a token in the userinfo of an `https` URL, a query)
is written there masked (`https://***`), as in messages. `pack sync`, `pack add` and `pack update`
find the real entry again in your `registries.yaml` (`the source <masked> matches none of your
sources (registries.yaml): add it there, with its credentials` otherwise). An SSH URL
(`git@host:org/repo.git`) carries no secret and is written as it is.

## Limits of pack management

- The link manifest keeps in clear a value resolved from a variable when it is written into a
  managed array (accepted and documented, not masked).
- A binary older than pack management that rewrites `.armadai/link-manifest.yaml` erases its
  `packs:` section: a copied pack then answers `the origin of this project's pack is unknown
  here` to `pack update`.
- That message is also what a fresh clone gets (the manifest is not versioned), and so does a
  project that never installed a pack but holds agents, skills or flows of its own: the command
  cannot tell them apart, so it guesses nothing. Only a project with nothing but a `project.yaml`
  gets "this project references no pack and records no installed pack: use `armadai pack add
  <name>` or `armadai init --pack <name>`".
- Two packs installed by copy in one project: `pack update` refuses and changes nothing.
- `vars.yaml` and `vars.local.yaml` are read through a symbolic link. The effective view copies the
  files it compiles and does not follow links. In a project with `pack:`, every command that
  reads the project refuses a symbolic link among the listed files (agents, templates, commands,
  skills, flows, automations): `<file> is a symbolic link, refused`, whether or not that file
  holds a variable. Without `pack:`, only `link` refuses, and only once any file holds a
  `{{var.…}}`.
- The undo of a failed `link` after `pack add` or `pack update` does not cover the process being
  interrupted (Ctrl-C) between the writes and the end of the link.
- For the policy gate (see [Policy gate](policy-gate.md)), a view that does not open is refused:
  missing cache, missing or foreign lock, a nested `pack:` in the pack, an unreadable pack
  `project.yaml`. A view that opens but whose merged project has errors leaves it with no opinion.
- Whole snapshot files lose their write bits, folders keep theirs: a file can be added to a
  snapshot by hand, none edited.
- A tag deleted in the source, followed by a `gc` of the local clone, can make a lock
  irreparable: the commit is gone with the tag. Tracked in [#540](https://github.com/Dr0drigues/armadai/issues/540).
- The `scripts/` and `references/` files of a skill, and the frontmatters (except `vars:` of an
  instance), are not substituted.
- A key at the top of `native.<target>` is checked once rendered: an error about it names the
  rendered key (for OpenCode: ``native.opencode key `<key>` contains a `.`: armadai cannot own it
  in opencode.json``).

## Available Packs

The eight packs below ship embedded in the binary, all native.

| Pack | Agents | Flows | Skills |
|---|---|---|---|
| `rust-dev` | `code-reviewer`, `test-writer`, `debug` | none | `rust-conventions` |
| `fullstack` | `test-writer`, `code-reviewer`, `doc-generator` | `review` (review loop: tests, review, docs) | none |
| `code-analysis-rust` | `lead-analyst` (coordinator), `rust-reviewer`, `rust-test-analyzer`, `rust-doc-writer`, `rust-security` | `analysis` (fan-out) | `analysis-standards` |
| `code-analysis-web` | `lead-analyst` (coordinator), `web-reviewer`, `web-test-analyzer`, `web-doc-writer`, `web-security` | `analysis` (fan-out) | `analysis-standards` |
| `armadai-dev` | `dev-lead` (coordinator), `schema-specialist`, `adapter-specialist`, `cli-specialist`, `qa-specialist`, `release-manager` | `delivery` (fan-out), `release` (QA approves, then the release manager) | `rust-conventions` |
| `armadai-authoring` | `authoring-lead` (coordinator), `authoring-architect`, `agent-builder`, `skill-builder`, `starter-builder`, `authoring-reviewer`, `authoring-tester` | `authoring` (fan-out), `pack` (plan, build, review loop, test) | `armadai-conventions` |
| `pirate-crew` | `capitaine` (coordinator), `vigie`, `charpentier`, `cartographe` | `mission` (fan-out) | `code-nautique` |
| `concept-to-code` | `lead` (coordinator), `analyst`, `framer`, `designer`, `architect`, `implementer`, `coach`, `guardian`, `validator` | `deliver`, `guide` | `briefing`, `cadre`, `standards-setup`, `mockups`, `diagrams`, `previewing` |

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

## The `concept-to-code` pack

From a vague idea to clean code, with you at every decision. Targets: Claude Code and OpenCode
(`policy.delegation: strict`, so the flows are enforced). `lead` is the only agent that talks
with you: the others return their questions to it.

**Team.** `analyst` writes the brief (problem, users, success criteria, non-goals); `framer`
finds the stack, conventions, security rules and infrastructure in place and records what is
decided and what you must still decide (it is called `framer`, not `scout`, because OpenCode
already has a built-in agent named `scout`); `designer` proposes three mockup variants;
`architect` designs components, interfaces and data, with diagrams; `implementer` writes the
code; `coach` guides you while you write it; `guardian` checks each change against the frame;
`validator` runs the tests and checks every success criterion.

**Two flows**, same steps except the build:

- `deliver`: the team writes the code (`implementer`).
- `guide`: you write the code, `coach` explains, proposes and reviews, and never writes a file.

Both run `refine` (brief), `frame`, `mockup`, `design`, `build`, `guard`, `validate`. `guard`
(`approved` / `rejected`) and `validate` (`approved` / `rejected` / `scope_changed`) end with a
state: a rejection goes back to `build`, a changed scope back to `refine`. `lead` asks you which
flow you want. Follow a run with `armadai flow status --watch` (Claude Code).

**Commands** (typed in the tool, `/<name> text`; each loads a skill):

| Command | Skill | Does |
|---|---|---|
| `/brief <idea>` | `briefing` | refines an idea into the brief, one question at a time |
| `/mockup <screen>` | `mockups` | three variants in one HTML file, you choose |
| `/iterate <feedback>` | `mockups` | three new variants built on the one you chose |
| `/init-standards` | `standards-setup` | analyses the repository, settles conventions with you, writes the project standards skill, then links |
| `/preview [file \| stop]` | `previewing` | serves `mockups/` and `docs/` on `127.0.0.1` and gives the link to a mockup or a diagram (the latest `.html` under `mockups/` or `docs/` with no argument), offers to open the browser; `stop` stops that server |

`/preview` always goes through a local server (`python3 -m http.server`, started in the
background on the first free port from 8765, bound to `127.0.0.1` only, reused if one already
serves the project), so every file gets the same kind of link,
`http://127.0.0.1:<port>/mockups/…` or `/docs/…`, which Claude Code opens on Cmd/Ctrl+click.
It never serves the project root: `http.server` does not check the `Host` header, so a web page
could read what it serves by DNS rebinding (a `.env`, `.armadai/`, the code). It serves
`.armadai/preview/` instead, a folder holding only the links `docs -> ../../docs` and
`mockups -> ../../mockups` and a marker `.project` (a random token written once, used to find
a running server); the skill adds `preview/` to `.armadai/.gitignore`. A file outside `docs/`
and `mockups/` is refused, with its `file://` path. While it runs, any program of the machine
can read those two folders: stop it with `/preview stop` once done. Without `python3` (or on
Windows), `/preview` gives the `file://` path. `lead` offers `/preview <file>` whenever
`designer` or `architect` produces an `.html` file, and the `mockups` and `diagrams` skills
offer it after writing one.

**Shortcuts.** Claude Code reads key bindings from `~/.claude/keybindings.json`, a user file
(not per project), so the pack does not install one. To bind `/preview` to Ctrl+X then P, add
to it:

```json
{
  "bindings": [
    { "context": "Chat", "bindings": { "ctrl+x p": "command:preview" } }
  ]
}
```

`command:<name>` is the documented action for a slash command; that it runs a project command
(one in `.claude/commands/`) and not only a built-in one is not verified yet. OpenCode has no
binding to a custom command: `ctrl+p` opens the command palette, where `preview` is listed.

**Slots.** The agents declare slots that your project fills: `code-standards` and `security`
(`framer`, `implementer`, `coach`, `guardian`; `validator` takes `code-standards` alone, for
the test command), `infra-docs` (`framer`, `architect`),
`ux-guidelines` (`designer`). An empty slot is not an error (`link` reports it `not-compiled`).
Fill them with `/init-standards`, which writes `.armadai/skills/project-standards/SKILL.md`, or
with a skill of your own whose frontmatter has `metadata: { armadai-slot: code-standards }`
(several slots: `"code-standards, security"`), then run `armadai link`.

**Where documents go.** The decisions live in `.armadai/docs/` (`brief.md`, `cadre.md`,
`design-decisions.md`); mockups in `mockups/<subject>/v<n>.html` (no dependency, open them in a
browser); lasting documentation in `docs/functional/` and `docs/technical/`. Diagrams rendered
as HTML load Mermaid from a CDN, so they need the network.

**Limits.** A write permission cannot be restricted to a folder: `write: allow` lets an agent
write anywhere, so the folders above are instructions, not enforcement. `coach`, `validator`
and the guardian have no write (`deny`). `lead` has `write: ask`; under OpenCode it also
executes commands. The shell is denied to `framer`, `architect` and `guardian`, and asked for
`implementer`, `coach` and `validator`.

## Pack Format

```
<pack-name>/
├── pack.yaml              # format: 1, name, description
├── project.yaml           # optional: targets, coordinator, policy
├── agents/<id>.md
├── templates/<id>.md     # agent templates (see Project format)
├── skills/<id>/SKILL.md   # and the rest of each skill folder
├── flows/<id>.yaml
└── automations/<id>.yaml
```

```yaml
# pack.yaml
format: 1
name: rust-dev
description: "Rust development agents: code review, test writing and debugging"
tests:                  # optional: scripts for `armadai pack test`
  - tests/review.sh
```

`armadai validate <pack-dir>` validates a native pack as the project it installs (the default
`project.yaml` when it ships none). A legacy pack is checked as before (its agents and prompts
exist and parse).

## Embedded Versioning

The packs embedded in the binary are extracted to `~/.config/armadai/starters/<pack>/`, and the
built-in skills to `~/.config/armadai/skills/<skill>/` by `armadai init`. Each extracted folder
holds a `.armadai-version` marker: the binary version and a fingerprint of the embedded content
on a first line, and the name of the folder on a second. One rule decides who owns a folder of an
embedded name:

- **Absent**: extracted fresh.
- **Ours** (a two-line marker naming that very folder): replaced whole when the binary's content
  differs (extracted next to it, then swapped in by renames, so it is never missing). Do not keep
  files of your own there.
- **Extracted by an older ArmadAI** (a one-line marker): moved aside once, whole, to
  `<name>.armadai-legacy-<seconds>` next to it, then extracted fresh; one line says where. Nothing
  is lost and nothing stale stays. Such a folder is never offered as a pack nor loaded as a skill;
  delete it by hand once checked.
- **Yours** (no marker, a symbolic link, or a marker naming another folder, as in a renamed copy):
  ArmadAI never writes, marks, replaces nor removes it. It **masks** the embedded pack or skill of
  the same name: `init --pack <name>` installs your folder, and `armadai config starters-dir list`
  says which embedded packs are masked. Rename or remove it to get the embedded one back.

A built-in skill a newer binary no longer ships is removed only if ArmadAI recorded it as
extracted (`~/.config/armadai/skills/.armadai-extracted`) **and** it is still ours by the rule
above. The marker is never copied into a project.

## Creating Custom Packs

1. Build the team in a scratch project: `armadai init --project`, `armadai new agent ...`,
   `armadai new flow ...`, `armadai validate`, `armadai link`.
2. Copy its `.armadai/` content (`project.yaml`, `agents/`, `templates/`, `skills/`,
   `flows/`, `automations/`) into a pack folder and add `pack.yaml` with `format: 1`: or let
   `armadai pack new <name> --from-project` do it (see [Pack commands](#pack-commands)).
3. Check it: `armadai validate <pack-dir>`, then `armadai init --pack <pack-dir>` in a scratch
   folder.
4. Put it in one of the starter directories below, or share its folder.

The `armadai-authoring` pack and the `armadai-starter-authoring` skill help with each step.

## Custom Starter Directories

### Priority order

`armadai init --pack <name>` takes a path (it holds `/`, or starts with `.` or `~`:
`./my-pack`, `../packs/x`, `/abs/x`, `~/packs/x`, a leading `~` being your home directory;
`~user` is not expanded) as a pack folder, and says `<folder> holds no pack.yaml` when it
holds none; an empty or blank name is refused. A bare name is never read from the
current directory: it is looked up in your own places first, then in the project's:

1. **Your starter folders**, the last one holding the pack wins: the built-in packs
   (embedded in the binary), then `~/.config/armadai/starters/`, then
   `ARMADAI_STARTERS_DIRS` (colon-separated paths), then `starters_dirs` in
   `~/.config/armadai/config.yaml`.
2. **Your remote sources**: the `starters:` of `~/.config/armadai/registries.yaml`.
3. **The project's sources**: `.armadai/starters/` in the current directory, and the
   `registries.starters` of a legacy project config. A pack taken from one of them is
   announced (`note: pack '<name>' comes from a project source ...`, with the source's URL
   masked).

A project source never masks one of yours: a pack found only in project sources is
installed only once all your sources synced without it; if one of them fails to sync
(offline, 404, broken archive), `init` stops and names it: retry once it can be reached,
or install a project's pack with the command the message prints for each candidate. A pack name matches a
folder name exactly, case included, whatever the file system; for a non-ASCII name, type
it as it is written on disk (the same accented letter can be stored in two Unicode forms),
as the `Available packs` list of the "not found" error shows it. A relative entry of `ARMADAI_STARTERS_DIRS` or
`starters_dirs` would be read from the current directory: it is ignored, with a warning
(`armadai config starters-dir add` refuses one). Two of your remote sources holding the
same pack is an error that names them; so are two project sources (`.armadai/starters/` and
a project registry included), once your sources are synced: install one by its folder
with the `armadai init --pack '<folder>'` command the message prints for each one (quoted
for the shell, it runs as pasted), or remove the other source. Only the caches of the sources declared now are read. Control characters of a
project's URL or path, and of the folder names in the `Available packs` list, are shown
escaped (`\u{1b}`).

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
registries once, then retries: the top-level `starters:` list of `~/.config/armadai/registries.yaml`
(no `registries:` wrapper there), plus the `registries.starters` of a legacy project's config. A
migrated project has none (see [Project format](project-format.md#migrating-a-legacy-project)). A
pack found in your own folders or sources does not trigger a sync for lack of a pack; one found only
in a project source does, so that one of your sources not synced yet still wins.

A pack of a source that lists a version for it in `armadai-packs.yaml` is installed at the commit
of that version's tag (`<pack>-vX.Y.Z`; an archive: its digest), not at the head of the clone, as
`pack update` would do.
If the cache does not know that version yet (`versions.yaml` lacks it, as before a first
`pack sync`), `init --pack` syncs the source then, even though the pack was found locally. A failed
sync is only a warning; the pack is then installed from the clone as it stands.

```yaml
# ~/.config/armadai/registries.yaml
starters:
  - url: https://github.com/acme/starters.git        # git clone
  - url: https://example.com/packs/starters.tar.gz   # archive (.tar.gz, .tgz or .zip)
    kind: archive                                     # optional: inferred from the extension
```

An archive source is downloaded with the `curl` program, which must be in `PATH` (it ships
with macOS, Windows 10+ and most Linux distributions), then extracted with `tar` or `unzip`:

- the URL must use `https://`, and so must every redirect;
- a download is capped at 300 seconds and 100 MiB;
- curl's environment applies (the proxy variables such as `https_proxy` and `no_proxy`, and
  `CURL_CA_BUNDLE`; other variables such as `SSL_CERT_FILE` depend on curl's version and TLS
  backend), but `.curlrc` files are ignored;
- `{}` and `[]` in the URL are sent as-is, never expanded;
- the `user:pass@` part of the URL and its query (`?token=…`) are masked in messages (a URL
  with an `@` after its host shows as `https://***`), and never appear in the cache directory
  name;
- an archive URL may end in a query (`…/starters.tar.gz?token=…`): the extension before it
  decides the kind;
- the archive is extracted and checked (no entry or link may point outside it, no dangling
  link, at most 100,000 entries, names of at most 4096 bytes and 64 MiB in total, and 1 GiB
  of files) before it replaces the previous copy: a
  failed sync keeps the previous copy, and never leaves a partial one;
- an encrypted zip is refused, never prompted for.

Known limits: named pipes (FIFOs) in an archive are extracted into the cache; the credentials
of a git source URL are stored in clear in its cache's `.git/config`, as `git clone` writes
them.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Project format](project-format.md) — the `.armadai/` files a pack holds
- [Templates](templates.md) — creating agents from templates
- [Link](link.md) — compiling a pack's agents for your assistant
