# Logbook — armadai e2e → gaveldrop migration (feed to a gaveldrop agent)

**Audience:** an agent working in `~/work/misc/gaveldrop` (its own AGENTS.md / CONTRIBUTING / tests govern).
**Author:** the armadai agent doing `gaveldrop/docs/superpowers/briefing-armadai-integration.md` — armadai is gaveldrop's **first real consumer**.
**Purpose:** a running record of what the migration needs from gaveldrop. The briefing's own premise: *"every previous technology added to gaveldrop revealed a defect that nothing else had found."* This logbook is the catalog of those. Each item is precise enough to action in gaveldrop with its own test + invariant. The armadai side is being built in parallel and does not modify gaveldrop.

Cross-refs: full write-up of G1 is committed in armadai at `docs/superpowers/gaveldrop-adapter-injection-finding.md`. gaveldrop @ `cd85141` when these were found.

---

## Status snapshot (2026-07-29)

- ✅ Both codebases mapped (adapter surface, fake-claude, 9 cases, invariants, isolation API).
- ✅ Dep-path compat confirmed: gaveldrop toolchain `1.97`, armadai on rustc 1.97.1; `gaveldrop-fake` deps (serde/serde_json/serde_yaml_ng/thiserror + tiny_http/schemars) don't clash.
- 🔴 **Blocked on G1**: the suite cannot run its cases through a custom adapter (see G1). Deliverables #4 (9 cases green) and #5 (delete 1655-line harness) wait on it.
- 🟠 **F2 / F3**: two more frictions that shape the armadai adapter; each is either a gaveldrop change or a documented divergence — decide per item below.
- 🟡 Armadai side buildable now (adapter, own Rule/Match, fake-claude on gaveldrop-fake, gaveldrop.yaml, cases) — not started pending F2/F3 resolution.

---

## FINDINGS

### G1 — the public runner cannot inject a consumer adapter  🔴 blocking

**Symptom.** `run_all` / `run_all_selected` (the only public runner entries) hardcode `let adapters = adapters::registry();` (`crates/gaveldrop/src/runner.rs:31`), where `registry()` is a fixed `vec![Web, Shell, Process]` (`crates/gaveldrop/src/adapters.rs`). `run_one`, the only fn that takes `&adapters`, is **private** (`runner.rs:65`). No `run_*_with_adapters`, no `register_adapter`, no extensible registry.

