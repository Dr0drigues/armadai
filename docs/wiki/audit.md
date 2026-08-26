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
| reads | `<root>/.claude/agents/`, `<root>/.claude/skills/`, `<root>/CLAUDE.md` | `~/.claude/agents/`, `~/.claude/skills/`, `~/.claude/CLAUDE.md`, `~/.config/armadai/skills/` |
| answers | what does *this repository* declare — what a team shares and a reviewer reads | what does *this user* carry into every session, wherever they work |
| rules | every family, `U01`–`U04` included | every family **except** `U01`–`U04` |

`--global` rather than a separate `doctor` command: the flag already exists on `list`, `tui` and `web`, so this is the repository's own convention rather than a new surface to learn. It cannot be combined with a path — one says "this repository", the other "no repository at all" — and the parser refuses the combination instead of silently picking one.

### Why global scope exists

**Skills do not live in repositories.** Measured on three real projects on one machine — `armadai`, `refonte-front`, `ci-engine` — all three declare **zero** local skills, against **48** installed globally (`~/.claude/skills` = 7, `~/.config/armadai/skills` = 41). Before `--global`, `R01` and the skill half of `R04` measured a location nobody populates: they could not fire in real use. The same holds for `~/.claude/CLAUDE.md`, which is loaded on every invocation of every project and was unreachable.

### What neither scope reads, and why

Three exclusions, three different reasons. Each is printed as a `Not read:` line in the report — an asset pile the audit skips has to be a *stated* omission, or the report reads as "you have none".

- **`~/.config/armadai/registry/`** — a synced catalogue of *other people's* skills. Flagging them would be noise, and this directory is also what skewed `R01`'s original calibration: of the 461 `SKILL.md` files the threshold was derived from, **407 (88%)** came from here. Excluding it is a correctness fix, not an optimisation.
- **`~/.config/armadai/starters/`** and **`~/.config/armadai/skills-registry/`** — starter packs and a second synced catalogue. Neither holds an asset that is installed, so neither is in anyone's context.
- **`~/.config/armadai/agents/`** — ArmadAI-format agents (H1 + `## Metadata` + `## System Prompt`), not native Claude Code frontmatter. Measured on a real 77-agent library, reading them through this reverse pass produced **77 `A01` criticals** ("missing YAML frontmatter") and a non-zero exit on a perfectly healthy library. Reading them properly needs an ArmadAI-format reverse importer, which does not exist yet; until it does, the report says how many files it left alone rather than reporting zero agents.

### Which rules apply to which scope

`U01`–`U04` are the single exclusion. They correlate declarations against *one project's* Claude Code transcripts, so they have no meaning over assets that belong to no project — and in global scope the transcript scan is skipped outright rather than run and then ignored. Every other family reads the assets themselves, and a property of an asset holds wherever the asset lives.

This is deliberately a whitelist of one exclusion rather than a per-rule opt-in, and it is enforced in the rule *registry* rather than inside any rule: a rule that only ever sees `ctx.config` cannot tell which scope filled it, so the default has to be "applies".

`--propose` and `--deep` both work in global scope. In global scope `--propose` writes its pack into the **working directory** — there is no project root to write into, and writing into `$HOME` unasked would be rude.

## What gets checked

Most of the rule surface predates this page's focus and is only summarized here — see `armadai audit --help` and the rule codes printed in the report for the exhaustive list:

- **`A0x` — static asset rules.** Checks over the declared agents, skills and instructions file in isolation: unparsable files, missing descriptive fields, deprecated or unknown models, oversized prompts, duplicated content, permissive tool access, malformed skills, broken `@agent` references, plaintext secrets.
- **`C0x` — collision rules.** Checks across declared assets: name collisions, overlapping scopes, overlapping activation surfaces, double ownership of the same module, inconsistent tool restrictions.
- **`R0x` — context rightsizing.** Checks what gets front-loaded into context rather than what is declared: an oversized skill with no progressive disclosure, a path cited in the instructions file that no longer exists, and the total weight of the context loaded up front. Detailed below. `R01` and the skill half of `R04` only have real material to work on in **global** scope — see [Scopes](#scopes).
- **`D0x` — optional deep pass (`--deep`).** Sends secret-redacted prompt excerpts to an installed CLI (`claude` or `gemini`) for an LLM-driven review, layered on top of the static findings.
- **`--propose`.** Generates an installable ArmadAI pack (`.armadai-proposal/`) from the audited native configuration. In global scope it writes into the working directory rather than into `$HOME`.

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

One known limit of the total, in both scopes: `R04` counts the instructions file as it is on disk and does not follow Claude Code's `@file` imports. Measured on one real `~/.claude/CLAUDE.md`, that is 59 tokens counted against a 241-token `@`-imported file left out — so the figure is a floor, not the whole bill. It is a pre-existing limit, not one the global scope introduced, but the global instructions file is where it bites hardest, because that one is loaded on every invocation of every project.

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

`skill_token_threshold` stays configurable, so a user whose skills genuinely run large can lower it.

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

Every threshold the static rules use is tunable per project, in an optional `audit:` section of `armadai.yaml` (or `.armadai/config.yaml`; the first of the two that exists wins). A missing file, a missing section, or YAML that fails to parse all leave the defaults in place — the audit never refuses to run over its own configuration.

```yaml
audit:
  prompt_token_threshold: 4000   # A05: an agent system prompt above this is flagged
  skill_token_threshold: 4000    # R01: a SKILL.md above this with no references/ is flagged
  activation_similarity: 0.6     # C03: Jaccard similarity above which two descriptions collide
  deep_prompt_truncation: 2000   # --deep: max characters kept per excerpt sent to the LLM
  usage: true                    # scan this project's transcripts (--no-usage always wins)
```

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
