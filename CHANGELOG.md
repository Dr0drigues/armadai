## v1.0.0-rc.6 (2026-10-21)

Sixth release candidate, and the first of the declarative refocus: ArmadAI no longer runs
agents. A team is described once in `.armadai/` (canonical model, `armadai validate`,
`armadai migrate`, guided `armadai new`, `armadai graph`), then compiled by `armadai link` into
the native files of Claude Code, Codex, OpenCode and Copilot.

Breaking: the runtime (`run`, the TUI, the web dashboard, the shell, the providers), Gemini and
`armadai config providers|secrets` are removed; a legacy project is converted with
`armadai migrate` (see the migration notes of step 0 below). Releases now publish a
`SHA256SUMS` file, checked by `install.sh` and `armadai update`, which refuse a release without
one: this is the first release they accept.

### Fixed

- The GitHub release notes are no longer empty: `release.yml` piped a failing `cz changelog`
  into `awk`, which hid the error (15 releases, v0.7.0 and v0.10.0 to v1.0.0-rc.5, shipped
  without notes). The notes are now the
  tag's `## vX.Y.Z` section of this file, and the release fails when that section is missing
  or empty (#497).
- YAML read by ArmadAI no longer loses the lines after a root node that ends early: a file
  starting with a flow mapping (`{a: 1}`) or with an indented first key (` x: v`) was read
  up to that node and the rest silently dropped; it is now an error at the first line that
  does not belong to it, as with the YAML reader before #467 (serde-saphyr 2.0.0).
- `armadai config starters-dir add|remove` now edits only the `starters_dirs` key of
  `config.yaml` (other keys and comments were all lost):
  - everything outside the key's items is kept byte for byte: other keys, comments, order,
    line endings, indentation. A removed item takes its own end-of-line comment with it; a
    list left empty keeps its key as `starters_dirs: []` when it holds a comment, and loses
    the key otherwise; a missing final newline is added before an appended item and not
    taken back by a later `remove`;
  - a null (`~`, `null`) or one-line flow (`[a, b]`) value becomes a block list, keeping
    its end-of-line comment;
  - refused with the file untouched: an anchor or alias, an item or flow list spanning
    several lines, a `#` inside a quoted flow item, a flow mapping as the root, a leading
    byte order mark; a file that does not open for writing (mode, ACL, immutable flag...);
    a file of another user; a file with several hard links (break the link first: the
    atomic rename would leave the other names on the old content);
  - the write is atomic and keeps the file's mode; its owner and group are kept when the
    system allows it, otherwise the file takes the group a new file gets in its directory
    (also for the files `link` patches, which never stop over ownership). A symbolic link `config.yaml` stays one: its target is
    written, and created with its directories when it does not exist yet (40 links at
    most); no directory outside the resolved target is created (#476).

### Removed (cleanup)

- Files left over from the removed surfaces: the root `agents/` (two legacy agents), `PRD.md`,
  the `Dockerfile`, `schemas/pack.schema.json`, `docs/archive/` (recoverable from the tag
  `pre-v1-declaratif`), the `examples/orchestration-patterns/` and `examples/demo-rust-team/`
  examples (legacy format), the storage database committed in `examples/demo-nextjs-shop/`.
- The built-in `zsh-env-authoring` skill, unrelated to ArmadAI, that `armadai init` installed:
  the next `armadai init` removes the copy it had installed in `~/.config/armadai/skills/`
  (even if you edited it: copy it elsewhere first if you want to keep it).
- `examples/demo-nextjs-shop` is now a canonical project (`.armadai/` from the `pirate-crew`
  pack); its tracked `.env` becomes a placeholder-only `.env.example`.
- **Breaking:** `armadai config providers` and `armadai config secrets` (`init`, `rotate`) are
  removed, with the `armadai-providers` and `armadai-secrets` crates: nothing reads provider
  settings any more. `config` keeps only `starters-dir`. A script that calls the removed
  commands now fails with `unrecognized subcommand`.
- `armadai init` no longer writes `providers.yaml`; an existing `providers.yaml`,
  `*.sops.yaml` or `.armadai/age-key.txt` is left in place and ignored.
- The hint about a `config/settings.yaml` file (it advised `armadai init`, which migrates
  nothing) is gone; the hint for a legacy `armadai.yaml` now advises `armadai migrate`.
- Dead code of the former engine removed from `armadai-core` (agent-source loaders, the
  orchestration config validator, the `./agents` and `./config` fallbacks, unused
  dependencies `dialoguer` and `thiserror`); `armadai update` and the workspace metadata
  point at `Dr0drigues/armadai`.
- `frozen/armadai-audit` vendors the small model-registry cache reader it needed from the
  removed `armadai-providers` crate (`src/vendored/model_registry.rs`).

### Changed (archive download)

- Starter archive sources are downloaded with the `curl` program (as `armadai update` already
  did) instead of `reqwest`; `reqwest`, `tokio` and their TLS stack leave the dependency tree
  (233 → 151 locked packages). `curl` must be in `PATH`; archive URLs must be `https://`
  (redirects too, scheme case-insensitive); a download is capped at 300 s and 100 MiB. curl's
  environment applies (proxy variables, `CURL_CA_BUNDLE`; others such as `SSL_CERT_FILE`
  depend on curl's version and TLS backend), but `.curlrc` is ignored (`-q`) and URL globbing
  is off. The download goes to a private (0700) temporary directory removed afterwards.
- A starter archive is extracted into a staging directory and checked (a symbolic link
  resolving outside the archive, or dangling, is refused; it may hold at most 100,000 entries
  and 1 GiB of files) before it replaces the cached copy: a failed sync now keeps the previous
  copy instead of deleting it and leaving a partial pack that `init` would use. A cache entry
  that is a symbolic link is replaced by a directory, not written through. Temporary
  directories a killed sync left behind (over an hour old, whatever the source) are removed by
  the next archive sync, which first puts back a copy set aside for a source whose cache is
  missing. `tar`, `unzip` and `curl` run with stdin closed, and an encrypted zip fails instead
  of asking for a password on the terminal.
- Credentials are masked in starter sync messages: the `user:pass@` part and the query of a
  source URL, including in the `git` command line and error a failed clone reports, with or
  without a scheme (`user:token@host:org/repo.git` shows as `***@host:org/repo.git`). A URL
  with an `@` after its host (in the path, or an unencoded password holding `/`) is shown as
  `https://***`, since its host cannot be shown reliably.
- An archive URL with a query (`…/starters.tar.gz?token=…`) is recognised as an archive
  without `kind: archive`.
- The starter cache directory name no longer contains the URL's `user:pass@` part nor, in
  clear, its query. A plain `scheme://host/path` URL of at most 60 characters keeps its
  directory: a lowercase scheme (letters only), then letters, digits, `.`, `-` and `/`, the
  first character after `://` a letter or digit, the last one a letter, digit or `-`. Any
  other source gets a new one, named with a 128-bit SHA-256 digest, and is synced again:
  credentials, query, `#`, `@`, `_`, `~`, `%`, a port (`:`), a scheme with a capital
  (`HTTPS://`), a digit or a `+` (`git+https://`), a host starting with `.` or `-`, a URL
  ending with `/` or `.`, a local path, a `git@host:` address, a longer URL. Before, two
  such URLs could be forged to share a directory (32-bit hash), letting one source replace
  another's packs. The old directory stays on disk: delete it by hand
  (`~/.config/armadai/registry/starters/`), as its name may hold credentials.
- The starter cache skips directories whose name starts with a dot (sync staging areas).
- Known limits: an archive's named pipes (FIFOs) are extracted into the cache like files; a
  git source's credentials (`https://user:token@…`) are stored in clear in its cache
  `.git/config`, as `git clone` writes them; `git` itself may still prompt for credentials.

### Changed (project registries, #479)

- `armadai migrate` reports each project registry source it drops (`registries.<type>: "<url>"`,
  with its delivery `kind:` when declared) instead of one `registries` line: the new format has no
  project registries, so a migrated project stops using them. For the `skills` and `starters`
  registry types, a warning gives the entries to paste into the top-level lists of the user
  `registries.yaml` (its real path is printed): starter entries keep their `kind`, skill entries
  carry the URL alone (a skill source's `kind` is not read, and only GitHub sources are
  fetched, which the warning says). URLs are quoted with control characters, U+FFFE/U+FFFF and
  bidirectional characters escaped, so a pasted entry reads back as the same URL. `agents` and
  `models` sources are read by no command. An empty url is reported as such; an empty
  `registries:` section is no longer reported.

### Changed (YAML library, #467)

- **YAML is read and written with serde-saphyr** instead of serde_yaml_ng, which pulled
  `unsafe-libyaml` (RUSTSEC-2024-0449). Values read the same, with these differences:
  - a duplicated key is refused everywhere, free maps (`native`, `metadata`, `steps`,
    `outputs`) included, and reported at its second occurrence;
  - an unknown tag (`!foo`, `!env`) is refused, also in `native`;
  - `.inf`/`.nan` and number literals beyond `f64` are refused in a free (`native`) value;
  - `null`/`~` where a string is required (e.g. `description: null`) is an error instead of
    the text `null`;
  - `!!binary` is base64-decoded; mappings nested deeper than 64 levels are refused (128
    before);
  - text the previous reader kept as text still is: `0123`, `1_000`, `0X1F`, `tRuE`, and
    integers beyond 64 bits (kept as their exact text, never rounded); a typed number or
    boolean field now accepts `1_000`, `0X10`, `0b1_0` and `tRuE` (refused before);
  - long descriptions are written double-quoted instead of single-quoted.
- `config starters-dir add|remove` refuses to rewrite a `config.yaml` that does not read
  (it used to replace it with defaults, dropping the configured directories); reading an
  unreadable `config.yaml` or `registries.yaml` warns once instead of
  silently using defaults.

### Changed (step 3: guided creation, native starters)

- **`armadai new` takes a kind**: `new agent|flow|automation|skill <id>` with options
  (`--template`, `--description`, `--stack`, `--tier`, `--skills`, `--agents`, `--event`,
  `--run`, `--force`), `-i` for one kind in a terminal, and a guided menu for `armadai new` alone.
  Every creation writes one file under `.armadai/` (strict id, Windows device names refused, no
  symbolic link followed, no overwrite without `--force`), then validates the project: a creation
  that introduces an error is undone. The former `armadai new <name>` is refused with "use
  `armadai new agent <name>`".
- **Agent templates are canonical** (11, a frontmatter with a default description): description,
  tier and skills are set on the parsed frontmatter, placeholders rendered in the body only; a
  template in the legacy format is a clear error. `new` no longer goes through `migrate`.
- **Starter packs are native** (`format: 1` in `pack.yaml`): `init --pack` validates the pack,
  copies it by a white list (any symbolic link refused), keeps existing files, lists the pack's
  automations, and undoes an install that would break the project. A pack without `format` is
  still converted (legacy). `validate <pack>` validates a native pack. The 7 built-in packs are
  rewritten (descriptions saying when to use each agent, prompts become skills given to the
  agents, real flows); `armadai-authoring` and the built-in authoring skills teach the canonical
  format.
- **Entry steps**: a step no unconditional link leads to (a `rejected` or `on_failure` way back
  does not count, the `approved` end of a loop is not an entry); one definition for the validator
  and the Claude Code gate, so a review loop no longer warns `no-entry-step`.
- The gate never offers the coordinator as a delegation target (it is no sub-agent).
- `link --dry-run` lists the files it would skip and the files already up to date apart from the
  ones it would generate.
- Re-extracted embedded packs replace the previous version instead of being written over.

### Added (step 3)

- `armadai graph [flow]`: the flow(s) as Mermaid on stdout (`flowchart TD`, one diagram per flow).
- Flow templates `pipeline`, `review-loop`, `plan-build-verify`, `fan-out` (`new flow --template`).
- `new flow` warns that a flow is guidance only when the project has neither a coordinator nor a
  strict policy.

### Removed (step 3)

- The `cli-generic` agent template, the `## Pipeline` sections of the TDD templates (documented
  as a flow), the three CLI relay agents of `fullstack` (`claude-cli-reviewer`, `echo-reviewer`,
  `gemini-reviewer`), the `prompt-builder` agent and `armadai-v012-updates` of
  `armadai-authoring`, the built-in `armadai-prompt-authoring` skill (a prompt is a skill now).

### Changed (step 2: link on the canonical model)

- **`armadai link` compiles `.armadai/project.yaml`** (see `docs/wiki/link.md`): one plan per
  target, every target checked before the first write, and a **conformance report** for each
  declared element (`compiled` / `approximated` / `not-compiled`, with the reason), as text or
  with `--json` (the report alone on stdout). `--dry-run` writes nothing, stale files included.
  A legacy project is refused: "legacy format, run `armadai migrate`".
- **Claude Code adapter**: subagents (`model` from the tier, `disallowedTools` for denied
  capabilities, `skills`), skills copied byte for byte, the coordinator in a block of the root
  `CLAUDE.md` (your content kept), automations as hooks merged into `.claude/settings.json` (your
  keys kept, nothing duplicated), `native` files and keys.
- **The delegation gate reads the flows** and is installed by `link` under
  `policy.delegation: strict` (see `docs/wiki/policy-gate.md`); `free_agents` may name Claude
  Code's own agents (`Explore`, `Plan`, `general-purpose`).
- Codex, Copilot and OpenCode keep their output, fed from the model; models come from the static
  tier table, never from a prompt or the local cache.
- `unlink` undoes the patched files too, and without a manifest never touches `settings.json`
  nor `CLAUDE.md`. Files link did not write are never overwritten without `--force`, and say so
  in the report; a file link still owns is rewritten when its source changed.
- The link manifest gains `patched` and a `native` kind: an older binary reads a manifest
  holding them as unusable and falls back to its content guard.
- `link`, `migrate` and `init` create `.armadai/.gitignore` (`link-manifest.yaml`,
  `.migrated/`): `.armadai/` is now meant to be versioned.
- `outputs` values must stay inside the project, outside `.armadai/` and `.git/`
  (`bad-output`).
- `models check` / `models update` read the canonical model: the deprecated ids agents pin in
  `native.<target>.model`, replaced in place (frontmatter only). `--all` covers the projects
  `link` registers (and `--prune` keeps them); a legacy project is reported "run migrate".
- `list`, `inspect` and `validate` read `.armadai/project.yaml` only; a legacy project gets
  "legacy format, run `armadai migrate`" (exit 1). `list` shows agents, skills, flows and
  automations (`--stack` and `--global` removed: no stacks in the model, no user library).
- `init --project` writes `.armadai/project.yaml`, `agents/`, `skills/`, `flows/`,
  `automations/` and `.armadai/.gitignore`; `init --pack` creates such a project from a starter
  pack (converted like `migrate`) instead of installing the pack into the user library.
- `new` writes the agent, converted, to `.armadai/agents/<name>.md` of the current project.
- `armadai init` (user configuration) no longer creates `agents/` and `prompts/` libraries in
  `~/.config/armadai/`; `migrate` still reads an existing one for a legacy project.

### Removed (step 2)

- `link --coordinator` and `link --model` (the coordinator and the tiers are declared in the
  project), `unlink --coordinator` and `unlink --with-config` (`project.yaml` is your source, not
  link output).
- The hand-written `schemas/armadai.schema.json` of the legacy `armadai.yaml` (#456): use the
  generated `schemas/v1/`.
- **`armadai extract`**: it extracted agents, prompts and skills out of the user library, which
  the canonical model no longer has (C11).
- **`armadai prompts`**: prompts are no longer a concept of the model (C6); `migrate` folds them
  into agent bodies.

### Added (step 1: canonical model)

- **`armadai-model` crate**: the new project format (`.armadai/project.yaml`, `agents/<id>.md`,
  `skills/<id>/SKILL.md`, `flows/<id>.yaml`, `automations/<id>.yaml`) as strict types, a loader
  with positioned diagnostics, a semantic validator with stable codes, and a canonical writer.
  Pure: no other workspace crate, no CLI, async or network dependency; CI builds it standalone.
- **JSON Schemas** of every file in `schemas/v1/`, generated from the types and checked for
  staleness by a test.
- **`armadai validate`** on the new format: `file:line:column: error[code]` diagnostics, exit
  code 1 on error, `--json`. A legacy project is validated as before, plus an invitation to
  migrate.
- **`armadai migrate`** (`--path`, `--dry-run`, `--force`): converts a legacy project, backs it up
  in `.armadai/.migrated/<timestamp>/` first, and reports written files, every dropped field and
  warnings; no text of an agent is lost. YAML comments are not preserved. `link` reads the new
  format only from step 2. See `docs/wiki/project-format.md`.

### Step 0: declarative refocus

ArmadAI becomes a declarative compiler: agents, prompts and
skills are described once and compiled by `armadai link` into the native files of Claude Code,
Codex, OpenCode and Copilot. Everything that executed agents is removed. The removed code is
recoverable from commit `9b02047`. Spec:
`docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md`.

The step 0 parts below cover the refocus and the two security PRs (#436, #437) only; earlier
unreleased changes since rc.5 are not itemised here. Where a later step changed a step 0 entry,
the entry says so.

### Removed (step 0)

- **`armadai run`** and the event-sourced orchestration engine (Direct, Blackboard, Ring,
  Hierarchical, Auto), with `history`, `costs`, `projections`, `up`/`down`, `--pipe`,
  `--orchestrate`, `--resume` and `--replay` (#446, #450).
- **The conversational shell** and its setup wizard, and **the TUI** (#446).
- **The web dashboard** (#445).
- **SQLite storage** (`armadai-storage` crate) and the run event log (#446).
- **`armadai watch`** and the Workroom, the Claude Code session adapter and plugin (#445).
- **`armadai registry`** (awesome-copilot community agents) (#448).
- **Gemini CLI as a link target**: `link`/`unlink --target gemini` (#449).
- **The execution providers**: API (Anthropic, OpenAI, Google), CLI relay and proxy, rate
  limiting, and the online model-catalogue refresh (#450).
- **Cargo features** `providers-api`, `tui`, `web`, `storage` and `e2e-fake`; the
  `armadai-fake` crate (#444, #445, #446, #450).
- **The gaveldrop e2e suite** and the `run_*` integration tests (#444).
- `docker-compose.yml` (#446).
- The embedded starter pack `orchestration-demo` (a demo of the removed engine's patterns) and
  the built-in skill `armadai-shell-config` (the removed shell's configuration).
- **The response-marker protocol**: the `ARMADAI_DELEGATE` / `ARMADAI_END` / `ARMADAI_META` block
  that `link` wrote into the coordinator's instruction file (all four targets), and the built-in
  skill `armadai-protocol`. Nothing consumed the markers since the shell relay was removed.

### Changed (step 0)

- The coordinator files generated by `link` no longer ask the assistant to end every reply with a
  closing marker; the Team table and the delegation through each tool's native mechanism are
  unchanged.
- `armadai` without a subcommand prints the help and exits with code 2, instead of starting the
  shell (#446).
- `link` and `new` never touch the network (#450). Since step 2, `link` takes models from the
  static tier table only, never from the local model-catalogue cache, and `new` sets a tier, not a
  model id.
- CI runs a single configuration (one clippy, one test job) instead of nine feature-gated
  clippy/test runs.
- `audit` is removed from the binary and moved to `frozen/armadai-audit/`, a frozen crate outside
  the workspace and CI, not maintained (#447).
- The output of `link` for the four kept targets is pinned by a golden test (#443).
- The project's own agent team and instructions (`CLAUDE.md`, `.claude/`) are redefined around
  the declarative scope (#451).
- Documentation rewritten for the declarative scope; documents of the execution era moved to
  `docs/archive/` (deleted later in this release, see Removed (cleanup)).
- Content shipped in the binary aligned with the declarative scope: the `armadai-dev` pack follows
  the redefined team (`schema-specialist`, `adapter-specialist` replace `core-specialist`,
  `provider-specialist`; `ui-specialist` removed); the `armadai-authoring` pack and the built-in
  authoring skills no longer describe `run`, the shell or the engine patterns;
  `armadai-orchestration-patterns` describes the `orchestration:` block as a declaration for the
  policy gate. The top-level
  `--help` describes the declarative product. `scripts/migrate-v0-to-v1.sh` moved to
  `docs/archive/scripts/` (deleted with `docs/archive/`).
- `schemas/armadai.schema.json`: the `link.target` enum lists the supported targets (`codex`
  added, `gemini` removed), `orchestration.policy` and `orchestration.free_agents` are accepted,
  and the fields of the removed engine and shell are described as unused. The file itself was
  removed in step 2 (#456).

### Fixed (step 0)

- **`armadai new`** works out of the box: the agent templates are embedded in the binary
  (it used to fail with "no templates directory found" on a fresh install). `./templates` and
  `~/.config/armadai/templates` still take precedence over the embedded ones.
- **`armadai init --pack <pack> --project`** now writes `link.target` only when a supported
  target can be inferred (agents read in file-name order, `anthropic` mapped to `claude`),
  instead of copying an arbitrary agent provider such as `google` or `cli`; when none can be
  inferred it prints a hint and `armadai link --target <t>` is required. Superseded in step 2:
  `init --pack` creates a canonical project, whose targets come from the pack's `project.yaml`.
- **`examples/demo-nextjs-shop/DEMO.md`** step 1 no longer fails with "armadai.yaml already
  exists".

### Migration notes (step 0)

- **Files generated by an earlier `link`** still contain the `## ArmadAI Response Protocol`
  block until they are regenerated: `link` skips files that already exist, so run
  `armadai link --force` (or delete the block by hand).
- **A project linked to Gemini**: `link`/`unlink --target gemini` no longer exist. By hand,
  delete the files listed under `targets: gemini:` in `.armadai/link-manifest.yaml` with
  `outcome: created` (leave `outcome: skipped` entries, which `link` never wrote) and the
  now-empty `created_dirs`, then remove that `gemini:` block; or run `unlink --target gemini`
  with the previous version before upgrading. If `.armadai/config.yaml` sets
  `link.target: gemini`, change it to another target, or a bare `armadai link` fails. The other
  targets keep working with the manifest entry in place.
- **Model catalogue**: nothing refreshes the local catalogue cache any more. Since step 2 `link`
  no longer reads it either: tiers become models through the embedded table, whose content is
  dated (e.g. `claude-sonnet-4-5-20250929` for Anthropic Pro), and `migrate` turns `latest:*`
  into a tier.
- **Agent metadata** used only by the execution engine (`command`, `args`, `max_tokens`,
  `timeout`, `cost_limit`, `rate_limit`, `context_window`, `mode`, `## Pipeline`,
  `## Triggers`, `## Ring Config`) is still parsed, but nothing executes it. Since step 2 only
  `migrate` reads it, and drops it with a reason in its report (`inspect` and `validate` read the
  canonical model only).
- **Starters and skills extracted by earlier versions** (for example
  `~/.config/armadai/starters/orchestration-demo` or `~/.config/armadai/skills/armadai-shell-config`)
  are not removed by the update and stay usable; they describe removed features, so delete them
  by hand.
- **`~/.config/armadai/config.yaml`**: only `starters_dirs` is read. The `defaults`, `storage`,
  `rate_limits`, `costs` and `logging` keys written by earlier versions of `armadai init` are
  ignored (the file still loads), and `armadai config starters-dir add`/`remove` rewrites the file
  without them. `armadai init` now writes a commented `starters_dirs` example only.
- **Contributors**: the agent team and the project instructions were redefined (5 agents:
  `dev-lead`, `schema-specialist`, `adapter-specialist`, `cli-specialist`, `qa-specialist`);
  the build has no feature flags.

### Security

- `armadai init --pack <name>` no longer lets a project source mask one of yours (#490). A
  cloned repository could get its own pack of the same name installed: a source of its legacy
  config (`registries.starters`) whose cache folder `read_dir` listed first (5 tries out of 5),
  its `.armadai/starters/` (which outranked the built-in packs), a `starters/` folder of the
  current directory (taken for the built-in packs), a `<name>/` folder of the current
  directory (read before any source), or a relative `starters_dirs` entry (read from the
  current directory). Now:
  - your starter folders, then your `registries.yaml` sources, always come before the
    project's sources;
  - `--pack <name>` never reads the current directory: only a path (holding `/`, or starting
    with `.` or `~`, such as `./my-pack`) names a local folder;
  - a relative entry of `starters_dirs` or `ARMADAI_STARTERS_DIRS` is ignored with a warning,
    and `armadai config starters-dir add` refuses one;
  - a pack name matches folder names exactly, case included, on every file system;
  - a pack taken from a project source is announced, its URL masked and its control
    characters escaped; one found only there (or in several project sources) first syncs the
    sources, so that one of yours not synced yet still wins, and is refused when one of your
    sources fails to sync (the error names it, and gives each candidate folder as a command
    ready to paste, quoted for the shell);
  - `--pack ~/x` (quoted) is read from the home directory (`~user` is not expanded, and is
    said so); an empty or blank `--pack` is refused; a path without `pack.yaml` is reported
    as `<folder> holds no pack.yaml` instead of an unknown pack;
  - two of your sources holding the same pack is an error naming them, where one was picked
    in `read_dir` order; so are two project sources, which includes `.armadai/starters/` and
    a project registry holding the same name (`.armadai/starters/` was installed before);
  - only the caches of the sources declared now are read: a cache left by a source since
    removed, or by another project's source, is ignored.
- Listing a starter archive is bounded in memory and time (#493): its entry names are read as
  `tar`/`unzip` print them, and past 100,000 entries, a name of more than 4096 bytes or 64 MiB
  of names in total the lister is stopped and the archive refused, the message naming the
  bound. Before, the whole listing was read first: a 1 MB `.tar.gz` declaring 1,000 names of
  900 KB took 1.8 GB of memory and three minutes (debug build) before being refused.
- `armadai update` downloads with the same hardened `curl` call as starter archives (#488):
  it no longer reads `~/.curlrc` (where `insecure` would turn TLS checks off while fetching
  the new executable), stays on HTTPS through redirects, does not expand `{}`/`[]` in URLs,
  and is bounded in time and size.
- Releases publish a `SHA256SUMS` file, and both `armadai update` and `install.sh` check the
  downloaded binary against it before installing it. A release without `SHA256SUMS` (every
  release up to v1.0.0-rc.5), or without a line for the platform, is **refused**: install such
  a version by hand. This guards against a corrupted or altered download, not against a
  compromised GitHub account (the checksums come from the same place as the binaries).
- `install.sh` ignores the `~/.curlrc` of the user (`curl -q`), stays on HTTPS through
  redirects, and no longer exits in error after a successful install (its EXIT trap read a
  variable that was gone).
- `rustls` upgraded to 0.23.46 to resolve RUSTSEC-2026-0346 to 0349 (#478).
- `armadai config secrets init`, removed in this release, wrote the age private key to
  `.armadai/age-key.txt` and the generated `.armadai/.gitignore` did not exclude it. If your
  project has that file, make sure it was never committed (`git log --all -- .armadai/age-key.txt`);
  if it was, rotate the key and re-encrypt what it protected.
- `quinn-proto` upgraded to resolve RUSTSEC-2026-0338 to 0342 (#471).
- `rustls` upgraded to resolve RUSTSEC-2026-0285 (#437).
- The security audit workflow repaired (permissions and inputs) (#436).
- Removing `ratatui` drops `lru` from the dependency tree.

## v1.0.0-rc.5 (2026-07-27)

Fifth release candidate. Consolidates the reopened-scope work: the full
design-system rollout, the event-sourcing orchestration epic, real
rate-limiting, and end-to-end hierarchical delegation. Also adopts a
master-only branch model.

### Design system (final surface: Docs)

- **Docs surface** (#261): standalone brand assets (`assets/brand/`), an
  identity-forward README (fixed badges), and an mdBook site reusing
  `docs/wiki/` with a DS theme (oklch tokens, self-hosted IBM Plex) published
  to GitHub Pages. Completes the DS rollout across Web + TUI + CLI + Docs.

### Orchestration

- **OH1 Lot 6** (#264): `armadai run --resume <run_id>` (continue an
  interrupted event-sourced run) and `--replay <run_id>` (deterministic
  re-display, no effects), with the `run_id` surfaced for the CLI and the live
  Workroom. Closes the event-sourcing epic (Lots 1-6).
- **End-to-end hierarchical delegation** (#273): the coordinator prompt now
  delegates by default instead of stalling on a clarifying question, and the
  orchestrated per-call timeout is raised to 600s (with an
  `orchestration.agent_timeout_secs` override), so real delegated runs
  complete on the CLI fleet.
- **Live-run abort** (#275): Ctrl+C / q now abort a running orchestration
  cleanly (the CLI subprocess is killed on drop instead of being orphaned),
  and the Workroom footer advertises the real abort keys.

### Providers

- **Rate-limiting Lot 1** (#269): real proactive throttling via a
  `RateLimitedProvider` decorator covering every provider call (event-sourced
  engines included) — a shared per-provider limiter (`config.rate_limits`)
  plus an optional per-agent limiter (frontmatter `rate_limit`). Fixes the
  previously inert config and the `parse_rate("N/hour")` panic.

### Debts

- **Hardening** (#262): hermeticized the flaky `test_resolve_shell_model_aliases`,
  zip-slip validation for remote-starter archives, and TUI UX (unified
  quit-keys, detail-view scroll, Costs alignment).

### Project

- **master-only branch model**: dropped `develop`; `master` is the default
  trunk. `develop` was reconciled into the release line first (#248).

## v1.0.0-rc.4 (2026-07-21)

Fourth release candidate for 1.0.0. Completes the RC roadmap with remote
starter registries (B2 Lot B) and clears the post-rc.3 hygiene debts. Scope
remains frozen (event-sourcing and the declarative engine stay in 1.1/v2).

### New: B2 Lot B — Remote starter registries

- **Fetch starter packs from remote registries** (#211, #212), transport-agnostic
  behind a pluggable fetcher: **git** (clone/pull, always available) and
  **archive** (download + `tar`/`unzip` extract, gated `providers-api`). A
  `RegistrySource` declares its `kind` (git|archive), inferred from the URL or
  explicit.
- **Discovery norm (hybrid)**: any directory with a `pack.yaml` is a pack; an
  optional `armadai-starters.yaml` manifest at the registry root enriches /
  restricts what is exposed (with `..`/absolute paths rejected).
- **Integration**: synced packs join the existing local starter resolution
  (`find_pack_dir` / `load_all_packs`), with **local packs always winning** over
  the remote cache; remote packs also resolve by their `name:` (not just dir).
- **CLI**: `armadai registry sources add|remove|list starters <url>` and
  `armadai registry sync` (now also syncs starter sources, user ∪ project).
- **Auto-sync-on-miss**: `armadai init --pack <name>` syncs the configured
  starter registries when the pack is missing locally, then retries — network
  only on a miss (no starter sources ⇒ no network, behavior unchanged).

### Fixed / hardening (#210)

- **Hermetic model-tier resolution tests**: `resolve_model_for_tier` now
  always returns a concrete pinned model (never a floating `-latest` alias or
  the `latest:*` placeholder), and the affected tests are isolated from the
  ambient model catalog cache — eliminating intermittent failures.
- **Accurate CLI input-token count**: the CLI provider now sums
  `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`, so
  History/Costs no longer show a misleading near-zero "IN" for cached prompts.
- **Trace list pagination**: `get_orchestration_trace` filters to root runs at
  the SQL level (before `LIMIT`), so nested children no longer consume list
  slots.

## v1.0.0-rc.3 (2026-07-21)

Third release candidate for 1.0.0. Completes the rc.3 scope (dynamic agent
routing + an event-based shell Workroom) and adds a substantial UX pass across
both terminal UIs plus genuinely useful run history/costs. Scope remains frozen
(event-sourcing and the declarative engine stay in 1.1/v2).

### New: C8 — Declarative agent routing

- **Deterministic routing** (#187, #188): select which roster agents run a task
  via **named routes** (`orchestration.routes:`) and/or **capability tags**
  (`--tags`), on top of `--route`. New `armadai run` flags `--route`, `--tags`,
  and `--dry-run` (preview the selection — agents + reason + pattern — with zero
  tokens). Targets blackboard/ring; hierarchical keeps its own topology.

### New: Event-based shell Workroom + drill-down

- **Marker-driven state machine** (#189, #190): the shell Workroom panel is now
  driven by the ArmadAI protocol markers in the stream (`ARMADAI_DELEGATE` /
  `META` / `END`) instead of fuzzy text matching — robust to markers echoed in
  recaps, resilient to chunk-split markers. `detect_mentions` removed.
- **Drill-down** (#194): `Ctrl+W` focuses the Workroom; `j`/`k` select an agent;
  `Enter` opens a detail popup (state, elapsed, last action, transition
  timeline); `Esc`/`Ctrl+W` exit. Works between AND during a streaming turn.

### New: Useful run history & costs

- **Absolute shared DB** (#208): the default storage path is now
  `~/.local/share/armadai/armadai.sqlite` (was a CWD-relative `data/…`), so runs
  and `armadai tui` share one database regardless of directory.
- **Per-run project** (#208): schema migration v2 tags every run (sequential and
  orchestrated) with its originating project; the dashboard History shows a
  PROJECT column.
- **Real CLI cost/tokens** (#208): the CLI provider now parses the underlying
  tool's stream-json output (`total_cost_usd` + usage) instead of reporting
  `$0.00`. Agents with explicit `args:` are respected verbatim.

### Changed: UX/UI pass across both TUIs

- **Shared semantic theme + light-terminal support** (#195): a `theme.rs` with
  named (terminal-adaptive) colors; the statusbar, detail views and markdown
  rendering are now legible on light terminals (were white-on-white / dark bar).
- **Safe Esc + shortcut bar** (#196): in the shell, Esc clears the input first,
  then requires a second Esc to quit (Ctrl+C still quits immediately); a
  persistent hint bar documents the shortcuts.
- **Dashboard polish** (#197, #191): scrollable detail views, the Costs tab
  brought to list conventions (selection/search/sort), `Ctrl+C` quits, and
  consistency fixes (legible Workroom colors, centralized cost/context
  formatting, deduped search bar & spinner).

### Fixed

- **Hierarchical orchestration was broken for kebab-keyed agents** (#198): the
  engine keyed agents by their H1 title instead of the config key, so the
  coordinator lookup failed (`No provider found for 'dev-lead'`). Now keyed by
  the roster key. Same class of fix as C8's routing (#192).
- **`--dry-run` now short-circuits without `--route`/`--tags`** (#192): a plain
  `--dry-run` previews the full roster instead of executing.
- **Workroom lifecycle** (#192, #193): CLI/provider pseudo-agents (e.g.
  `claude`) filtered out; final agent states persist between turns (reset moved
  to turn start).

## v1.0.0-rc.2 (2026-07-20)

Second release candidate for 1.0.0. Consolidates the post-rc.1 work: the
OH3/OH4 orchestration instrumentation, audit refinements, the web trace UI,
custom registries, and the full **C9 pattern mixing** feature. Scope remains
frozen (event-sourcing and the declarative-engine vision stay in 1.1/v2).

### New: C9 — Pattern mixing (hierarchical → nested blackboard/ring)

- **Engine** (#183): a hierarchical team can declare a nested sub-pattern
  (`teams[].pattern: blackboard|ring`) and run its agents as that sub-pattern
  instead of flat delegation. Budget/depth are shared with the parent run,
  metrics fold back, and the team lead gets an **arbitration turn** over the
  sub-run outcome (accept/refine/override). New `NestedStart`/`NestedEnd`
  JSONL events. A dedicated `NestedPattern` enum makes deeper nesting
  impossible by construction (single level).
- **Storage** (#184): a real schema migration mechanism (`PRAGMA
  user_version`) — the missing brick — relaxes the orchestration CHECK to
  allow `hierarchical`, adds `parent_run_id`, and a `delegation_events`
  table. Hierarchical runs and their nested sub-runs are now persisted and
  linked. Legacy databases migrate without data loss (foreign keys are
  disabled only around the table rebuild).
- **Exposition** (#185): the web trace UI renders hierarchical runs — a
  delegation-tree diagram plus expandable nested sub-runs (reusing the
  existing sequence/timeline views). The trace list shows root runs only.

### New: Custom registries (B2 Lot A)

- **Configurable registries** (#182): a `registries:` section (user
  `~/.config/armadai/registries.yaml` and/or project `armadai.yaml`) adds
  custom sources for agents, skills, and models on top of the built-in
  defaults (union, dedup). New CLI: `armadai registry sources
  list|add|remove <agents|skills|models> <url>`.

### New: Web orchestration traces (C6)

- **Traces tab** (#181): the web dashboard lists orchestration runs and shows
  a run's flow (Mermaid sequence diagram + timeline) via a detail endpoint.

### Changed

- **OH3/OH4 consolidation** (#179): JSONL run events (`Delegate`/`Vote`/
  `Board`) are now emitted across the orchestration engines, and
  `model: latest:auto` tier routing applies inside orchestration
  (blackboard/ring/hierarchical), not just single-agent runs.
- **Audit refinements** (#180): post-rc backlog cleanups for `armadai audit`.

## v1.0.0-rc.1 (2026-07-17)

First release candidate for 1.0.0. Scope frozen (event-sourcing and the
declarative-engine vision are deferred to 1.1/v2).

### Fixed

- **Beta.2/beta.3 technical debt resolved** (#176): exact metrics from
  `result_event` now used in Pipeline/Tandem modes (previously
  approximated), `deprecated_model` warning now emitted consistently in
  orchestration mode (previously single-agent-only), plus assorted
  cleanups (`content_out` clone avoidance, duplicate `exit_code_for` call,
  obsolete `#[allow(dead_code)]` removal, documented lint suppressions,
  redundant `#[serde(default)]` removal in `ProjectConfig`).

### New: v0 → v1 Migration Guide

- **[`docs/wiki/migration-v0-to-v1.md`](docs/wiki/migration-v0-to-v1.md)**:
  step-by-step guide covering breaking changes for v0.x users — removal of
  `fleet`, non-canonical `provider` syntax, deprecated models, the
  `.armadai/` project format, and diagnostic tooling.
- **[`scripts/migrate-v0-to-v1.sh`](scripts/migrate-v0-to-v1.sh)**: companion
  automation script for the mechanical, deterministic parts of the
  migration. Dry-run by default (nothing is written unless `--apply` is
  passed), backs up every file it touches, and never deletes anything.

## v1.0.0-beta.3 (2026-07-17)

Third beta of the 1.0.0 release. Adds two OpenHands-study features: a
CI-first headless mode for `armadai run` and a dynamic model-tier router.

### Feat

- **[OH3] Headless CI mode for `armadai run`** (#173): new `--headless`
  (non-interactive, CI exit codes, skips the model-updater prompt), `--json`
  (structured JSONL event stream on stdout), `--quiet` (result event only),
  and `--max-content N` (truncates intermediate event content) flags.
  Introduces `RunEvent`/`EventSink` (`NullSink`/`JsonlSink`) in
  `core/events.rs`, with short JSON keys for token economy. High-level
  events — `run_start`, `agent_start`, `agent_end`, `warning`, `result`,
  `error` — are instrumented on the single-agent path, `--pipe`, and
  orchestration (blackboard/ring/hierarchical) at the per-agent level. CI
  exit codes: `0` success, `1` execution error, `2` usage error, `3` budget
  exhaustion (`Err`-propagating paths only — orchestration budget halts
  remain a graceful partial result with exit `0`), `4` provider unavailable.
- **[OH4] Dynamic model-tier router (`model: latest:auto`)** (#174): an
  agent declaring `model: latest:auto` has its tier (`Fast`/`Pro`/`Max`)
  selected at run time by zero-token static heuristics — input length,
  keyword matching, agent tags (override), and budget (cap) — then resolved
  to a concrete model via `resolve_model_for_tier`. Signal precedence:
  tag override → `max(length, keywords)` → budget cap. Defaults are
  embedded and overridable per-field via `armadai.yaml > routing:`.
  `ModelTier` is now orderable (`Fast < Pro < Max`). Emits
  `RunEvent::Route { agent, tier, reason }` in `--json` mode (OH3 synergy).
  Scoped to the single-agent/`--pipe` path in this release; orchestration
  continues to use its own `agent_model` resolution.

## v1.0.0-beta.2 (2026-07-17)

Second beta of the 1.0.0 release. Resolves the four P0 blockers from the
v1.0.0 review and integrates the RUSTSEC security fixes carried over from the
dependency syncs.

### Fixed

- **[B1] Real streaming in Pipeline and Tandem modes** (#169): progressive
  output rendering with a final drain, concurrent stderr capture that is also
  shown on failure, and removal of the previous output duplication.
- **[B2] Cursor wrapping on multi-line input** (#170): correct cursor position
  on wrapped lines using `unicode-width`, with scroll support.
- **[B3] Poisoned mutex handling in the hierarchical orchestration engine**
  (#168): recovery and `Result`-based propagation instead of panicking.
- **[B4] I/O errors are logged instead of silently ignored** (#167): failures
  are surfaced via `tracing::warn!` / `tracing::debug!`.

### Security

- **RUSTSEC**: transitive advisories in `quick-xml` and `quinn-proto` resolved
  via dependency sync bumps.

## v0.12.0 (2026-04-09)

### New: `armadai shell` — Interactive Conversational TUI

- One-shot CLI runner supporting Gemini, Claude, Aider, Codex
- Rich markdown rendering with syntax-highlighted code blocks (tui-markdown)
- Mouse scroll, input history (↑↓), popup overlays for slash commands
- Spinner animation with elapsed time during loading
- Auto-detect provider from PATH, model from config
- UTF-8 cursor handling for accented characters
- Terminal restore on panic (panic hook)

### New: Wizard Setup

- Auto-detect project config and existing links
- Interactive init with starter pack selection
- Interactive link target selection
- Provider auth verification

### New: Slash Commands

- `/help`, `/clear`, `/cost`, `/agents`, `/model`, `/history` — session management
- `/providers`, `/switch <name>` — multi-provider support
- `/sessions`, `/resume <id>`, `/save` — persistent sessions
- `/quit` — exit shell
- All commands render as popup overlays with markdown formatting

### New: Multi-Provider Support

- Switch between Gemini, Claude, Aider, Codex mid-session
- Preserves conversation history across provider switches
- Provider detection with model name and pricing display

### New: Persistent Sessions

- Auto-save after each turn to `~/.config/armadai/sessions/`
- Resume conversations with `/resume`
- JSON-based storage (no SQLite dependency)
- Relative timestamps ("2 hours ago", "yesterday")

### New: Response Protocol

- `armadai-protocol` skill with standardized markers (END, DELEGATE, META)
- Injected automatically in all linker outputs (Gemini, Claude, Copilot, Codex, OpenCode)
- Shell parser module for marker extraction

### Web UI Improvements

- Light/dark mode toggle with localStorage persistence
- Markdown rendering for system prompts, skills, prompts (marked.js)
- Download `.md` buttons on agent, prompt, skill detail views
- Orchestration config display in starter detail views
- Graceful Ctrl+C shutdown

### Dependencies

- New: `tui-markdown` 0.3 (rich markdown rendering in TUI)

## v0.11.0 (2026-04-01)

### Feat

- TUI: inline search with `/`, column sort with `s`, new Orchestration tab (key `8`)
- Web UI: search/filter bars on all tabs, Mermaid.js orchestration topology diagram
- New API endpoint: `GET /api/orchestration/topology`
- Orchestration: parallel dispatch for hierarchical delegations via `tokio::spawn`
- Orchestration: cost budget enforcement with `token_budget` and `cost_limit` (graceful halt)
- New starter pack `armadai-dev`: 7 agents + `rust-conventions` prompt
- 4 Gemini API integration tests (gated behind `GOOGLE_API_KEY`)
- Gemini CLI E2E test script with 18 assertions (`tests/gemini_cli_e2e.sh`)
- `link.rs` validation improvements, 4 orchestration pattern examples, `demo-rust-team` config
- Orchestration user guide with decision matrix (`docs/wiki/orchestration-guide.md`)
- `pack.schema.json` for starter pack config validation

### Fix

- Remove unimplemented `--replay` flag from history command
- Serialise env-mutating tests with a shared mutex to prevent race conditions
- Fix `rustfmt` formatting in `web/mod.rs` route definition

### Refactor

- Harden error handling: 33 dangerous `unwrap()` replaced
- Document all unsafe blocks with `SAFETY` comments

## v0.10.6 (2026-03-31)

### Fix

- Revert coordination-only mode, always include coordinator prompt

## v0.10.5 (2026-03-28)

### Fix

- Skip coordinator system prompt when root context file exists
- Match coordinator name against slugified agent name

## v0.10.4 (2026-03-25)

### Feat

- add coordinator name and delegation instructions to generated CLAUDE.md/GEMINI.md

## v0.10.3 (2026-03-24)

### Feat

- add interactive shell setup (PATH + completions) on init and update

### Fix

- use ValueEnum for link/unlink --target autocompletion

## v0.10.0 (2026-03-23)

### Feat

- non-hierarchical orchestration: Blackboard (shared-state parallel agents) and Ring (sequential token-passing with voting)
- task-dependent classifier for automatic pattern selection (keyword heuristics + tag overlap)
- LLM agent wrappers with structured prompts (ACTION/TARGET/CONFIDENCE/CONTENT) and graceful fallback
- SQLite persistence for orchestration runs, board entries, ring contributions, and votes
- `--orchestrate blackboard|ring` CLI flag for manual pattern override
- new agent format sections: `## Triggers` (Blackboard) and `## Ring Config` (Ring)
- project-level orchestration config via `armadai.yaml` defaults (max_rounds, thresholds, budget, etc.)
- weighted voting in Ring pattern via `vote_weight` agent config
- position similarity grouping in vote resolution (Jaccard word-overlap)

### Refactor

- remove dead `coordinator.rs` and `pipeline.rs` execution code (hub & spoke pattern preserved for `link` command)
- remove global `serde/rc` feature, replaced by local `arc_vec_serde` module
- remove `PRAGMA foreign_keys = ON` from global schema (FK constraints kept for documentation)

### Fix

- prefix matching in classifier (tag "review" matches "reviewing", "infra" matches "infrastructure")
- parser fallback to Finding/Propose when LLM omits TARGET (no silent pointer to entry 0)

## v0.9.0 (2026-03-13)

### Feat

- add `armadai models check/update/list` commands for deprecated model management
- add project auto-registration on `run` and `link` commands
- add deprecated model alias resolution with embedded YAML registry
- auto-check deprecated models on `run`, `link`, and `init --project` with interactive prompt
- consolidate `example/` into `examples/` and migrate to `.armadai/` project format

## v0.8.0 (2026-02-24)

### Feat

- add Models catalog tab in TUI (key `7`) and Web UI (`/api/models`)
- add model resolution preview in agent detail views (TUI + Web)
- add `preview_model_resolution()` for link target model preview
- add sync cache-only helpers `load_models_cached` and `load_all_providers_cached`
- dynamic `{{model}}` placeholder in templates and starter-packs

## v0.7.0 (2026-02-19)

### Feat

- add .armadai/ project directory and ARMADAI_STARTERS_DIRS env var
- add `armadai config starters-dir` subcommand (list/add/remove)
- 3-level resource resolution: .armadai/ → project root → user library
- automatic migration hint for legacy armadai.yaml projects

## v0.6.1 (2026-02-18)

### Feat

- add starter-builder agent and armadai-starter-authoring skill

## v0.6.0 (2026-02-17)

### Feat

- embedded versioning, skill references content, init from UI (#72)

## v0.5.2 (2026-02-17)

### Feat

- detail views + starters tab + reorder tabs (TUI/Web) (#71)

## v0.5.1 (2026-02-17)

### Feat

- prompts & skills in TUI/web, fix template parser, add zsh-env skill

### Fix

- suppress tracing output in TUI for malformed agent files

## v0.5.0 (2026-02-17)

### Feat

- skills support in starter packs, add armadai-authoring pack
- agent mode (guided/autonomous) with project defaults, deprecate legacy fleet

## v0.4.0 (2026-02-13)

### Feat

- built-in skills meta, linker skills+prompts integration, unlink command
- add JSON Schema for armadai.yaml with IDE support

## v0.3.0 (2026-02-13)

### Feat

- model_fallback — automatic model retry chain (#66)

## v0.2.1 (2026-02-13)

### Fix

- correct awesome-copilot registry URL (#65)

## v0.2.0 (2026-02-13)

### Feat

- Google Gemini provider, code-analysis starters, scope & completions (#64)

## v0.1.3 (2026-02-12)

### Feat

- add models.dev registry for enriched model selection
- add OpenCode linker for link command

## v0.1.2 (2026-02-12)

### Feat

- add skills registry for GitHub-based discovery (#63)

### Fix

- embed starter packs in binary for installed usage

## v0.1.1 (2026-02-12)

### Feat

- migrate storage to SQLite and CI to cross-rs (#61)

## v0.1.0 (2026-02-12)

### Feat

- coordinator agent, pirate-crew demo & linker improvements (#60)
- add awesome-copilot registry integration (#58)
- add composable skills and prompts system (#56)
- add link command to generate native AI assistant configs (#55)
- add rich armadai.yaml project config format (#46)
- rebrand to ArmadAI + centralized config with XDG resolution (#53)
- rebrand to ArmadAI with install script and self-update (#52)
- add interactive agent creation and fleet management (#43)
- add web UI dashboard for fleet management (#40)
- implement SOPS + age secret management (#39)
- abstract provider configuration with unified tool names (#38)
- shell completion, TUI fleet management UX and demo agents (#37)
- implement cost tracking, history, and streaming TUI (#31)
- implement swarm run command and rate limiter (#30)
- add Anthropic API provider and enhance CLI provider (#29)
- implement swarm new, inspect and validate commands (#27)
- implement swarm list command (#25)
- initial project scaffolding

### Fix

- **ci**: add g++ cross-compiler for aarch64 RocksDB build
- switch reqwest from native-tls to rustls-tls for cross-compilation
- replace unsound serde_yml with serde_yaml_ng and update docs (#59)

### Perf

- feature flags to speed up CI builds (#26)