**Impact.** A project whose cases carry keys no built-in claims (armadai's `pattern`) cannot run its suite through gaveldrop. The conformance kit *does* take an adapter (`run_with`), so the adapter is provable in isolation, but the runner that actually executes cases + produces a `Report` cannot use it. The two escape hatches both defeat the task: rewrite cases to `run:`+`fake.render` (briefing forbids; kills readability + never exercises the adapter API), or reimplement `run_one`'s loop from the public pieces (the "reimplementation gaveldrop already does" the briefing says to flag).

**Fix (minimal, additive, no behavior change for existing callers).**
```rust
// crates/gaveldrop/src/runner.rs
pub fn run_all_with_adapters(
    config: &Config, root: &Path, fake_binary: &Path, sink: &mut dyn Sink,
    shard: Option<Shard>, only: Option<&str>, adapters: &[Box<dyn Adapter>],
) -> Result<Report, ConfigError> {
    // current body of run_all_selected, using the passed `adapters`
}
pub fn run_all_selected(/* unchanged sig */) -> Result<Report, ConfigError> {
    run_all_with_adapters(config, root, fake_binary, sink, shard, only, &adapters::registry())
}
```
Consumers include `registry()` in the slice to keep the built-ins; the `gaveldrop` CLI (`locate_fake()` → `gaveldrop-fake`) is unchanged. A custom-adapter consumer runs its suite from a Rust test calling `run_all_with_adapters` with its own `fake_binary` — matches the "cargo test --workspace" model.

**gaveldrop's own test (its invariant).** Mirror `shell.rs`'s guard: a trivial `Echo` adapter that `claims` cases with an `echo` key and writes a marker to `stdout`; a case carrying `echo:` that no built-in claims; `run_all_with_adapters(..., &[Box::new(Echo)])`; assert the `Report` reflects `Echo`'s invoke (marker present) — proving the injected adapter claimed + ran through the public runner. Invariant locked: *a consumer-provided adapter that claims a case is the one that invokes it, through the public runner.*

---

### F2 — `Case.fake` is gaveldrop's `Scenario`; a consumer's fake vocabulary silently can't live there  🟠

**Symptom.** `Case` is `#[serde(deny_unknown_fields)]` (`crates/gaveldrop/src/case.rs:20`) with `fake: Option<Scenario>` where `Scenario == gaveldrop_fake::Scenario`. armadai's cases carry a fake block with `match: { agent: … }` + `respond: "…"`. Neither `gaveldrop_fake::Match` (`rule.rs:19`, no `deny_unknown_fields` — deliberately, so a project can flatten it) nor `Response` denies unknown fields, so `agent`/`respond` **parse without error but are dropped** if the block lands in `Case.fake`. And `Case`'s `deny_unknown_fields` forbids adding an alternative top-level key (e.g. `fake_armadai:`).

**Impact.** armadai owns its fake vocabulary (briefing §2/§3: define your own `Rule`/`Match` with `#[serde(flatten)] gaveldrop_fake::Match` + `agent`; keep `respond:`). But it has nowhere at the top level to put it — the only place a consumer's opaque block survives is `setup.extra` (the `#[serde(flatten)] BTreeMap<String, Value>` on `Setup`, `case.rs:96`). So armadai must nest its fake under `setup` (e.g. `setup.fake:`), and the adapter deserializes `setup.extra["fake"]` into armadai's own scenario type at invoke time. This **diverges from the briefing's example**, which shows `fake:` as a top-level sibling of `setup`/`expect` (as gaveldrop's own cases do).

**Decision needed (gaveldrop side).** Either (a) accept the divergence: armadai's fake lives under `setup.extra`, gaveldrop unchanged, and this is documented as "consumer fakes go in setup.extra, not the top-level `fake:`"; or (b) gaveldrop makes the top-level `fake:` consumer-extensible — e.g. `Case.fake` typed as opaque `Option<Value>` that each adapter interprets (loses gaveldrop's own Scenario typing/validation at the Case layer), or a documented "if you own the fake, put it under `setup`" convention baked into the schema + error messages. **Recommendation:** (a) for now (armadai proceeds), but gaveldrop should decide whether the top-level `fake:` is reserved for gaveldrop's Scenario or open to consumers — right now it silently eats a consumer's fields, which violates gaveldrop's own "a failure is diagnosable" property (this is a *silent* loss, not a loud refusal).

**If (a):** no gaveldrop change; armadai-side only. **If (b):** gaveldrop change + a test that a consumer fake vocabulary either round-trips or is loudly refused — never silently dropped.

---

### F3 — the conformance kit requires the adapter's subject to be an arbitrary shell script  🟠

**Symptom.** `gaveldrop_conformance`'s six checks each call the factory with a concrete shell script and require the *subject* to execute it (`crates/gaveldrop-conformance/src/checks.rs`):
- `exit_code_is_reported` → `how("exit 7")`, asserts `seen.exit == 7`.
- `both_streams_are_reported` → `how("echo out; echo err >&2")`.
- `the_home_directory_is_the_isolated_one` → `how("printf %s \"$HOME\"")`.
- `a_cleared_variable_does_not_reach_the_subject` → `how("printf %s \"${XDG_CONFIG_HOME-absent}\"")`.
- `files_written_are_reported` → `how("printf hello > written.txt")`.
- `an_unexpected_call_reaches_the_catch_all` → `how("conformance-probe-tool || true")`.

The `shell` adapter satisfies these because *its subject is the script* (`bash -c "eval <script>"`). armadai's adapter's subject is `armadai run <fleet>` — a purpose-built invocation that **cannot** be made to `exit 7`, print `$HOME`, write `written.txt`, or call `conformance-probe-tool`.

