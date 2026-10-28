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
   `flows/`, `automations/`) into a pack folder and add `pack.yaml` with `format: 1`.
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
pack found in your own folders or sources never triggers a sync; one found only in a project
source does, so that one of your sources not synced yet still wins.

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
