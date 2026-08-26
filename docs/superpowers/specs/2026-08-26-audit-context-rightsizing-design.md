# Audit: context rightsizing — design

**Issue:** #384 · **Date:** 2026-08-26 · **Status:** design, awaiting approval

## Problem

`armadai audit` carries 26 rules in four families: `A01`–`A12` (per-asset quality),
`C01`–`C05` (collisions between assets), `D01`–`D05` (deep pass, LLM-judged), `U01`–`U04`
(observed usage from transcripts). They answer **is it declared?** and **does it run?**

None answers **is it sized correctly?** — which is what costs context on every single
invocation, for every agent in the fleet.

This matters more for ArmadAI than for a single repo, because ArmadAI *produces* these assets
across a fleet: `link` generates configs per target, `new` propagates templates, `init`
installs skills. A 6000-word skill in a ten-agent fleet is ten times the cost.

## What grounds this

Measured on our own assets, 2026-08-26, before correction:

| Asset | Before | After | Finding |
|---|---|---|---|
| skill `armadai` | 6163 words, loaded **in full as soon as the skill triggers** | 1334 (−78%) | 9 of 10 lessons **duplicated the memory** |
| root `CLAUDE.md` | 1247 words | 726 | its module map was **stale** — OH7 (#252) had moved `parser/`, `providers/`, `core/`, `storage/`, `secrets/`, and `ls` showed none of them at the documented path; `claude_adapter/`, which exists, was absent; and it described `api/openai.rs`/`proxy.rs` as `todo!()` stubs one day after #374 implemented them |

A stale map is **worse** than no map: it is read as authoritative. Both defects survived weeks
of daily reading, including by the agent that read the file every session.

### Distribution, to set the threshold rather than guess it

460 real `SKILL.md` files on this machine:

```
n=460  min=3  median=746  p75=1343  p90=2224  max=41795
```

Correlation between size and having a `references/` directory:

| Band | Skills | Without `references/` |
|---|---|---|
| above p90 (2224 words) | 45 | 21 (46%) |
| at or below p90 | 415 | 281 (67%) |

Two things follow. Large skills are **more** often split than small ones, so size alone is a
bad signal — the rule must require *both* size and the absence of splitting. And the target is
narrow: **21 of 460 (4.6%)** exceed p90 with no `references/` at all. That is the right order
for a warning: neither noise nor decoration. Note `quality-playbook` at 41795 words with 16
references — big and correctly structured, and it must not be flagged.

## Design

A new family **`R`** (rightsizing). `A`, `C`, `D`, `U` are taken; `R` is free.

The family is justified rather than folded into `A`: `A` rules judge one asset's own quality,
`R` judges the **cost of the context loaded by default**. That is a different question, and it
wants its own section in the report.

### `R01` — oversized skill with no progressive disclosure

**Warning.** A `SKILL.md` whose estimated tokens exceed `skill_token_threshold` **and** whose
skill directory has no `references/` entry.

This is the exact counterpart of `A05` (`a05_oversized_prompt`, agents' `system_prompt`,
threshold 4000 tokens) for skills. Not a duplicate: `A05` covers agents, `R01` covers skills.
`A09` already validates a skill's *structure* (SKILL.md present, frontmatter, description) but
never its size.

- Threshold: `skill_token_threshold`, default **3000** tokens. Derived from the measured p90
  (2224 words ≈ 3000 tokens via `estimate_tokens`, chars/4). Configurable like the others.
- The `references/` condition is what keeps `quality-playbook` out of the report.
- Suggestion: name the mechanism the product already supports — `core/skill.rs:61` loads
  `references/`, `:106` copies it on install. The author is not being asked to invent anything.

> **Correction (post-review, measured):** the conversion above is wrong and the default is
> **4000**, not 3000. On the same 460-skill corpus the ratio is **1.84 tokens per word**
> (median), not 1 — so the p90 of 2224 words is ≈4100 tokens, and the p90 of the token
> distribution read directly is 3956. Measured through the real command over those 460 skills:
> the 3000 default flagged **54 (11.7%)** where this section promises ~4.6%; 4000 flags
> **20 (4.3%)**. The p90 was the right criterion; only its conversion was wrong. Also stale
> here: the loader is at `crates/armadai-core/src/skill.rs` — `core/skill.rs` is one of the 16
> paths `R02` itself reports as nonexistent.

### `R02` — a path named in the instructions file does not exist

**Warning.** A filesystem path cited in the root instructions file that resolves to nothing.

Counterpart of `A10` (`a10_broken_references`, which does this for `@agent` mentions). This is
the rule that would have caught our own stale map.

False positives are the whole difficulty, so the rule is deliberately narrow:

- only paths inside backticks (the convention the instructions files already use);
- must look like a repo path — contains `/` **or** ends in a known source extension;
- skipped inside fenced code blocks (those are examples, not claims);
- skipped when matching an obvious placeholder (`path/to/`, `<...>`, `foo/bar`, `example/`);
- skipped when the path contains a glob or a `*` — it describes a set, not a file.

Anything that survives those filters and does not exist is a real broken claim.

> **Correction (Task 3-4, measured against the standard):** the Agent Skills standard is
> three-tier — metadata always, the `SKILL.md` body **when the skill triggers**, linked
> files on demand. Earlier wording here said a skill body loads "on every invocation",
> which is wrong. The cost is real but it is engaged at trigger time; the instructions
> file is what loads unconditionally. Rule messages were reworded accordingly.

### `R04` — weight of the front-loaded context

**Info, no judgement.** Reports the total estimated tokens loaded by default: root instructions
file + each skill's `SKILL.md` (excluding its `references/`, which load on demand).

Modelled on `U04`, which "reports without judgement". Its value is making a cost visible that
nobody currently sees, not flagging anything. Rendered in the report as one line per
contributor plus a total.

### `R03` was considered and dropped

The issue proposed a rule for a lesson duplicated between a skill and the project memory — the
defect measured above (9 of 10). It is **out of scope**, and not for cost reasons: the memory
lives outside the project, under the user's own directory, and is private to them. `armadai
audit` is project-scoped and must not read it. Detecting this belongs to a personal tool, not
to a project audit someone may run on a repo that is not theirs.

## Where the code goes

The registry documents its own extension point: *"adding a rule = one module + one entry
here"*.

- New module `crates/armadai/src/audit/rules/rightsizing.rs`.
- Three entries in `registry()` (`audit/rules/mod.rs:155`).
- Two fields on `AuditSettings`: `skill_token_threshold: usize` (default 4000, see the
  correction under `R01`) and nothing for
  `R02`/`R04` — neither has a tunable.
- `R04`'s output is a finding like any other, so `report.rs` needs no new plumbing; the report
  groups by rule family already.

### The one context change required

Checked, so implementation does not have to rediscover it:

- `ImportedSkill.source_path` points at the **`SKILL.md` itself** (`reverse/claude.rs:155`), so
  `R01` gets the skill directory from `source_path.parent()` and tests `references/` there. No
  change needed.
- `ImportedInstructions` carries `content: String` (`reverse/mod.rs:70-73`), so `R02` and `R04`
  have what they need. No change needed.
- **`ImportedSkill` carries no body**, only `name`, `source_path`, `description`,
  `has_skill_md`, `has_frontmatter`, `issues`, `extra`. `R01` needs a size, and **no rule reads
  the disk today** — the single `read_to_string` in the rules tree is `AuditSettings`
  loading config (`rules/mod.rs:93`). Rules judge a pre-loaded context; the reverse pass is
  what touches the filesystem.

So: add **`body_tokens: usize`** to `ImportedSkill`, filled by the reverse pass which already
has the file open. Not the full body — a count. Loading a whole `SKILL.md` into the audit
context to ask one question about its size would be, precisely, the defect this spec exists to
measure. The field also keeps `R01` a pure function of the context, like every other rule.

`estimate_tokens` is `pub(crate)` in `rules/mod.rs`, but the reverse pass sits outside that
module; expose the same helper (or move it) rather than writing a second chars/4 somewhere.

## Tests

Each rule gets unit tests via the existing `rules::test_support` helpers, plus one black-box
case on the real binary — the pattern `link_manifest.rs` uses, because the substance of `R02`
is filesystem resolution and of `R04` the aggregation, neither of which a unit test on a
fixture proves.

Mandatory per this repo's measured recurring defect (10 occurrences): every test is verified by
mutation — break what it protects, confirm red, restore, report the observed output. Specific
mutants to kill:

- `R01` with the `references/` condition removed → `quality-playbook`-shaped fixture must fail.
- `R01` with the threshold set to 0 → the negative fixture (small skill) must fail.
- `R02` with the fenced-code-block filter removed → a fixture citing `path/to/thing` inside a
  code fence must fail.
- `R02` with the existence check inverted → the positive fixture must fail.
- `R04` with one contributor dropped from the sum → the total assertion must fail.

`R02`'s filters are where this rule lives or dies, so each filter gets its own negative
fixture. A filter with no fixture is a filter that can silently stop working.

## Out of scope

- The memory-duplication rule (`R03`, above).
- Any automatic rewriting. The audit reports; it does not edit. `armadai audit --fix` does not
  exist and this design does not introduce it.
- The separate question of whether `armadai_protocol_block()` is *honoured*. Measured at 50
  words across 6 injection sites, so it is not a sizing problem — but the project memory notes
  the model does not reliably emit those markers in practice. That is an effectiveness
  question, and it needs its own issue.

## What was checked and is healthy — do not "fix" it

Two of the four hypotheses behind #384 were **false**, measured:

- **`armadai new`'s templates are already well-sized**: 34 to 293 words across the twelve, at
  most 4 prescriptive turns (`tdd-red.md`). No rule needed.
- **`armadai_protocol_block()` is 50 words for 6 sites** — negligible. The "costly repetition"
  hypothesis does not survive measurement.

A third was half wrong: `core/skill.rs` **already supports** `references/`. The product has the
progressive-disclosure mechanism; what is missing is only the measurement of whether it is used.