**Impact.** An adapter whose subject is a fixed program (not a user-supplied script) cannot pass the conformance kit through its real invocation path. armadai's only options are: (i) give its adapter a conformance-only branch — if the case is a conformance-probe (a distinct `extra` key), `invoke` runs `sh -c <script>` through the *same isolation-plumbing helper* it uses for `armadai run`, otherwise it runs the fleet. This proves the isolation contract (env applied, exit captured, streams apart, files/journal reported) — which is exactly what the 6 checks verify — without pretending the fleet ran the script. Or (ii) declare the kit unsatisfiable for fixed-subject adapters.

**Decision needed (gaveldrop side).** The kit's docstring says *"The checks are about the isolation contract, not about how a subject is invoked. An adapter that takes no `run:` … must still be checkable"* (`lib.rs:13`) — so the intent is exactly to support non-`run:` adapters. But every check still *requires a script executor*. For a purpose-built adapter, the honest way to keep the kit meaningful is to test the adapter's **isolation-plumbing helper directly** (the shared code that applies `iso.env()`/`iso.cleared()`/`iso.root()`, captures exit/streams, reads the journal, reports `iso.changes()`), which is what conformance is really asserting. **Recommendation:** armadai exposes that helper and the factory drives *it* (so the conformance subject is a script run through armadai's isolation helper, not `armadai run`). Document this pattern in `docs/conformance.md` as "purpose-built adapters: factor the isolation plumbing into a helper the factory exercises." If gaveldrop wants the kit to natively support fixed-subject adapters, that is a gaveldrop change (a check mode that doesn't presuppose a script subject).

**Workaround armadai will use (no gaveldrop change required):** factor the isolation-plumbing into a private helper `run_in_iso(command, case, iso) -> Observations`; `invoke` builds `armadai run …` and calls it for real cases; the conformance factory `as_armadai(script)` produces a case whose `extra` carries the script, and armadai's adapter runs `sh -c <script>` through the *same helper* for those. Conformance then proves the plumbing, which is the point.

---

## Gaveldrop change requests (prioritised, for the gaveldrop agent)

1. **G1 (do first — unblocks everything):** add `run_all_with_adapters` (spec + test above). Small, additive.
2. **F2 (decide):** is the top-level `fake:` reserved for gaveldrop's `Scenario`, or open to consumers? At minimum, make a consumer fake vocabulary either round-trip or be *loudly* refused — never silently dropped (violates the "diagnosable failure" property). armadai works around via `setup.extra` regardless.
3. **F3 (decide/document):** either document the "purpose-built adapter → isolation-plumbing helper the factory exercises" pattern in `docs/conformance.md`, or add a conformance mode that doesn't presuppose a shell-script subject. armadai works around via the shared-helper pattern regardless.

Items 2 and 3 do **not** block armadai; only G1 does.

---

## Armadai-side plan (built in parallel, `~/work/misc/armadai`, does not touch gaveldrop)

Target: replace `crates/armadai/tests/e2e/` (1655 lines) with an adapter + `gaveldrop.yaml` + the 9 cases. Deletion + suite-run wait on G1.

