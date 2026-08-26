# Audit scopes: project vs global — design

**Issue:** #389 · **Date:** 2026-08-26 · **Status:** design, awaiting approval

## Problem

`armadai audit` is project-scoped: `import_surfaces(root)` reads the repo's `.claude/agents/`,
`.claude/skills/` and `CLAUDE.md`. Skills do not live in repos.

Measured on three real projects on this machine — armadai, refonte-front, ci-engine — all three
have **zero** local skills, against **47** installed globally (`~/.claude/skills` = 7,
`~/.config/armadai/skills` = 40). So `R01`, delivered by #386, **never fires in real use**. It
measures a location nobody populates.

Worse, its threshold was calibrated on a corpus that is 88% out of scope:

| source of the 460 calibration files | count |
|---|---|
| `~/.config/armadai/registry` (synced catalogue) | **407 (88%)** |
| installed skills | 47 |
| starters | 6 |

The catalogue is other people's skills. Nobody should audit those, and they should never have
set a threshold.

## Design

Two scopes, because they answer different questions, have different owners, and admit
different rules.

**Project scope** (today's behaviour, unchanged default) — what *this repo* declares. It is what
a team shares and a reviewer reads.

**Global scope** (`armadai audit --global`) — what *this user* carries into every session:
`~/.claude/skills`, `~/.config/armadai/skills`, `~/.claude/agents`,
`~/.config/armadai/agents` (**77 entries here**), and `~/.claude/CLAUDE.md`, which exists and
is currently unreachable.

`--global` rather than a new `doctor` command: the flag already exists on `list`, `tui` and
`web`, so this is the repo's own convention. No new surface to learn.

### The catalogue is excluded from both

`~/.config/armadai/registry` is a synced catalogue. Flagging it would be pure noise, and it is
what skewed the calibration. Excluding it is not an optimisation, it is a correctness fix.

### Which rules apply to which scope

Measured by what each family actually reads:

| family | reads | project | global |
|---|---|---|---|
| `assets` (A01 A02 A05 A08 A09 A12) | agents, skills | yes | **yes** |
| `collisions` (C01–C05) | agents, skills, instructions | yes | **yes** |
| `models` (A03 A04) | — | yes | **yes** |
| `references` (A10 A11) | agents, instructions | yes | **yes** |
| `similarity` (A06 A07) | agents | yes | **yes** |
| `rightsizing` (R01 R02 R04) | skills, instructions | yes | **yes** |
| `usage_rules` (U01–U04) | + transcripts | yes | **no** |

Only `usage_rules` is genuinely project-bound: it correlates declarations against *this
project's* Claude Code transcripts. Everything else is a property of the assets themselves and
holds wherever they live.

This is deliberately a whitelist of one exclusion rather than a per-rule opt-in: a rule that
reads only `ctx.config` cannot tell which scope filled it, so the default must be "applies",
with `usage` the single documented exception.

### The threshold: keep 4000, change its justification

The measured distribution of the 47 **installed** skills, in tokens:

```
n=47  min=2  median=628  p75=915  p90=2456  max=10087
```

So 4000 sits **above** the p90 of the corpus that will actually be audited. Recalibrating to
2456 would flag roughly 5 skills instead of 1.

**Decision: keep 4000 and stop justifying it as a quantile.** Two reasons.

A quantile says "you are in the top 10% of what happens to be on this machine" — which changes
as the corpus changes, and 47 samples is a thin base to derive precision from. A **context
budget** says "this costs 4000 tokens the moment it triggers", which is true regardless of what
anyone else's skills look like. The second is the useful claim, and it is the one the rule was
always really making.

And the honest reporting follows: the docs must state the rate on the auditable corpus — **1 of
47 installed skills (2%)** — not the 4.3% derived from a corpus that is 88% catalogue. That
4.3% figure is exactly the defect #384 existed to fix, one layer up: a documented number that
the code does not deliver.

`skill_token_threshold` stays configurable, so a user whose skills run large can lower it.

## What this unlocks beyond R01

Three defects we hit **by hand** this week, invisible to the tool as it stands:

- Stale agent definitions — `qa-specialist.md` asserted two contradicting gaveldrop counts
  (#388), found by an agent reading its own definition, never by the audit.
- Skill/memory duplication — measured on the `armadai` skill: 9 of 10 lessons duplicated the
  memory, 6163 words loaded on every invocation before #382/#383.
- The real weight of global context, which `R04` cannot compute because it only sees the repo.

It is also the only angle from which "a `/doctor` for fleets" holds: sizing a fleet means
looking where the fleet lives.

## Out of scope

- Auditing the catalogue (above).
- `R03`, the memory-duplication rule, still dropped on principle: the memory is under the
  user's own directory and private to them. Note the tension — a *global* audit is closer to
  that boundary than a project one, so if this is ever revisited it belongs here, not in the
  project scope.
- Any automatic rewriting. The audit reports.

## Measured corrections, found while implementing (2026-08-26)

The three decisions above held. Four factual claims inside them did not — each measured on the
same machine the spec was measured on, through the real binary where the binary can answer.

1. **`~/.config/armadai/agents` cannot be read by this reverse pass.** The design above lists it
   as a global surface ("**77 entries here**") and applies the `assets` family to it. All 77 files
   are ArmadAI-format (H1 + `## Metadata` + `## System Prompt`); **none** carries YAML
   frontmatter. Measured by pointing the shipped binary at them
   (`ln -s ~/.config/armadai/agents probe/.claude/agents && armadai audit probe`):
   `77 critical, 2 warning(s), 0 info` / `A01×77` — every file flagged "missing YAML
   frontmatter", and a non-zero exit on a healthy library. So `--global` reads
   `~/.claude/agents` and *names* `~/.config/armadai/agents` as skipped, with its file count.
   Reading it properly needs an ArmadAI-format reverse importer, which is separate work: even
   with one, `A02` would fire on all 77, because the ArmadAI agent format has no `description`
   field at all.

2. **"Recalibrating to 2456 would flag roughly 5 skills instead of 1" is false.** Measured over
   the installed corpus: 2456 flags **1**, exactly as 4000 does. Only 4 skills exceed 2456 and
   3 of them already carry a `references/`. This *strengthens* the decision to keep 4000 — the
   threshold is not the binding constraint on this corpus, the `references/` condition is — but
   the reason given was not the real one.

3. **The `references/` split-rate argument does not survive the corpus fix.** It is inherited
   from #386 (`52%` of skills above the p90 carry a `references/` against `32%` below), and that
   was measured on the same 88%-catalogue corpus this spec disqualifies. Re-measured on the
   auditable 48: **75%** above the p90 versus **77%** at or below — no contrast at all, on 4
   samples and 44. The two-condition rule is still right, for a definitional reason (a skill
   with `references/` has already used the mechanism the suggestion recommends), and the
   statistical claim has been removed from both the rule and the wiki.

4. **The corpus is 48, not 47.** `~/.config/armadai/skills` now holds 41 directories, one of
   which has no `SKILL.md` (hence `min=0`, not `min=2`). `p90=2456` is unchanged and the
   reported rate is unchanged at 2%, which is the point: the figure is robust to that drift and
   the exact `n` is not load-bearing.

One limit worth recording rather than fixing here: `R04` counts the instructions file as it sits
on disk and does not follow Claude Code's `@file` imports. On this machine `~/.claude/CLAUDE.md`
is 59 tokens and `@`-imports a 241-token file, so the global weight figure is a floor. That is a
pre-existing `R04` limit, not one the scope split introduced.
