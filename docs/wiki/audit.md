# Audit

`armadai audit` scans *native* Claude Code configuration — no ArmadAI setup required — and reports on it as an adoption funnel: are the declared agents, skills and instructions internally consistent, and does what they *declare* match what Claude Code actually *ran*.

It answers that question over one of two **scopes**: a project, or the library you carry into every session.

## Usage

```bash
armadai audit [path]                 # project scope; defaults to the current directory
armadai audit --global               # global scope: your own library, no repository
armadai audit --report report.md     # write the report to a file (markdown)
armadai audit --report report.html   # ...or HTML, by extension
armadai audit --min-severity warn    # only show findings at or above this severity
armadai audit --quiet                # shortcut for --min-severity warn
armadai audit --propose              # generate an installable ArmadAI pack
armadai audit --deep                 # add an optional LLM-driven pass
```

The command exits non-zero when critical findings exist, regardless of `--min-severity` (which only filters what is displayed) and regardless of scope. See `armadai audit --help` for the exact option reference.

## Scopes

Two scopes, because they answer different questions, have different owners, and admit different rules.

| | project scope (default) | global scope (`--global`) |
|---|---|---|
| reads | `<root>/.claude/agents/`, `<root>/.claude/skills/`, `<root>/CLAUDE.md` | `~/.claude/agents/`, `~/.claude/skills/`, `~/.claude/CLAUDE.md`, `~/.config/armadai/skills/`, `~/.config/armadai/agents/` — **not** the plugin trees, see [What neither scope reads](#what-neither-scope-reads-and-why) |
| settings | `<root>/armadai.yaml` or `<root>/.armadai/config.yaml` | `~/.config/armadai/config.yaml` |
| answers | what does *this repository* declare — what a team shares and a reviewer reads | what does *this user* carry into every session, wherever they work |
| rules | every family, `U01`–`U04` included | every family **except** `U01`–`U04` |

`--global` rather than a separate `doctor` command: the flag already exists on `list`, `tui` and `web`, so this is the repository's own convention rather than a new surface to learn. It cannot be combined with a path — one says "this repository", the other "no repository at all" — and the parser refuses the combination instead of silently picking one.

`--global` needs `$HOME`, and **refuses** to run without it rather than falling back to the current directory: a global audit *is* "what lives under `~`", so with no `~` there is nothing to audit. The fallback that used to stand there reported the current repository's `.claude/` as the user's library, labelled `~/.claude` — a wrong answer indistinguishable from a right one.

### Why global scope exists

**Skills do not live in repositories.** Measured on three real projects on one machine — `armadai`, `refonte-front`, `ci-engine` — all three declare **zero** local skills, against **48** installed globally and read by this pass (`~/.claude/skills` = 7, `~/.config/armadai/skills` = 41), plus **17 more** in installed plugins that it names but does not read (see below). Before `--global`, `R01` and the skill half of `R04` measured a location nobody populates: they could not fire in real use. The same holds for `~/.claude/CLAUDE.md`, which is loaded on every invocation of every project and was unreachable.

### What neither scope reads, and why

Six directories are left unread. Five of them get a `Not read:` line in the report — an asset pile the audit skips has to be a *stated* omission, or the report reads as "you have none". The sixth (`plugins/data/`) gets none, because it holds no asset at all; the reason is below. (This arithmetic was one short until #391: a seventh, `~/.config/armadai/agents/`, was on the list. It is read now.) So the counts a global report prints are a **floor**, and the `Not read:` lines are what tell you by how much.

Under `~/.claude`:

- **`~/.claude/plugins/cache/`** — the skills of the plugins you have **installed**. These are live in the session, exactly like `~/.claude/skills`, and this is the largest omission of the two scopes: measured on one real machine, the report announced 48 skills and ~49967 tokens of front-loaded context while this directory held **17 further `SKILL.md` worth ~39177 tokens** — the stated total 44% short of what is actually loaded — and **2 of those 17 cross `R01`**. Reading them means knowing which plugins are *enabled*, which lives in Claude Code's own `installed_plugins.json` and per-plugin manifests: a plugin-aware importer, and a separate piece of work. Until it exists, the note carries the skill count so the total is visibly a floor.
- **`~/.claude/plugins/marketplaces/`** — the **catalogue** each plugin was installed *from*: every plugin on offer, not the ones in use. Same category as `registry/` below, and it gets its own line rather than being folded into one `~/.claude/plugins` note, because installed-versus-catalogue is exactly the distinction this pass draws under `~/.config/armadai` (`skills/` read, `registry/` excluded). It carries no count: it holds git checkouts, and counting it would mean walking their object stores.
- **`~/.claude/plugins/data/`** gets no line at all. It is Claude Code's per-plugin *writable state* — measured on the same machine: 5 directories, 0 files, 0 `SKILL.md`. Nothing there is an agentic asset, and a note about a directory that holds none would be a lecture, not a fact.

Under `~/.config/armadai`:

- **`~/.config/armadai/registry/`** — a synced catalogue of *other people's* skills. Flagging them would be noise, and this directory is also what skewed `R01`'s original calibration: of the 461 `SKILL.md` files the threshold was derived from, **407 (88%)** came from here. Excluding it is a correctness fix, not an optimisation.
- **`~/.config/armadai/starters/`** and **`~/.config/armadai/skills-registry/`** — starter packs and a second synced catalogue. Neither holds an asset that is installed, so neither is in anyone's context.
The exclusions are **structural**, not a blacklist: only `<config>/skills` and `<config>/agents` are ever read, so a redirected `$ARMADAI_CONFIG_DIR` moves them with it and a skill of your own that happens to be *named* `registry` is read normally.

`~/.config/armadai/agents/` used to be on this list, and is not any more — it is read, through a second reader. See [Two agent formats](#two-agent-formats).

### Two agent formats

Global scope reads agents from two roots that hold two different formats, and each gets the reader that belongs to it:

| root | format | reader |
|---|---|---|
| `~/.claude/agents/` | native Claude Code: YAML frontmatter (`name`, `description`, `tools`) | `audit::reverse::claude` |
| `~/.config/armadai/agents/` | ArmadAI: `# H1` + `## Metadata` + `## System Prompt`, no frontmatter at all | `audit::reverse::armadai`, which calls the product's own `parse_agent_file` |

The second reader is an **adapter, not a parser**. `armadai_core::parser::parse_agent_file` is what `run`, `link` and `list` already go through, so the audit sees exactly what the product sees, and whatever that parser refuses is a file `armadai run` cannot load either — a real `A01`, not an artefact of the reader. (Measured: pointing the *Claude Code* reader at a healthy 77-agent library produced **77 `A01` criticals** and a non-zero exit, which is why the directory was excluded before #391.)

Three mappings are decisions rather than plumbing:

- **The agent's name is its file stem**, not its H1. `resolve_agent` looks up `<dir>/<name>.md`, so the stem is what `armadai run <name>` accepts and what an `@mention` resolves against. Measured: on 6 of the 77 agents the H1 slugifies to something else (`gravitee-am-app-manager.md` is titled *Gravitee AM Application Manager*), so the two are not interchangeable.
- **The description is the one `armadai link` publishes.** `AgentMetadata` has no `description` field; what ArmadAI publishes as one is the first non-empty line of the system prompt, because that is what `LinkAgent` derives and what `link` writes as `description:` into the generated `.claude/agents/<slug>.md`. The audit reuses that derivation rather than inventing a second one, so it judges the description a router will actually see. Adding a `description` field to the domain type was rejected: on day one it would be absent from all 77 files, and `A02` would flag every one of them — the exact noise the exclusion existed to avoid.
- **The prompt measured is the whole body**, in the order a linked config lays it out: `## System Prompt`, then `## Instructions`, `## Output Format`, `## Context`. Measured: 60 of the 77 agents carry `## Instructions` and 44 carry `## Output Format`, so counting only the first section understates most of the library. It sees exactly what `link` and `run` see — the point of reusing the product parser rather than writing a second one. Until [#392](https://github.com/Dr0drigues/armadai/issues/392) was fixed, that was less than the files held: a `###` sub-heading truncated the rest of its section. These rules now read **3.01x** more text than when this adapter was written (16 205 -> 48 778 estimated tokens across the 76 parsable agents).

And two rules are **not applicable** to this format, because the format cannot express what they measure:

| rule | verdict on an ArmadAI agent |
|---|---|
| `A01` unparsable | **applies** — via the product parser, so the message names the real defect (a missing `## Metadata`, not a missing frontmatter), and so does the suggestion |
| `A02` missing fields | **applies**, on the derived description. It fires when `## System Prompt` is empty, which is the only way this format can leave a router nothing to match on |
| `A03`/`A04` model | **apply** — `## Metadata` carries `model:` |
| `A05` oversized prompt | **applies**, on the whole body |
| `A06`/`A07` duplication, redundancy | **apply**. `A07` compares descriptions, so it only works *because* of the derivation above |
| `A08` permissive tools | **not applicable** — no tool list exists in this format. Reporting it anyway printed `76/76 parsed agents inherit all tools` on a real library, and one native agent among them would have turned that Info into a fleet-wide Warning: a finding about the reader, not the fleet |
| `A11` plaintext secret | **applies**, over the whole body |
| `A12` non-standard frontmatter | **not applicable** — `extra` holds Claude Code frontmatter keys, and routing `## Metadata` keys into it would announce `provider`, `model` and `tags` across the whole library |
| `C01` name collision | **applies**, on the stem |
| `C02`/`C05` path scopes | **not applicable**. They read Claude Code's non-standard `paths:` frontmatter; ArmadAI's `- scope:` looks similar but is a per-project routing hint, and a global library holds agents for unrelated repositories. Measured: feeding `scope` in yields 149 overlapping pairs across the 77 — one cluster naming 31 agents, and no conflict at all |
| `C03` activation overlap | **applies** (agent↔skill only; agent↔agent is `A07`'s turf) |
| `C04` double ownership | **applies** — it reads the instructions file against the known agent names |
| `R01`/`R02`/`R04` | unaffected: they measure skills and the instructions file |

Measured end to end on the 77-agent library that motivated this: **1 `A01`** (a real `armadai new` stub that was never filled in and that `armadai run` cannot load), **2 `A06`** Warning findings on genuinely shared blocks, and **3 `A07`** Info findings on Java/Node twin agents. Nothing else — which is the point: a rule that fires 77 times on a healthy library is not a finding, it is a reader bug.

**Project scope does not read ArmadAI agents yet.** It has no `Not read:` line to break — project scope states no omissions at all — and the surface is three sub-surfaces rather than one: `.armadai/agents/*.md`, the legacy bare `agents/*.md` (a directory name generic enough that pointing an agent parser at it would produce `A01`s in repositories that have nothing to do with ArmadAI), and the declarative `.armadai/agents.yaml`, whose loader needs the project config and the prompt fragments. Each needs its own decision.

### Which rules apply to which scope

`U01`–`U04` are the single exclusion. They correlate declarations against *one project's* Claude Code transcripts, so they have no meaning over assets that belong to no project — and in global scope the transcript scan is skipped outright rather than run and then ignored. Every other family reads the assets themselves, and a property of an asset holds wherever the asset lives.

This is deliberately a whitelist of one exclusion rather than a per-rule opt-in, and it is enforced in the rule *registry* rather than inside any rule: a rule that only ever sees `ctx.config` cannot tell which scope filled it, so the default has to be "applies".

`--propose` and `--deep` both work in global scope. In global scope `--propose` writes its pack into the **current directory, whatever that is** — there is no project root to write into, so the pack follows where you stand, exactly as it does in project scope. If you stand in `$HOME`, that is where it lands.

`--deep` in global scope widens what leaves the machine: the excerpts it sends are your own `~/.claude/CLAUDE.md` and the prompts of your global agents — personal material rather than a repository's shared config. It is the same privacy boundary that kept `R03` out of the rule set, and the command's `--deep` warning names the scope for that reason.

## What gets checked

Most of the rule surface predates this page's focus and is only summarized here — see `armadai audit --help` and the rule codes printed in the report for the exhaustive list:

- **`A0x` — static asset rules.** Checks over the declared agents, skills and instructions file in isolation: unparsable files, missing descriptive fields, deprecated or unknown models, oversized prompts, duplicated content, permissive tool access, malformed skills, broken `@agent` references, plaintext secrets.
- **`C0x` — collision rules.** Checks across declared assets: name collisions, overlapping scopes, overlapping activation surfaces, double ownership of the same module, inconsistent tool restrictions.
- **`R0x` — context rightsizing.** Checks what gets front-loaded into context rather than what is declared: an oversized skill with no progressive disclosure, a path cited in the instructions file that no longer exists, and the total weight of the context loaded up front. Detailed below. `R01` and the skill half of `R04` only have real material to work on in **global** scope — see [Scopes](#scopes).
- **`D0x` — optional deep pass (`--deep`).** Sends secret-redacted prompt excerpts to an installed CLI (`claude` or `gemini`) for an LLM-driven review, layered on top of the static findings.
- **`--propose`.** Generates an installable ArmadAI pack (`.armadai-proposal/`) from the audited native configuration. In global scope it writes into the current directory, there being no project root to write into.

The rest of this page covers the two newer halves: **context rightsizing**, rules `R01`, `R02` and `R04`, and **observed usage**, rules `U01`–`U04`.

## Context rightsizing

The `A`, `C` and `U` families ask *is it declared?* and *does it run?*. The `R` family asks a third question none of the others does: **is it sized correctly?** — what gets spent on context before anyone has typed a prompt.

### Rules

| Rule | Severity | Detects |
|---|---|---|
| `R01` | Warning | A `SKILL.md` whose estimated size exceeds `audit.skill_token_threshold` **and** whose skill directory has no `references/` at all. Both conditions are required — see below. |
| `R02` | Warning | A repo path cited in backticks in the root instructions file that resolves to nothing. The counterpart of `A10`, which does the same for `@agent` mentions. |
| `R04` | Info | The total front-loaded context — the instructions file plus every `SKILL.md` — reported without judgement, on the model of `U04`. No threshold, no suggestion. |

`R03` was designed and dropped. It would have flagged a lesson duplicated between a skill and the user's personal memory; that memory lives under the user's own directory. It stays dropped now that a global scope exists, and the tension is worth stating rather than hiding: a global audit sits closer to that boundary than a project one, so if `R03` is ever revisited it belongs in the global scope — never in the project one.

### What "front-loaded" means, exactly

The Agent Skills standard loads a skill at **three** levels, and the distinction is the whole point of `R01`:

1. **metadata** (name and description) is in context always — it is what lets the model decide whether the skill applies;
2. the **body of `SKILL.md`** enters context when the skill *triggers*. Not on every invocation — but whole, because there is no partial read, so its size is a cost the author commits to at that moment;
3. **bundled files** (`references/`, scripts, templates) load only when something asks for them.

Only the root instructions file is loaded unconditionally, on every invocation. `R04` words its message that way and counts each skill body in full, because level 2 is all or nothing; `references/` are excluded, because level 3 is precisely what splitting them out buys.

So `R01` does not say "this skill is too big". It says "this skill is big *and* has nothing at level 3" — and `armadai` already installs `references/` (`crates/armadai-core/src/skill.rs`), so the fix asks the author to use a mechanism that exists rather than invent one.

`R04`'s total is a **floor**, for two reasons, and both are stated in the report rather than hidden. In global scope, the skills of installed plugins are counted by no one: measured on one machine, `~/.claude/plugins/cache` held 17 further `SKILL.md` worth ~39177 tokens against a reported total of ~49967 — 44% short — which is why that directory gets a `Not read:` line carrying its skill count (see [What neither scope reads](#what-neither-scope-reads-and-why)).

The second limit holds in both scopes: `R04` counts the instructions file as it is on disk and does not follow Claude Code's `@file` imports. Measured on one real `~/.claude/CLAUDE.md`, that is 59 tokens counted against a 241-token `@`-imported file left out — so the figure is a floor, not the whole bill. It is a pre-existing limit, not one the global scope introduced, but the global instructions file is where it bites hardest, because that one is loaded on every invocation of every project.

### Where the default threshold comes from

`audit.skill_token_threshold` defaults to **4000** estimated tokens, and it is a **context budget**, not a quantile: 4000 tokens is the point past which "the whole body enters context the moment this skill triggers" stops being a detail and becomes a cost worth naming. That claim is true regardless of what anyone else's skills look like, which is exactly why it is the one worth making.

It used to be justified as the p90 of a measured corpus, and that justification was wrong twice over.

First, the corpus. The 461 `SKILL.md` files it was measured on were **88% `~/.config/armadai/registry`** — a synced catalogue of other people's skills that no scope audits (see [What neither scope reads](#what-neither-scope-reads-and-why)). A threshold for *your* skills was set by 407 files that were never yours.

Second, the method. On the corpus that is actually auditable — the 48 skills installed on the same machine — the distribution in tokens is:

```
n=48  min=0  median=610  p75=874  p90=2456  max=10087
```

So 4000 sits *above* the p90 of what will really be audited. But recalibrating to 2456 changes nothing observable: measured over those same 48 skills, the rule flags **1 (2%)** at 4000, and **1 (2%)** at 2456 too — only 4 skills exceed 2456 and 3 of them already carry a `references/`. The threshold is not what makes `R01` narrow; the `references/` condition is. And 48 samples are far too thin a base to derive a precise threshold from in the first place.

So the honest reporting is **1 of 48 installed skills (2%)**. An earlier version of this page claimed 4.3%, derived from that 88%-catalogue corpus — a documented number the code does not deliver, which is precisely the class of defect this project has had to fix before.

`skill_token_threshold` stays configurable, so a user whose skills genuinely run large can lower it — in global scope, where `R01` actually has material to work on, that means `audit.skill_token_threshold` in `~/.config/armadai/config.yaml` (see [Settings](#settings)).

### Why `references/` is the second condition

Both conditions are required for a definitional reason, not a statistical one: a skill that already has a `references/` directory has used the mechanism `R01`'s own suggestion would recommend, so flagging it would be advice with no action attached. The largest skill on the measured machine (10087 tokens, split into references) is big and correctly structured, and must never be flagged.

An earlier version of this page argued it statistically instead — "52% of the skills above p90 carry a `references/` against 32% below it, so large skills are *more* often split". That was the catalogue-heavy corpus again. Re-measured on the auditable 48, the contrast disappears: **75%** above the p90 have `references/` against **77%** at or below it, on 4 samples and 44 respectively. There is no signal there, and a 4-sample rate was never one. The rule stands; that argument for it does not.

### `R02`'s known blind spots

`R02` reads prose, so it is a heuristic and its filters *are* the rule: a citation must sit inside backticks, outside a fenced code block, carry both a directory part and a real source extension, and not look like a placeholder, a glob or a template (`{userData}/config.json`, `$HOME/x.yaml`, `%APPDATA%/x.json`, `[version]/notes.md`, `:owner/:repo.md` — all measured or observed forms, the first on a real project's `CLAUDE.md`). On this repository's own `CLAUDE.md`, the naive form — any backticked token containing `/` — reported 23 paths, 22 of them false.

A citation resolves either under the instructions file's own directory or as a whole-component suffix anywhere deeper in the tree, because a multi-crate repo cites modules relative to their crate (`cli/mod.rs`, not `crates/armadai/src/cli/mod.rs`). Measured against this repository's pre-rewrite `CLAUDE.md`, suffix resolution reported 16 stale paths with no crate-prefix false positive where root-only resolution reported 24 with 9 false.

Limits are known, and documented rather than hidden — a rule whose blind spots are written down survives its first false positive; one that claims to be exact does not. The first is the largest by far:

- **Only eight extensions are recognised**, and everything else is dropped in silence: `rs`, `toml`, `md`, `yaml`, `yml`, `json`, `sh`, `ts`. Measured: `scripts/deploy.py`, `web/ui/src/App.svelte`, `src/main.go`, `pkg/mod/x.java` and `styles/app.css` are all rejected, so in a project written in Python, Go, Java, Svelte or CSS **`R02` is inert**. That is a false negative and it is deliberate — each extension added is a new false-positive surface, and these eight are the only ones whose false-positive rate has been measured. If you audit such a project and `R02` never fires, this is why.
- **A real path containing a template character is silenced with the templates.** The framework dynamic-route form `app/[id]/route.ts` reads exactly like the `[version]/notes.md` metavariable the placeholder filter exists to drop. Taken knowingly: the alternative was a measured false positive on `{userData}/config.json`.
- **An unclosed backtick silences its line.** Every span after it pairs with the wrong delimiter, so code reads as prose and prose as code. The direction is the safe one: it loses a real citation (a false negative) rather than inventing one, and guarding on backtick parity would cost recall without buying precision.
- **Only backtick fences are tracked.** A `~~~` fence, or a code block indented by four spaces, is read as prose, so a path cited inside one can be reported. Compared against a tracker handling all three over the 234 markdown files of this repository, the difference is **0 candidates** — a constructed risk, not an observed one, so the simpler tracker stays.
- **Build output is not indexed** (`.git`, `target`, `node_modules`, `dist`, `build`, `.venv`). A real file under one of those, cited *relative to its crate* rather than to the audited root, is reported as nonexistent, since only the index resolves that form. Accepted: build output holds no path a human cites, and the same skip is what keeps `target/` from dwarfing the index.
- **A path differing only in case is not detected** on a case-insensitive filesystem (macOS by default): `exists()` answers `true` where a human reader would not. Another false negative, and a platform-dependent one.
- **A path cited as a convention can be reported.** `.armadai/agents.yaml` named in a repository that ships no example of one is a true statement about a naming convention, not a claim about a location — and nothing short of reading the sentence tells the two apart.

## Settings

Every threshold the static rules use is tunable, in an optional `audit:` section:

```yaml
audit:
  prompt_token_threshold: 4000   # A05: an agent system prompt above this is flagged
  skill_token_threshold: 4000    # R01: a SKILL.md above this with no references/ is flagged
  activation_similarity: 0.6     # C03: Jaccard similarity above which two descriptions collide
  deep_prompt_truncation: 2000   # --deep: max characters kept per excerpt sent to the LLM
  usage: true                    # scan this project's transcripts (--no-usage always wins)
```

**The settings follow the audited surface, not the directory you typed the command in.**

| scope | reads |
|---|---|
| project (default) | `<root>/armadai.yaml`, else `<root>/.armadai/config.yaml` — the first of the two that *exists* wins, whether or not it carries an `audit:` section |
| global (`--global`) | `~/.config/armadai/config.yaml` — the user-level config, next to the library it configures |

A global audit reads one fixed set of assets, so it has to reach one fixed verdict. Sourcing its thresholds from the working directory made that false: measured on one machine, the same global library reported **2 `R01` warnings** from a folder carrying `skill_token_threshold: 5` and **0** from a neutral one. `~/.config/armadai/config.yaml` already exists and holds the rest of the user-level configuration, so `audit:` simply joins it there.

A missing file, a missing section, or YAML that fails to parse all leave the defaults in place — the audit never refuses to run over its own configuration.

## Observed usage

Project scope only (see [Scopes](#scopes)): `--global` never scans transcripts, so this whole section and rules `U01`-`U04` are absent from a global report.

Beyond what a project declares, `armadai audit` also measures what it actually *ran*, by scanning the project's Claude Code transcripts. A project with no transcripts at all is not an error: the audit still runs to completion, simply without an "Observed usage" section and without any `U0x` finding — the same report you'd get before this feature existed.

### Discovery

Claude Code writes one JSONL transcript per session under `~/.claude/projects/<slug>/`, where `<slug>` is the project's absolute path with every path separator replaced by `-` (e.g. `/Users/x/work/proj` → `-Users-x-work-proj`).

Resolution is two-tier:

1. try the slug directly;
2. if that directory doesn't exist, scan every directory under the projects root and keep the ones whose transcript entries declare the audited project as their `cwd`.

The `cwd` field recorded in each transcript is authoritative; the slug is only an access shortcut, because its exact encoding of dots, underscores and spaces isn't publicly documented.

Internally, only absolute path forms are ever matched against — but that's transparent to you: a relative path you pass (e.g. `armadai audit .`) is canonicalized to an absolute form before matching, so it resolves exactly as expected. Relative forms are never matched *as given*, because that used to actively misfire: a bare `.` made the slug lookup collapse onto the projects root directory itself, before the canonical absolute form — the only one that could ever legitimately match — got a chance to try.

Set `ARMADAI_CLAUDE_PROJECTS_DIR` to point at a different projects root. It exists for the test suite, and for auditing a corpus of transcripts stored elsewhere.

### Opting out

This pass reads this project's own transcript history under `~/.claude/projects/`; that data is only ever read from and aggregated on this machine, never sent anywhere (the *contents* of `--deep`-selected finding messages are the exception — see below). To skip it entirely: pass `--no-usage` on the command line, or set `audit.usage: false` in `armadai.yaml` / `.armadai/config.yaml`. The flag always wins when both are set. Either way, `usage` stays unset and the report looks exactly like it did before this feature existed.

### Scan

The scan streams every transcript line by line rather than loading files into memory — a full scan of a project with a few hundred megabytes of transcripts across dozens of files completes in about a second or two, which is why there is no `--since` flag to restrict the window.

Each line is parsed once. The scan never fails on bad data: an unreadable file is skipped, a malformed line is skipped, and a missing field only degrades the metric it feeds — never the rest of the scan.

### Two metrics, not one

- **Skills** are measured in **attributed turns** (the transcript's `attributionSkill` field) — how many turns a skill actually governed, not how many times it was invoked. A single invocation can end up governing many subsequent turns, so the two numbers can diverge sharply; turns governed is the more honest measure of how much a skill actually shaped a session.
  Turns taken inside sub-agents count too, and they dominate: on this project they raised one
  skill's total from 296 to 1931.
- **Agents** are measured in both **invocations** and **turns**. An invocation is one
  delegation; a turn is one assistant step the agent actually took. Claude Code records
  sub-agents under `<session-id>/subagents/`, sometimes nested one level further
  (`subagents/workflows/wf_<id>/`), each with a `.meta.json` carrying `agentType`,
  `parentAgentId` and `spawnDepth`. A turn is one assistant *message*, deduplicated on its
  id — Claude Code writes one entry per content block, so counting entries would
  over-report by about half. The two numbers diverge sharply —
  on this project one agent shows 286 invocations for over 16,000 turns — so reading
  invocations as a measure of work done would badly mislead.
- Because that metadata states the parent and the depth outright, the observed **delegation
  depth** is reported as a fact rather than inferred from a chain of message identifiers.
- A sub-agent that actually ran appears to always leave a `.meta.json`, and a delegation
  refused before it started leaves none — so these counts look like executions rather than
  attempts. That is an observation, not a guarantee: the two files are written separately,
  with no atomicity, and this on-disk layout is an undocumented internal detail of one
  Claude Code version.

### Rules

| Rule | Severity | Detects |
|---|---|---|
| `U01` | Warning | A declared agent or skill that never ran across the observed sessions. |
| `U02` | Info | A sub-agent that ran but is not declared in this project's `.claude/agents/` — including Claude Code's own built-ins (`general-purpose`, `Explore`, `Plan`). A non-built-in name may instead come from a plugin, which is outside what this check can see. ArmadAI has no implicit equivalent for the built-ins, which makes this the rule that matters most for migration: ignoring it loses the actual workers. |
| `U03` | Warning | The root instructions name a coordinator that delegations bypass in practice (its observed share of delegations falls below half). |
| `U04` | Info | Activity of a declared skill: how many turns it governed across the scanned sessions, reported without judgement. |

All four rules are silent when nothing was observed for a project: absence of measurement is never treated as evidence of absence of use.

`U03` only fires when exactly one declared agent unambiguously qualifies as "the coordinator" — its `@name` mention must sit on a line that also carries delegation or coordination wording, with a word boundary around the name. If more than one declared agent qualifies, the rule stays silent, because accusing the wrong agent of being bypassed is worse than saying nothing.

### Report output

When transcripts are found, the terminal output and both report formats (`--report out.md` and `--report out.html`) gain an "Observed usage" section — sessions scanned, the observed time window, agents by invocation and skills by attributed turns — printed before any finding. Each of the two invocation/turn lists is truncated to the top 10; an "(+N others)" line says when more were hidden.

`--deep` additionally sends every finding's message — including any U01-U04 message produced by this pass — to the selected LLM CLI as part of its analysis payload.

### Assumed limits

- **Ring and Blackboard orchestration are not inferable from a transcript, and are never proposed.** Claude Code's delegation model is a tree-shaped call/return: there is no cycle to observe for a Ring, and no shared, concurrently-written blackboard to observe either. Guessing either pattern from the shape of an observed tree would be invention dressed up as measurement.
- **A parallel fan-out can't always be attributed precisely.** When one assistant message opens several agents at once, a later delegation nested inside one of them cannot be traced back to that specific sibling — the transcript carries no link from a sub-agent's entry back to the `tool_use` that spawned it. When that happens, the delegation is attributed to the root rather than guessing which sibling it belongs to.

## See Also

- [Getting Started](getting-started.md) — installation and first steps
- [Agent Format](agent-format.md) — agent Markdown file reference
- [Migration v0 → v1](migration-v0-to-v1.md) — mentions `armadai audit` as a complement once migration off the legacy format is done