1. Add gaveldrop crates as **path dev-deps** (`gaveldrop`, `gaveldrop-fake`, `gaveldrop-conformance`) in `crates/armadai/Cargo.toml` — by path (briefing: never by version).
2. **`fake-claude` on `gaveldrop-fake` (library):** `Scenario::from_env`/`Invocation::from_env`/`Counter::next`/`Scenario::select`/`Journal::record` for the engine; armadai's binary renders the two-line Claude Code `stream-json` (assistant text + `result`, `respond` emitted twice, defaults `fake-model`/0/0.0 — preserve byte-for-byte from today's `src/bin/fake-claude.rs`). Env moves from `FAKE_SCENARIO`/`FAKE_STATE_DIR` to gaveldrop-fake's `GALDROP_*` (`SCENARIO`/`STATE`/`JOURNAL`). **This is atomic with the case migration** — the old harness drives fake-claude via `FAKE_SCENARIO`, so both flip together.
3. **`Armadai` adapter** (`impl gaveldrop::Adapter`): `claims` = `setup.extra.contains_key("pattern")`; `invoke` writes the project (agents/, armadai.yaml with the hierarchical `orchestration:` block + `defaults.orchestration.token_budget`, `agents/<n>.md` with the `FAKE_AGENT_ID:` marker + `provider: claude`/`model: fake-model`), builds `armadai run <agents[0]> <input> [--pipe <rest>] [--orchestrate blackboard|ring] <flags>` (⚠️ `--pipe` before `--orchestrate`; `hierarchical` = no flags, config-driven), runs it through the shared isolation helper (F3), returns `Observations { exit, stdout, stderr, calls: Journal::read(iso.journal_path()), files: iso.changes(), events: empty }`.
4. **armadai's fake types**: own `Rule`/`Match` (`#[serde(flatten)] gaveldrop_fake::Match` + `agent`, plus `tokens_in/out`, `cost`, `model`, `latency_ms`, `exit_code`), keep `respond:` shorthand (document the divergence from gaveldrop's `stdout:`); reuse `gaveldrop_fake::{Counter, Journal, Call}` unchanged; call `require_catch_all` at load; do NOT use `Scenario::select`-tied-to-Match — armadai's matcher includes `agent`.
5. **`gaveldrop.yaml`**: `cases: crates/armadai/tests/cases/**/*.yaml`, `events: { type_field: t }`, the four named invariants (exact YAML — `agent_start_end_symmetric: {shape: paired, start: agent_start, end: agent_end, key: agent}`, `single_result: {shape: exactly_one, type: result}`, `prov_model_non_empty: {shape: field_non_empty, type: agent_start, field: prov}` ⚠️ but armadai's check requires BOTH `prov` AND `model` non-empty on `agent_start` — gaveldrop's `field_non_empty` takes ONE field, so this needs TWO named invariants or a gaveldrop extension — **candidate F4, verify**), `fake: { bins: [claude] }`, `clear_env: [ARMADAI_CONFIG_DIR]`.
6. **Cases**: migrate the 9 `tests/e2e/cases/*.yaml` — move `pattern/agents/flags/input` under `setup`, `fake` under `setup.extra` (F2), keep `expect.{exit_code,events,event_counts,invariants}` as-is. Same verdicts.
7. **Conformance test**: factory + guard test + `run_with(&Armadai, &fake_claude, &as_armadai)` (works pre-G1).
8. **Suite-run test** (post-G1): `run_all_with_adapters(Config::load("gaveldrop.yaml"), root, fake_claude, sink, None, None, &[Box::new(Armadai)])`, assert report; then delete `tests/e2e/`.
9. **Keep** `tests/e2e/hook_stdout.rs`'s two `#[test]`s (separate concern — the `armadai __claude-register-session` stdout contract; NOT part of the case harness). Move to its own test file so deleting the e2e dir doesn't lose it.

### ⚠️ Candidate F4 (verify while building)
armadai's `prov_model_non_empty` invariant requires **both** `prov` AND `model` non-empty on every `agent_start` (`runner.rs:176-188`). gaveldrop's `field_non_empty` shape checks **one** field per invariant. So it maps to *two* named invariants (`prov_non_empty` + `model_non_empty`) — a small semantic split, or a gaveldrop extension to `field_non_empty` (multiple fields). Confirm the exact `agent_start` field names armadai emits (`prov`, `model`) and split accordingly. Not blocking; note in the report.

---

## Open questions / to confirm next
- F4 above (two-field non-empty invariant).
- Does the ordered-subsequence `expect.events` in gaveldrop match armadai's runner semantics exactly (cursor-advance, subset field match)? gaveldrop's `check_subsequence` (`events.rs:75`) vs armadai's `check_events_order_and_fields` (`runner.rs:81`) — verify identical before trusting green.
- `weight` semantics: armadai's report uses weighted score; gaveldrop's `GateConfig`/`Report` — confirm the weighted-score gate is preserved (CI may read `e2e-report.json`; gaveldrop writes its own report — check the CI artifact contract in `.github/workflows/ci.yml` still holds or is updated).
- storage checks (`expect.storage`) are dormant (no case uses them) but the field exists — gaveldrop has no equivalent; defer, keep out of scope.
