# Audit

`armadai audit` scans a project's *native* Claude Code configuration — no ArmadAI setup required — and reports on it as an adoption funnel: is `.claude/agents/`, `.claude/skills/` and `CLAUDE.md` internally consistent, and does what they *declare* match what Claude Code actually *ran*.

## Usage

```bash
armadai audit [path]                 # defaults to the current directory
armadai audit --report report.md     # write the report to a file (markdown)
armadai audit --report report.html   # ...or HTML, by extension
armadai audit --min-severity warn    # only show findings at or above this severity
armadai audit --quiet                # shortcut for --min-severity warn
armadai audit --propose              # generate an installable ArmadAI pack
armadai audit --deep                 # add an optional LLM-driven pass
```

The command exits non-zero when critical findings exist, regardless of `--min-severity` (which only filters what is displayed). See `armadai audit --help` for the exact option reference.

## What gets checked

Most of the rule surface predates this page's focus and is only summarized here — see `armadai audit --help` and the rule codes printed in the report for the exhaustive list:

- **`A0x` — static asset rules.** Checks over the declared `.claude/agents/`, `.claude/skills/` and `CLAUDE.md` in isolation: unparsable files, missing descriptive fields, deprecated or unknown models, oversized prompts, duplicated content, permissive tool access, malformed skills, broken `@agent` references, plaintext secrets.
- **`C0x` — collision rules.** Checks across declared assets: name collisions, overlapping scopes, overlapping activation surfaces, double ownership of the same module, inconsistent tool restrictions.
- **`R0x` — context rightsizing.** Checks what the project front-loads into context rather than what it declares: an oversized skill with no progressive disclosure, a path cited in the instructions file that no longer exists, and the total weight of the context a project loads up front. Detailed below.
- **`D0x` — optional deep pass (`--deep`).** Sends secret-redacted prompt excerpts to an installed CLI (`claude` or `gemini`) for an LLM-driven review, layered on top of the static findings.
- **`--propose`.** Generates an installable ArmadAI pack (`.armadai-proposal/`) from the audited native configuration.

The rest of this page covers the two newer halves: **context rightsizing**, rules `R01`, `R02` and `R04`, and **observed usage**, rules `U01`–`U04`.

## Context rightsizing

The `A`, `C` and `U` families ask *is it declared?* and *does it run?*. The `R` family asks a third question none of the others does: **is it sized correctly?** — what a project spends on context before anyone has typed a prompt.

### Rules

| Rule | Severity | Detects |
|---|---|---|
| `R01` | Warning | A `SKILL.md` whose estimated size exceeds `audit.skill_token_threshold` **and** whose skill directory has no `references/` at all. Both conditions are required — see below. |
| `R02` | Warning | A repo path cited in backticks in the root instructions file that resolves to nothing. The counterpart of `A10`, which does the same for `@agent` mentions. |
| `R04` | Info | The total front-loaded context — the instructions file plus every `SKILL.md` — reported without judgement, on the model of `U04`. No threshold, no suggestion. |

`R03` was designed and dropped. It would have flagged a lesson duplicated between a skill and the user's personal memory; that memory lives outside the project, under the user's own directory. `armadai audit` is project-scoped and must not read it.

### What "front-loaded" means, exactly

The Agent Skills standard loads a skill at **three** levels, and the distinction is the whole point of `R01`:

1. **metadata** (name and description) is in context always — it is what lets the model decide whether the skill applies;
2. the **body of `SKILL.md`** enters context when the skill *triggers*. Not on every invocation — but whole, because there is no partial read, so its size is a cost the author commits to at that moment;
3. **bundled files** (`references/`, scripts, templates) load only when something asks for them.

Only the root instructions file is loaded unconditionally, on every invocation. `R04` words its message that way and counts each skill body in full, because level 2 is all or nothing; `references/` are excluded, because level 3 is precisely what splitting them out buys.

So `R01` does not say "this skill is too big". It says "this skill is big *and* has nothing at level 3" — and `armadai` already installs `references/` (`crates/armadai-core/src/skill.rs`), so the fix asks the author to use a mechanism that exists rather than invent one.

### Where the default threshold comes from

`audit.skill_token_threshold` defaults to **4000** estimated tokens. That number is derived, not picked: 460 real `SKILL.md` files measured on one machine give `min=3 · median=746 · p75=1343 · p90=2224 · max=41795` words — and the *same corpus* measures **1.84 tokens per word** (median) under the `chars/4` estimate the audit uses everywhere. So the p90 is ≈4100 tokens; read directly in tokens rather than converted, the p90 of the same distribution is 3956. Either way the answer rounds to 4000.

The rule first shipped with 3000, from assuming one token per word. The corpus says 1.84, so the converted threshold came out **36% low** — and the consequence was measurable, not theoretical: run over those same 460 skills, the 3000 default flagged **54 (11.7%)** where the p90 criterion promises ~4.6%. The criterion was right; only its conversion was wrong.

Size alone would be the wrong signal, and the same corpus says why: **52%** of the skills above p90 carry a `references/` directory against **32%** below it — large skills are *more* often split, not less. Hence two conditions rather than one. The largest skill in that corpus (41795 words, 16 references) is big and correctly structured, and must never be flagged.

The intersection is deliberately narrow, and this is where the threshold is judged rather than argued. The design target was the 21 skills of 460 (4.6%) that exceed the p90 with no `references/` at all. Run through the real command over those same 460 skills, the shipped rule at the 4000 default flags **20 (4.3%)**: of the 44 skills above the threshold, 24 are already split. Target met — the right order of magnitude for a warning, neither noise nor decoration. At 3000 the same run flagged 54 (11.7%), 2.5× the promise.

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
