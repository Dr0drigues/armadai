# gaveldrop defects & divergence report — armadai's first-consumer migration

**Author:** armadai (`~/work/misc/armadai`), the migration that moved `crates/armadai/tests/e2e/`
(a hand-rolled YAML case engine) onto `gaveldrop` (`~/work/misc/gaveldrop`), armadai's **first
real consumer**.
**Audience:** a gaveldrop maintainer. This is the deliverable the whole migration exists to
produce: the briefing's own premise is that *"every previous technology added to gaveldrop
revealed a defect that nothing else had found"* — this report is the catalog of what armadai
found, what got fixed upstream during the build, and what remains open.
**Scope:** covers the full migration (tasks T1–T6, branch `feat/gaveldrop-migration`, commits
`3e75060`..`39784cb`). gaveldrop working tree evidence is cited against `6d896b8` (the tree as it
stood when this report was written); armadai evidence is cited against the branch tip.
**Non-goal:** this is not a bug tracker entry — no gaveldrop file was modified to produce it (all
fixes below already shipped upstream before or during the build). Findings still open are flagged
as such, with a recommendation, not a patch.

---

## Summary table

| # | Finding | Status | Severity |
|---|---|---|---|
| G1 | Public runner had no adapter-injection seam | **Fixed upstream** (`run_all_with`, PRs #70/#71) | Was blocking |
| F2 | Unknown `fake:` key silently widened the match to a catch-all (inversion) | **Fixed upstream** (#71); armadai diverges from the briefing's example as a result | Was high — silent, not loud |
| F3 | Conformance kit assumes a script-executing subject; armadai's is fixed-purpose | Confirmed satisfiable via a shared-helper pattern, no gaveldrop change | Design note, not a defect |
| F4 | `prov_model_non_empty` → two `field_non_empty` invariants | Confirmed a deliberate semantic split (gaveldrop's shape is single-field by design) | Non-issue once understood |
| — | Events semantics (`check_subsequence`/`check_counts` vs. old harness) | Confirmed **identical** | Non-issue |
| — | `armadai-fake`'s plain path-dep leaked into a bare workspace release build | **New finding, fixed in armadai**, worth an upstream adoption note | Real adoption footgun |
| — | `expect.storage` (SQLite row-count assertions) | Deliberately dropped — no case used it, gaveldrop has no equivalent | Scope drop, not a gap |
| — | CI HTML/JSON report artifact | Gaveldrop already offers the capability (`report::html::Html`, `jsonl`, `junit`); wiring is a **separate, unfinished task** — current CI is stale, not gaveldrop's fault | Open, tracked |
| — | gaveldrop working-tree drift during the build (9ed05ec → 6d896b8) | Coordination observation, not a defect | Informational |

---

## 1. G1 (fixed) — the public runner had no adapter-injection seam

**Symptom.** Before the fix, gaveldrop's only two public suite-runner entry points hardcoded the
built-in adapter registry, and the one function that accepted an adapter list was private:

```rust
// crates/gaveldrop/src/runner.rs (pre-fix, as found ~cd85141)
pub fn run_all_selected(config, root, fake_binary, sink, shard, only) -> Result<Report, ConfigError> {
    let adapters = adapters::registry();          // hardcoded
    ...
    run_one(&case, fake_binary, config, root, &adapters)
}
fn run_one(..., adapters: &[Box<dyn Adapter>]) -> ...   // private
```

`gaveldrop::adapters::registry()` is a fixed `vec![Box::new(Web), Box::new(Shell), Box::new(Process)]`
(`crates/gaveldrop/src/adapters.rs:35`). The conformance kit *does* take an adapter
(`gaveldrop_conformance::run_with`), so a custom adapter is provable in isolation — but the runner
that actually executes cases and produces a `Report` could not use it. armadai's cases carry
`setup.pattern`, a key no built-in adapter claims: the exact scenario the task exists to test was
unreachable through gaveldrop's own suite runner. The only escape hatches (rewrite cases to
`run:`+`fake.render`, or reimplement `run_one`'s loop) both defeated the point of adopting
gaveldrop at all.

**Fix, as shipped** (gaveldrop PRs #70/#71, confirmed at `6d896b8`,
`crates/gaveldrop/src/runner.rs:88-113`):

```rust
pub fn run_all_with(
    config: &Config,
    root: &Path,
    fake_binary: &Path,
    sink: &mut dyn Sink,
    shard: Option<crate::config::Shard>,
    only: Option<&str>,
    adapters: &[Box<dyn Adapter>],
) -> Result<Report, ConfigError> {
    let paths = crate::config::select(config.discover(root)?, shard, only)?;
    let mut outcomes = Vec::with_capacity(paths.len());
    for path in paths {
        let outcome = match Case::load(&path) {
            Ok(case) => run_one(&case, fake_binary, config, root, adapters),
            Err(error) => setup_failure(&path.to_string_lossy(), 0, error.to_string()),
        };
        sink.case_finished(&outcome);
        outcomes.push(outcome);
    }
    let report = Report::from(outcomes);
    sink.finish(&report);
    Ok(report)
}
```

`run_all_selected` (`runner.rs:35-52`) is now a thin delegate: `run_all_with(config, root,
fake_binary, sink, shard, only, &adapters::registry())`. Shipped exactly as the additive,
backward-compatible shape armadai's finding doc (`docs/superpowers/gaveldrop-adapter-injection-finding.md`)
proposed — including the doc comment's own worked example of prepending a consumer adapter to
`adapters::registry()`.

**armadai's consumption** (`crates/armadai/tests/gaveldrop.rs:407-436`, `e2e_suite_passes_through_gaveldrop`):

```rust
let mut chain: Vec<Box<dyn Adapter>> = vec![Box::new(Armadai)];
chain.extend(adapters::registry());
let report =
    gaveldrop::runner::run_all_with(&config, root, fake, &mut sink, None, None, &chain).unwrap();
```

This is the decisive migration gate (task T6): all 9 cases ran through the public runner with the
`Armadai` adapter first in the chain, first try, no wiring bug — `score 60/60`.

**Verdict.** Fixed cleanly, no residual gap. This one item alone was blocking; everything else in
this report was buildable in parallel while G1 was outstanding.

---

## 2. F2 (fixed) — an unknown `fake:` key didn't get dropped, it emptied the match (inversion)

**Symptom, as originally found.** `gaveldrop::case::Case` is `#[serde(deny_unknown_fields)]`
(`crates/gaveldrop/src/case.rs:19-20`) with `fake: Option<Scenario>` (`case.rs:35`, using
`gaveldrop_fake`'s own `Scenario`/`Rule`/`Match`/`Response`). `gaveldrop_fake::Match`
(`crates/gaveldrop-fake/src/rule.rs:20`) is **deliberately without** `deny_unknown_fields` — its
own doc comment explains why:

> "Deliberately without `deny_unknown_fields`: a project composes its own criterion on top of
> this one — `struct MyMatch { #[serde(flatten)] core: Match, agent: Option<String> }` — and
> `flatten` is incompatible with rejecting unknown fields." (`rule.rs:14-18`)

Before the fix, this meant a consumer's extra `Match` fields (armadai's `agent:`) parsed
**silently and were dropped** rather than composed — worse than a no-op: an unrecognized
criterion collapsed the whole `Match` toward "all fields absent," which is gaveldrop's own
definition of the **catch-all** (`rule.rs:12-13`, "a `Match` whose fields are all absent is the
catch-all"). A rule meant to answer only one agent's first call became a rule that answered
*everything*, silently — a case could load green while proving nothing, because the first
(over-eager) catch-all rule shadowed every later, more specific rule.

**Fix, as shipped** (gaveldrop #71, `crates/gaveldrop-fake/src/rule.rs:340-370`,
`the_published_key_lists_are_what_serde_understands`): `Match`, `Response`, and `Rule` each
publish a `KEYS` constant, and gaveldrop's own test enforces that a serialized key list of every
populated field matches `KEYS` exactly — the test's own assertion message states the fix's
intent directly:

> "whoever parses a scenario out of a case refuses unknown keys against these lists, because
> `flatten` forbids `deny_unknown_fields` here. A field present in the type and missing from the
> list would be refused as a typo" (`rule.rs:352-355`)

So gaveldrop now **loudly refuses** an unknown key under a `fake:` rule's `match`/`response`
(checked against the published `KEYS`, at case-load time, rather than silently degrading the
match). This restores gaveldrop's own stated "a failure is diagnosable" property (see the
finding doc, §F2, for the original severity assessment: this was worse than reported — an
*inversion*, not a drop).

**Consequence for armadai — a deliberate divergence from the briefing.** The fix does not (and
cannot, given `flatten`'s constraint) make the top-level `Case.fake:` extensible to a consumer's
own vocabulary — `agent:`, armadai's own match criterion, is still not one of `gaveldrop_fake::Match`'s
`KEYS`, so it would still be refused there. armadai's scenario therefore lives under
**`setup.scenario:`** — nested inside `Setup`'s `#[serde(flatten)] extra: BTreeMap<String, Value>`
(`crates/gaveldrop/src/case.rs:80-116`, genuinely opaque, no `deny_unknown_fields` — see the
struct's own doc: "Everything else is opaque and travels untouched... which is what lets a
project write its own vocabulary here without the core learning any domain words," `case.rs:82-85`)
— **not** under the top-level `fake:` the briefing's own worked example showed. Confirmed in every
one of the 9 migrated cases (e.g. `crates/armadai/tests/cases/direct.yaml`): the whole scenario
block moved from a top-level `fake:` to `setup: { scenario: { rules: [...] } }` (task T2 report,
mechanical for all 9 files, no exceptions).

`gaveldrop.yaml`'s own project-level `fake:` key (distinct from `Case.fake`) only understands
`bins`/`no_passthrough` (`FakeConfig`, `deny_unknown_fields` — confirmed against
`crates/gaveldrop/src/config.rs`), which is a second, independent reason armadai's data could
never sit there.

**Recommendation (unchanged from the finding doc, now with the fix's actual shape known):**
gaveldrop's maintainers may still want to decide, and document, whether the top-level `fake:` is
reserved for gaveldrop's own `Scenario` shape or meant to be consumer-extensible — the `KEYS`
refusal makes the failure loud (good), but a consumer with its own fake vocabulary (like armadai)
still has no path at the top level and must be told, in `docs/`, to use `setup:` instead. This is
not blocking and armadai works around it regardless, but the current state (loud refusal, no
guidance to `setup:`) means the next consumer will rediscover this by trial and error unless it's
written down.

---

## 3. F3 — the conformance kit's checks assume a script-executing subject; armadai's is fixed-purpose

**The tension.** `gaveldrop_conformance`'s six checks (`crates/gaveldrop-conformance/src/checks.rs`)
each hand the adapter's factory a concrete shell script and assert the *subject* ran it: `exit 7`,
`echo out; echo err >&2`, `printf %s "$HOME"`, a cleared-variable probe, a file write, and an
unclaimed-tool probe. The built-in `Shell` adapter satisfies these trivially because its subject
*is* the script. armadai's subject is a fixed invocation, `armadai run <fleet>` — it cannot be
made to `exit 7` or print `$HOME` on demand.

**Resolution used (no gaveldrop change).** `Armadai::invoke` (`crates/armadai/tests/gaveldrop.rs:317-335`)
has two branches funneling through one shared helper, `run_in_iso` (`gaveldrop.rs:38-73`):

- **Conformance-probe branch**: `setup.extra["probe_script"]` present → runs `sh -c <script>`
  through `run_in_iso`.
- **Real branch**: writes the project + scenario, builds `armadai run …`, runs *that* through the
  same `run_in_iso`.

Both branches apply the isolation's environment, run the command with the isolated root as CWD,
capture exit/stdout/stderr, read the call journal, and report file changes — through the **same
function**. This satisfies the load-bearing condition F3 exists to name: the conformance-probe
branch and the real branch must end in the same exit, or the kit certifies isolation plumbing that
the real cases never actually exercise (a "vacant kit"). Confirmed at `gaveldrop.rs:11-19`
(module doc, states this explicitly as the design rationale) and by reading the final
implementation (not just design intent): there is exactly one `run_in_iso` definition and both
call sites (`gaveldrop.rs:322-326`, `gaveldrop.rs:331-333`) invoke it identically, differing only
in `argv`/`extra_env`.

**Result.** `armadai_adapter_is_conformant` (T5) passed **first run, no adapter change needed**
(`.superpowers/sdd/2026-07-30-gaveldrop-migration/task-T5-report.md`): all 6 checks held. The
kit's own docstring already states the intent this pattern relies on — *"The checks are about the
isolation contract, not about how a subject is invoked... must still be checkable"*
(`gaveldrop-conformance/src/lib.rs:13`) — so this is confirmed to be the intended usage for a
fixed-subject adapter, not a workaround exploiting an oversight.

**Recommendation.** Document the "purpose-built adapter → funnel through a shared isolation-plumbing
helper the factory can also drive" pattern in gaveldrop's `docs/conformance.md`, since armadai is
proof it works but nothing in the kit's own docs spells it out for the next non-`run:` adapter
author. No code change requested — this is a documentation gap, not a defect.

---

## 4. F4 — `prov_model_non_empty` split into two invariants (a semantic split, not a rename)

**What changed.** The old harness had one hand-rolled invariant checking that *both* `prov` and
`model` were non-empty on every `agent_start` event (confirmed at
`git show 2cb3f52:crates/armadai/tests/e2e/runner.rs`, `prov_model_non_empty`, lines ~176-188 in
that blob):

```rust
fn prov_model_non_empty(observed: &[Value]) -> Result<(), String> {
    for ev in observed.iter().filter(|v| v.get("t")... == Some("agent_start")) {
        let prov = ev.get("prov")...unwrap_or("");
        let model = ev.get("model")...unwrap_or("");
        if prov.is_empty() || model.is_empty() {
            return Err(format!("agent_start with empty prov/model: {ev}"));
        }
    }
    Ok(())
}
```

gaveldrop's `FieldNonEmpty` invariant shape (`crates/gaveldrop/src/verdict/invariants.rs:44-50`)
checks exactly **one** field per invariant, and this is a deliberate, documented design choice —
not a limitation the fix works around:

> "**One field, deliberately.** A project wanting two — 'every `agent_start` carries both a
> provider and a model' — declares two named invariants rather than one taking a list. It costs
> a line of configuration and buys the diagnostic: a case failing `model_non_empty` says which of
> the two was missing, where a `prov_and_model_non_empty` would only say that one of them was."
> (`invariants.rs:38-43`)

`gaveldrop.yaml` (repo root) therefore declares two invariants where the old suite had one:

```yaml
invariants:
  agent_start_end_symmetric: { shape: paired, start: agent_start, end: agent_end, key: agent }
  single_result:             { shape: exactly_one, type: result }
  prov_non_empty:            { shape: field_non_empty, type: agent_start, field: prov }
  model_non_empty:           { shape: field_non_empty, type: agent_start, field: model }
  no_orphan_events:          { shape: no_orphan, key: agent, root: agent_start }
```

**The old suite had 4 invariants; `gaveldrop.yaml` has 5** — confirmed by `config_loads`
(`crates/armadai/tests/gaveldrop.rs:338-343`, `assert_eq!(cfg.invariants.len(), 5)`) and by reading
the old `runner.rs`'s `check_invariants` dispatch (4 named functions:
`agent_start_end_symmetric`, `prov_model_non_empty`, `single_result`, plus a 4th not reproduced
here). **A reader diffing invariant counts across the migration will notice a `+1` and should not
read it as scope creep** — it is the same semantic coverage, expressed as gaveldrop's shape
requires, with a better per-field diagnostic as a side effect.

**Verdict.** Not a defect — gaveldrop's `field_non_empty` shape works exactly as designed and
documented; this is recorded here because the task brief called it out as a "candidate finding to
verify," and it is now confirmed decided, not open.

---

## 5. Events semantics — `check_subsequence`/`check_counts` vs. the old harness: confirmed identical

The old harness's ordered-event check (`git show 2cb3f52:crates/armadai/tests/e2e/runner.rs:81-99`,
`check_events_order_and_fields`) and exact-count check (`runner.rs:124-141`,
`check_event_counts`) had these documented semantics:

- **Order/subset match**: each expected event must appear, in order, at or after a cursor that
  advances past the last match — "a subsequence match — unrelated events may appear in between."
  A match compares only the fields the expectation names (`event_matches`, subset-of-fields, not
  full equality).
- **Counts**: exact per-type counts across the *whole* observed stream, independent of the
  order check.

gaveldrop's equivalents (`crates/gaveldrop/src/verdict/events.rs`):

- `check_subsequence` (`events.rs:75-101`): a `cursor` that only ever advances (`cursor += offset
  + 1`), matching via `matches_partially` (`events.rs:132-136`, "True when every field the case
  named matches. Fields it did not name are not checked.") — the same cursor-advance,
  subset-of-fields subsequence semantics as the old harness, confirmed line-for-line against the
  old implementation, not just by description.
- `check_counts` (`events.rs:107-124`): exact per-`kind` counts across the whole `actual` slice —
  same "declared 0 proves an event never happened" semantics as the old exact-count check.

**Empirical confirmation, not just code reading**: task T6's decisive gate ran all 9 migrated
cases (each carrying both `expect.events` and `expect.event_counts` blocks, byte-for-byte
unchanged from the old harness per T2's transform) through `run_all_with`, and every case reached
the **same verdict** the deleted hand-rolled harness used to compute — `9 cases · 9 passed · 0
failed · 0 tolerated · score 60/60`, first run, no case needing a rewrite to pass. This is the
strongest evidence available: the two implementations produce identical pass/fail decisions
across the entire suite, not merely "look similar on paper."

**Verdict**: confirmed semantically identical. No finding here — recorded because the task brief
asked for it to be verified rather than assumed.

---

## 6. Scope drops (deliberate, not gaps)

### `expect.storage` — dropped, unused

The old harness's `Expect` struct (`git show 2cb3f52:crates/armadai/tests/e2e/case.rs:143-153`)
carried a `storage: Option<BTreeMap<String, usize>>` field, backed by a best-effort SQLite
row-count check (`git show 2cb3f52:crates/armadai/tests/e2e/runner.rs`, `check_storage`,
`#[cfg(feature = "storage")]`, opening the isolated project's SQLite DB and asserting exact row
counts against the tables `src/storage/schema.rs` creates).

Grepping every one of the 9 case files at the pre-migration commit (`6331363`, before any T2
transform) for a `storage:` key returns **zero matches** — no case ever exercised this capability.
gaveldrop has no equivalent assertion shape (its `Config`/`Case` schemas have no `storage:` key
anywhere). This is a clean, deliberate drop: a capability with zero live callers, dropped because
the new engine doesn't offer it, not because a case needed it and was silently weakened.

### CI report artifact — open, and currently **stale** (a real loose end, not gaveldrop's fault)

The old CI (`.github/workflows/ci.yml:94-107`) uploads `target/e2e-report/{e2e-report.html,json}`
as a build artifact, produced by the deleted `tests/e2e/report.rs` (429 lines, hand-written HTML
renderer with inline design tokens, deleted in T3). **As of this report, `.github/workflows/ci.yml`
has not been updated** — the `Upload e2e report` step still references a path nothing writes to
anymore (T3's own report flags this explicitly and defers it: "T7 does not address this either per
its brief; flagging for whoever picks up CI wiring." No T7 task exists in this migration's task
list — CI wiring was never picked up). Concretely: the artifact-upload step will run against an
empty/missing path (`if-no-files-found: warn`, so it won't fail the build, but the artifact will
silently stop existing).

**This is not a gaveldrop gap.** gaveldrop already ships file-based report sinks beyond `Terminal`:
`crates/gaveldrop/src/report/html.rs` (`Html<W: Write>`, a from-scratch self-contained HTML page —
its own module doc explains it was written fresh rather than ported from "the prototype," i.e.
gaveldrop's own predecessor, specifically because a prototype's HTML "renders things specific to
one project" and "a shell is quicker to write than to adapt"), plus `jsonl.rs`, `junit.rs`, and
`annotate.rs` sinks (`crates/gaveldrop/src/report/`). The capability the old CI step depended on
exists in gaveldrop today. **What's missing is armadai-side wiring** — the suite-run test
(`e2e_suite_passes_through_gaveldrop`) currently uses `Terminal::plain(stdout)` only; producing
`e2e-report.html`/`.json` again would mean adding an `Html`/`Jsonl` sink (or a `Tee`,
`crates/gaveldrop/src/report.rs:167`, which fans out to multiple sinks) and updating
`ci.yml`'s upload paths. **This is an open item for a follow-up task**, not a defect to report
upstream — flagging here because the brief asked whether this scope drop was addressed, and the
honest answer is: not yet, and CI is presently pointing at a dead path.

---

## 7. New friction found during the build: a non-optional path dependency leaked into the release binary

**Not anticipated by the briefing or the logbook — found empirically in task T3.** The new
`crates/armadai-fake` workspace member (added in T1) declared `gaveldrop-fake` as a **plain,
non-optional** dependency:

```toml
# crates/armadai-fake/Cargo.toml, as T1 first wrote it
[dependencies]
gaveldrop-fake = { path = "../../../gaveldrop/crates/gaveldrop-fake" }
```

Root `Cargo.toml` has `members = ["crates/*"]` — a glob that automatically included
`crates/armadai-fake` the moment the directory existed (verified: no root-manifest edit was needed
for `cargo build -p armadai-fake` to work at all). The consequence, found when T3 ran the
release-clean check the plan requires:

```
$ cargo build --release --no-default-features --features tui,storage   # no -p flag, exactly what CI runs
   Compiling armadai v1.0.0-rc.5
   Compiling gaveldrop-fake v0.1.0 (.../gaveldrop/crates/gaveldrop-fake)   # <- leaked in
   Compiling armadai-fake v0.0.0
    Finished `release` profile [optimized] target(s) in 32.45s
```

**Root cause**: a bare `cargo build --release` with no `-p` builds *every* workspace member
directly, independent of any other crate's feature gating. `armadai`'s own `e2e-fake` feature
gate (controlling whether `armadai` itself pulls in `armadai-fake`) is irrelevant here —
`armadai-fake` is built as its own top-level target regardless, and its manifest as originally
written unconditionally pulled in the external `gaveldrop-fake` path dependency. Confirmed by
isolating the two builds: `cargo build --release -p armadai …` (scoped) never touches
`gaveldrop-fake`; the unscoped, whole-workspace build always did, purely because
`armadai-fake` is a workspace member with a non-optional external dep.

**Fix applied (armadai-side only)**: `crates/armadai-fake/Cargo.toml` gates `gaveldrop-fake`
behind a new, off-by-default `engine` feature:

```toml
[features]
engine = ["dep:gaveldrop-fake"]

[dependencies]
gaveldrop-fake = { path = "../../../gaveldrop/crates/gaveldrop-fake", optional = true }
```

with `#[cfg(feature = "engine")]` gating the three symbols in `armadai-fake::lib.rs` that actually
touch `gaveldrop_fake` (the import, `pub fn run()`, and one unit test). `armadai`'s own two
dependency edges on `armadai-fake` (the optional main dependency behind `e2e-fake`, and the
dev-dependency used by tests) both explicitly request `features = ["engine"]`, so `armadai`
itself is unaffected — it always gets the full engine when it needs it. Confirmed after the fix:
`cargo build --release --no-default-features --features tui,storage` compiles only `armadai` +
`armadai-fake` (no `gaveldrop`/`gaveldrop-fake`/`gaveldrop-conformance`); `cargo tree
--no-default-features --features tui,storage -e normal,build -i gaveldrop-fake` → "nothing to
print."

**Why this is worth surfacing to gaveldrop, not just noting as an armadai fix.** This is a general
shape any Rust consumer of `gaveldrop-fake`/`gaveldrop` will hit the moment it puts its own
adapter-support crate inside its workspace: a workspace member with a plain path (or even
released) dependency on a gaveldrop crate is built by a bare `cargo build --release` regardless of
how carefully the *top-level* binary's own Cargo features are designed, because Cargo builds
workspace members independently of each other's feature gates unless the dependent crate itself
guards the edge. **Recommendation**: gaveldrop's own adoption docs (wherever integration is
documented for a consumer building a custom adapter/fake-engine crate) should call this out
explicitly — "if your fake/adapter support code lives in its own workspace member, gate your
`gaveldrop`/`gaveldrop-fake` dependency behind a feature; a plain dependency there will reach a
release build of your whole workspace even if your main binary never activates it." This is not a
gaveldrop code defect (gaveldrop's own crates are fine either way) — it's an omission in guidance
for exactly the "first custom-adapter consumer" scenario this migration is meant to pressure-test.

---

## 8. gaveldrop working-tree drift during the build

Because armadai depends on gaveldrop **by path** (not a released version — deliberate, per the
migration plan, until gaveldrop ships one), gaveldrop's own motion during the build was felt
directly rather than through a version bump the consumer chooses when ready:

- Logbook baseline: gaveldrop @ `9ed05ec` (F2's fix, "refuse a fake key nothing reads instead of
  widening the match," #71) — "UNBLOCKED" snapshot dated 2026-07-29.
- T4 (adapter implementation) found the checkout had moved to `6d896b8` — two commits further:
  `5b4c7d5` ("fail where a capture was declared, not two steps later," #73) and `6d896b8` ("let a
  case declare the variables its subject reads," #75).
- The `6d896b8` commit (#75) added a **new field**, `env: BTreeMap<String, String>`, to
  `gaveldrop::case::Setup` (`crates/gaveldrop/src/case.rs:80-116`) — not present in the brief's
  original `Setup` snippet. T5's hand-constructed `Case` literal (`crates/armadai/tests/gaveldrop.rs:372-386`,
  `as_armadai_probe`) had to add `env: std::collections::BTreeMap::new()` to keep compiling, since
  `Setup` has no `#[serde(deny_unknown_fields)]`-adjacent `Default` shortcut usable from a hand-built
  literal with every other field named explicitly.
- No API surface relevant to any task in this migration **changed semantics** between `9ed05ec`
  and `6d896b8` — only additive drift (a new field, unrelated fixes). Every task report confirmed
  its API facts against whatever commit the checkout actually stood at when that task ran (a path
  dependency always builds what's on disk, not a pinned hash), catching each drift before it
  caused a silent mismatch.

**Observation, not a defect**: a path-dependency consumer feels every upstream commit immediately,
including ones landing mid-task with no changelog to check against. This worked out fine here
because gaveldrop's own commits were small, additive, and well-documented in their own commit
messages/doc comments — but it is a coordination cost specific to path-dependency adoption that a
released-version consumer wouldn't pay. Worth keeping in mind if/when gaveldrop and armadai
discuss what a first tagged gaveldrop release should stabilize before armadai's migration branch
merges to `master`.

---

## 9. Line count: what gaveldrop now owns vs. what armadai still maintains

The deleted harness (`git show 2cb3f52`, one commit before T3's deletion) totaled:

| File | Lines | Responsibility |
|---|---|---|
| `tests/e2e/runner.rs` | 456 | verdict evaluation (event/invariant/storage checks) |
| `tests/e2e/report.rs` | 429 | HTML/JSON report rendering |
| `tests/e2e/case.rs` | 253 | case model + JSON-schema generation |
| `tests/e2e/harness.rs` | 441 | project-writing + command-building (isolation, argv, agent markdown) |
| `tests/e2e/mod.rs` | 7 | module wiring |
| `tests/e2e.rs` | 8 | test-target entry |
| **Total** | **1594** | |

**Now gaveldrop's responsibility, deleted from armadai**: `runner.rs` (456, evaluation) +
`report.rs` (429, reporting) + `case.rs` (253, case model/schema) = **1138 lines** armadai no
longer authors or maintains — replaced by gaveldrop's `verdict::evaluate`, `report::*` sinks, and
`case::Case`/`Config` respectively.

**Still armadai-side** (the "adapter + config" the briefing asked for):

| File | Lines | Notes |
|---|---|---|
| `crates/armadai/tests/gaveldrop.rs` | 537 | Replaces `harness.rs` (441) + `mod.rs` (7) + `e2e.rs` (8) = 456 lines of the old project-writing/command-building glue — but **also absorbs** the conformance test, the suite-run test, and 7 unit tests the old harness didn't have as isolated units (it tested indirectly through full runs). The pure "port of `harness.rs`'s logic" (`write_project`/`project_yaml`/`agent_markdown`/`build_command`/`run_in_iso`) is ~330 lines; the remainder (~200) is new test surface. |
| `crates/armadai-fake/src/lib.rs` | 375 | A lateral move from `src/bin/fake-claude.rs` (360 lines, deleted) — the scenario engine ported near-verbatim onto `gaveldrop-fake`'s `Counter`/`Journal`/`Invocation` primitives (T1: "Ported... verbatim... with two intentional changes"). Not new logic; a re-platform of existing logic. |
| `gaveldrop.yaml` | 19 | New — project-level config (cases glob, invariants, events, fake bins). |
| 9 case files (`tests/cases/*.yaml`) | 451 total | Migrated test data — T2's transform was purely mechanical (move `fake:` under `setup.scenario:`, rename one invariant); content otherwise byte-for-byte identical to the pre-migration cases. Not new authorship. |

**Honest framing.** The brief's own target was "the ~300-line adapter + config" for the *genuinely
new* glue. That target is roughly met: `write_project`/`project_yaml`/`agent_markdown`/
`build_command`/`run_in_iso` (the part of `gaveldrop.rs` that is actually new adapter logic, as
opposed to tests) plus `gaveldrop.yaml` land in the 300–350 line range. The larger headline
numbers above (537, 375) are inflated by two things that are not scope creep: (a) `gaveldrop.rs`
carries its own test suite (conformance test + suite-run test + 7 unit tests) inline, which the
old harness didn't need because it tested itself through whole e2e runs; (b) `armadai-fake`'s 375
lines are a **port**, not new logic — the fake-response engine existed before at 360 lines and
still exists now, just retargeted onto gaveldrop's counter/journal primitives.

**The actual win, stated plainly**: **1138 lines of test-engine machinery** (case-loading,
verdict evaluation, report rendering — the parts that have nothing to do with armadai specifically)
are now gaveldrop's problem, not armadai's. armadai kept authorship only of what's inherently
armadai-specific: how to write an armadai project, how to invoke the armadai binary, and how to
fake `claude`'s stream-json output.

---

## 10. Anything reimplemented rather than delegated

**Nothing.** Every piece of gaveldrop's stated responsibility — case discovery/loading (`Config`,
`Case::load`), isolation (`Isolation::prepare_with`, env/clear_env/root/journal/file-change
tracking), scheduling/sharding (`select`), verdict evaluation (`verdict::evaluate_in`, the event
subsequence/count checks, the four/five named invariant shapes), and reporting (`Report`,
`Summary`, the `Sink` trait and its implementations) is used as gaveldrop provides it, with zero
duplicated logic on the armadai side. The one candidate for "reimplementation" the finding doc
flagged as a risk — armadai having to re-derive `run_one`'s loop (discovery → isolate → invoke →
evaluate → report) from the public pieces, because `run_all`/`run_all_selected` couldn't take a
custom adapter — never happened, because G1 was fixed upstream before T4–T6 were built. What
remains on armadai's side (§9 above) is exclusively armadai-domain logic gaveldrop was never
meant to own: writing an `armadai.yaml` project, building `armadai run …` argv, rendering a fake
`claude` response, and generating `## Metadata`/`## System Prompt` agent Markdown.

---

## Appendix: evidence index (file:line)

**armadai** (branch `feat/gaveldrop-migration`):
- `crates/armadai/tests/gaveldrop.rs:38-73` (`run_in_iso`, the shared F3 exit), `:311-335`
  (`Armadai::invoke`, both branches), `:338-343` (`config_loads`, confirms 5 invariants),
  `:407-436` (`e2e_suite_passes_through_gaveldrop`, the G1 decisive gate + vacuous-pass guard),
  `:368-400` (`as_armadai_probe` + `armadai_adapter_is_conformant`, F3 in practice).
- `crates/armadai-fake/src/lib.rs:20-25` (`SCENARIO_ENV` divergence from `GAVELDROP_SCENARIO`),
  `:163-209` (`run()`, the ported engine).
- `crates/armadai-fake/Cargo.toml:10-24` (the `engine` feature gate, §7).
- `gaveldrop.yaml:1-19` (project config, F2/F4 in the actual YAML).
- `.github/workflows/ci.yml:94-107` (the stale e2e-report upload step, §6).
- `.superpowers/sdd/2026-07-30-gaveldrop-migration/task-T{1..6}-report.md` (ground truth for what
  was built and what each task found).
- `docs/superpowers/gaveldrop-migration-logbook.md`, `docs/superpowers/gaveldrop-adapter-injection-finding.md`
  (the running record this report synthesizes).
- `git show 2cb3f52:crates/armadai/tests/e2e/{runner,report,case,harness,mod}.rs`,
  `git show 2cb3f52:crates/armadai/tests/e2e.rs` (the deleted harness, for line counts and the
  pre-migration `check_events_order_and_fields`/`check_event_counts`/`prov_model_non_empty`/
  `check_storage` implementations cited above).

**gaveldrop** (working tree `6d896b8`):
- `crates/gaveldrop/src/runner.rs:22-52` (`run_all`/`run_all_selected`, now thin delegates),
  `:88-118` (`run_all_with`, the G1 fix), `:123-160` (`run_one`, still private — correctly so).
- `crates/gaveldrop/src/adapters.rs:24-36` (`Adapter` trait + `registry()`).
- `crates/gaveldrop/src/case.rs:19-30` (`Case`, `deny_unknown_fields`), `:35` (`fake: Option<Scenario>`),
  `:80-116` (`Setup`, opaque `extra`, the `env` field added by #75/`6d896b8`).
- `crates/gaveldrop-fake/src/rule.rs:11-33` (`Match`, no `deny_unknown_fields`, the F2 rationale),
  `:340-370` (`the_published_key_lists_are_what_serde_understands`, the F2 fix's own test).
- `crates/gaveldrop/src/verdict/events.rs:75-124` (`check_subsequence`/`check_counts`).
- `crates/gaveldrop/src/verdict/invariants.rs:17-51` (`InvariantShape`, the F4 rationale in the
  `FieldNonEmpty` doc comment).
- `crates/gaveldrop/src/report.rs:151-183` (`Sink` trait, `Tee`), `crates/gaveldrop/src/report/html.rs:1-9`
  (`Html` sink, §6).
- `crates/gaveldrop-conformance/src/lib.rs:13` (the kit's own "not about how a subject is invoked"
  docstring, §3).
- `git log --oneline 9ed05ec..6d896b8` (the drift catalog in §8, in order after the logbook's
  `9ed05ec` baseline: `0f4b28c` #72, `38ec703` #73, `5b4c7d5` #74, `6d896b8` #75 — the last of
  which added `Setup.env`).
