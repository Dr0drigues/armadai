# Gaveldrop Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace armadai's ~1655-line hand-rolled e2e harness with a thin `gaveldrop::Adapter` + a `gaveldrop.yaml` + 9 migrated cases, delegating discovery/isolation/evaluation/reporting to gaveldrop, while armadai becomes gaveldrop's first real consumer.

**Architecture:** gaveldrop (external YAML test engine at `~/work/misc/gaveldrop`, path dep pinned at `9ed05ec`) owns isolation, event extraction (`EventsConfig.type_field: t`), subsequence/count matching, named invariants, and reporting. armadai supplies: (1) an `Armadai` adapter that turns a case's `setup` into an `armadai run …` invocation inside gaveldrop's `Isolation`; (2) `fake-claude` rebuilt on the `gaveldrop-fake` library (Counter + Journal + env), keeping its Claude Code `stream-json` byte shape; (3) config + cases. Two lots: **Lot A** is purely additive (new files, old e2e untouched, everything compiles and passes side by side); **Lot B** is the atomic flip (rebuild `fake-claude`, add the suite-run test, delete the old harness, update CI).

**Tech Stack:** Rust edition 2024; `gaveldrop`, `gaveldrop-fake`, `gaveldrop-conformance` (path deps); `serde`/`serde_yaml_ng`/`serde_json`; `assert_cmd`.

## Global Constraints

- **NEVER modify `~/work/misc/gaveldrop`.** Depend on it by **path**, pinned to revision `9ed05ec`. Any needed gaveldrop change is a written finding, not an edit.
- Branch: `feat/gaveldrop-migration`. **Do NOT merge to `master`** until gaveldrop ships a definitive/released version (then path dep → version dep). Both lots live on this branch.
- French for all user communication; **English** for code, comments, commit messages.
- Commit trailer: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Local gate before pushing (all must pass): `cargo fmt --all`; clippy in 3 modes (`--no-default-features --features tui`; `…tui,providers-api`; `…tui,web,storage`) with `-D warnings`; `cargo test --no-default-features --features tui` and `…tui,storage`.
- **`fake-claude` must emit exactly two `stream-json` lines, byte-identical to today**: an `assistant` line `{"type":"assistant","message":{"content":[{"type":"text","text":<respond>}]}}` and a `result` line `{"type":"result","subtype":"success","is_error":false,"result":<respond>,"total_cost_usd":<cost|0.0>,"usage":{"input_tokens":<tokens_in|0>,"output_tokens":<tokens_out|0>},"modelUsage":{<model|"fake-model">:{"outputTokens":<tokens_out|0>}}}`. `respond` appears twice. Defaults: `model`→`"fake-model"`, tokens→0, cost→0.0.
- The released `armadai` binary crate must NOT pull `gaveldrop-fake`/`gaveldrop` into a default `cargo build --release`. External gaveldrop deps enter only under the `e2e-fake` feature (bin) and as dev-deps (test).
- **All 9 migrated cases must PASS** (none is `allow_fail`) with verdicts identical to the current harness.
- Keep `tests/e2e/hook_stdout.rs`'s two tests (armadai's `__claude-register-session` stdout contract) — they are NOT part of the e2e suite and must survive the deletion.

---

## File Structure

**New:**
- `crates/armadai-fake/` — new lib crate. `src/lib.rs` = shared fake scenario types (`Scenario`/`Rule`/`Match`) + `select_response` + `emit_claude_jsonl` + `run()` (the engine, built on `gaveldrop_fake::{Counter, Journal, env}`). Isolates the external `gaveldrop-fake` dep away from the shipped `armadai`/`armadai-core` crates.
- `crates/armadai/tests/gaveldrop.rs` — the new integration test target: the `Armadai` adapter, the shared `run_in_iso` helper, the conformance test, and the suite-run test. Feature-gated `#[cfg(feature = "e2e-fake")]`.
- `crates/armadai/tests/cases/*.yaml` — the 9 migrated cases (gaveldrop format).
- `gaveldrop.yaml` — repo-root gaveldrop config.
- `docs/superpowers/gaveldrop-defects-report.md` — the deliverable defects/divergence report.

**Modified:**
- `crates/armadai/Cargo.toml` — `e2e-fake` feature, optional `armadai-fake` dep, `fake-claude` bin `required-features`, gaveldrop dev-deps, `[[test]] name="gaveldrop"`.
- `crates/armadai/src/bin/fake-claude.rs` — becomes a thin `fn main() { armadai_fake::run() }`.
- `Cargo.toml` (workspace root) — add `crates/armadai-fake` to `members`.
- `.github/workflows/ci.yml` — enable `e2e-fake` in the e2e test job; swap the report artifact.

**Deleted (Lot B):** `crates/armadai/tests/e2e.rs`, `tests/e2e/{mod,harness,runner,case,report}.rs`, `tests/e2e/cases/*.yaml`. **Kept:** `tests/e2e/hook_stdout.rs` (rehomed).

---

## LOT A — Additive (old e2e untouched)

### Task A1: `armadai-fake` crate + `fake-claude` on gaveldrop-fake

**Files:**
- Create: `crates/armadai-fake/Cargo.toml`
- Create: `crates/armadai-fake/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`)
- Modify: `crates/armadai/Cargo.toml` (`e2e-fake` feature, optional dep, bin `required-features`)
- Rewrite: `crates/armadai/src/bin/fake-claude.rs`

**Interfaces:**
- Produces (consumed by A2, A5, B1): `armadai_fake::{Scenario, Rule, Match, select_response, emit_claude_jsonl, run, SCENARIO_ENV}`.
  - `pub struct Scenario { pub rules: Vec<Rule> }` — `Serialize + Deserialize + Clone`.
  - `pub struct Rule { #[serde(rename="match", default)] pub match_: Match, pub respond: String, pub tokens_in: Option<u32>, pub tokens_out: Option<u32>, pub cost: Option<f64>, pub model: Option<String>, pub latency_ms: Option<u64>, pub exit_code: Option<i32> }` — all metric fields `#[serde(default)]`.
  - `pub struct Match { pub agent: Option<String>, pub call: Option<u32>, pub prompt_contains: Option<String> }` — `Default`, all `#[serde(default)]`.
  - `pub fn select_response<'s>(scenario: &'s Scenario, agent: &str, call: u32, prompt: &str) -> Option<&'s Rule>` (returns `None` on no match — the engine decides what to do, see below).
  - `pub fn emit_claude_jsonl(rule: &Rule) -> String` — the two `stream-json` lines joined by `\n` (see Global Constraints for the exact bytes).
  - `pub fn run()` — the binary entry point (reads env, journals, selects, emits, exits).
  - `pub const SCENARIO_ENV: &str = "ARMADAI_FAKE_SCENARIO";`

**Why the scenario env is a dedicated var, not `GAVELDROP_SCENARIO`:** gaveldrop's `Isolation` always writes a fallback catch-all scenario in *its* vocabulary and points `GAVELDROP_SCENARIO` at it (even when `case.fake` is `None`). armadai's scenario lives under `setup:` (opaque to gaveldrop), so the adapter (A2) writes it and points `fake-claude` at it via `ARMADAI_FAKE_SCENARIO`. But state and journal DO come from gaveldrop: `Counter::from_env()` reads `GAVELDROP_STATE`, `Journal::from_env()` reads `GAVELDROP_JOURNAL` — both set by `Isolation` and propagated to the `claude` subprocess.

- [ ] **Step 1: Create the workspace member + crate manifest**

Add `"crates/armadai-fake"` to the `members` array in the root `Cargo.toml`.

`crates/armadai-fake/Cargo.toml`:
```toml
[package]
name = "armadai-fake"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
gaveldrop-fake = { path = "../../../gaveldrop/crates/gaveldrop-fake" }
serde = { version = "1", features = ["derive"] }
serde_yaml_ng = "0.10"
serde_json = "1"

[dev-dependencies]
tempfile = { workspace = true }
```
(Verify the exact relative path to `gaveldrop-fake` from `crates/armadai-fake/`: run `ls ../../../gaveldrop/crates/gaveldrop-fake/Cargo.toml` from that dir; the repo sits at `~/work/misc/armadai`, gaveldrop at `~/work/misc/gaveldrop`, so `../../../gaveldrop/...` is correct. Confirm `serde_yaml_ng`/`serde_json` versions match what the workspace already uses — copy from `crates/armadai/Cargo.toml`.)

- [ ] **Step 2: Write `armadai-fake/src/lib.rs` — types + select + emit (ported verbatim)**

Port `Scenario`/`Rule`/`Match`/`select_response` (returning `Option`)/`emit_claude_jsonl` from the current `crates/armadai/src/bin/fake-claude.rs` (lines 18-154). Keep the field shape EXACTLY (this is the on-wire scenario format the adapter serializes and the cases carry). Also port `agent_id_from_prompt` (lines 61-74). Add the `run()` engine:

```rust
use gaveldrop_fake::{Counter, Journal, Invocation};

/// Binary entry point: journal the call, select a rule, emit stream-json, exit.
pub fn run() {
    let inv = Invocation::from_env(false);
    let prompt = std::env::args().next_back().unwrap_or_default();
    let agent = agent_id_from_prompt(&prompt).unwrap_or_else(|| "unknown".to_string());

    // Per-agent 1-indexed call rank, persisted under GAVELDROP_STATE.
    let counter = Counter::from_env().expect("GAVELDROP_STATE must be set by isolation");
    let call = counter.next(&agent).unwrap_or(1);

    // The scenario armadai's adapter wrote (NOT GAVELDROP_SCENARIO — see module doc).
    let scenario: Option<Scenario> = std::env::var(SCENARIO_ENV).ok().map(|path| {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fake-claude: cannot read {SCENARIO_ENV} {path}: {e}"));
        serde_yaml_ng::from_str(&raw)
            .unwrap_or_else(|e| panic!("fake-claude: invalid {SCENARIO_ENV} YAML {path}: {e}"))
    });

    let journal = Journal::from_env().ok();
    let rule = scenario.as_ref().and_then(|s| select_response(s, &agent, call, &prompt));

    match rule {
        Some(rule) => {
            if let (Some(j), false) = (&journal, false) {
                let _ = j; // journaled below regardless
            }
            if let Some(j) = &journal {
                let call_line = gaveldrop_fake::Call::from_invocation(
                    &inv, call, &agent, /*catch_all*/ false, /*passthrough*/ false,
                    rule.exit_code.unwrap_or(0),
                );
                let _ = j.record(&call_line);
            }
            if let Some(ms) = rule.latency_ms {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            println!("{}", emit_claude_jsonl(rule));
            std::process::exit(rule.exit_code.unwrap_or(0));
        }
        None => {
            // No scenario (conformance probe) or no matching rule: journal a catch-all
            // and exit non-zero WITHOUT stream-json. This is what makes fake-claude usable
            // as the conformance kit's fake binary, and it turns a scenario-authoring gap
            // (a missing catch-all rule) into an observable failed call rather than a panic.
            if let Some(j) = &journal {
                let call_line = gaveldrop_fake::Call::from_invocation(
                    &inv, call, &agent, /*catch_all*/ true, /*passthrough*/ false, 127,
                );
                let _ = j.record(&call_line);
            }
            std::process::exit(127);
        }
    }
}
```
Simplify the `Some(rule)` arm's dead `if let (Some(j), false)` block away — it is shown only to flag that journaling happens in both arms; the real code journals once. Confirm `Call::from_invocation`'s signature against `~/work/misc/gaveldrop/crates/gaveldrop-fake/src/journal.rs:36` (`inv, call, key, catch_all, passthrough, exit`) and `Invocation::from_env(read_stdin: bool)` against `invocation.rs:32`.

- [ ] **Step 3: Port the fake's unit tests into `armadai-fake`**

Move the `#[cfg(test)] mod tests` from the current `fake-claude.rs` (lines 201-359) into `lib.rs`, adjusting: `select_response` now returns `Option<&Rule>` so `.unwrap()`/`.is_none()` where the old code returned `&Rule`/panicked. Keep `emits_parseable_claude_jsonl`, `emits_defaults_when_metrics_absent`, `extracts_agent_id_from_prompt`, `selects_by_agent_and_call`, `selects_by_prompt_contains`, `next_call_count_increments_and_persists` (rewrite this last one against `gaveldrop_fake::Counter::new(dir).next("t-a")`).

- [ ] **Step 4: Run the armadai-fake tests**

Run: `cargo test -p armadai-fake`
Expected: PASS (all ported unit tests).

- [ ] **Step 5: Wire `crates/armadai/Cargo.toml`**

Add to `[features]`: `e2e-fake = ["dep:armadai-fake"]`.
Add to `[dependencies]`: `armadai-fake = { path = "../armadai-fake", optional = true }`.
Change the `fake-claude` `[[bin]]` block (currently lines 15-16) to add `required-features = ["e2e-fake"]`.

- [ ] **Step 6: Rewrite the bin as a thin shim**

`crates/armadai/src/bin/fake-claude.rs` becomes:
```rust
//! `fake-claude` — deterministic stand-in for the `claude` CLI, used by the gaveldrop
//! e2e suite. The engine lives in the `armadai-fake` crate (built on `gaveldrop-fake`);
//! this binary is only its entry point. Built only under the `e2e-fake` feature so a
//! default release build never pulls the external gaveldrop deps.
fn main() {
    armadai_fake::run();
}
```

- [ ] **Step 7: Verify feature gating both ways**

Run: `cargo build --release --no-default-features --features tui,storage` — MUST succeed and MUST NOT build `fake-claude` (confirm: no `gaveldrop-fake` in `cargo tree --no-default-features --features tui,storage -i gaveldrop-fake` — expect "package not found in dependency graph").
Run: `cargo build --no-default-features --features tui,storage,e2e-fake --bin fake-claude` — MUST build the bin.

- [ ] **Step 8: Commit**
```bash
git add crates/armadai-fake Cargo.toml crates/armadai/Cargo.toml crates/armadai/src/bin/fake-claude.rs
git commit -m "test(e2e): armadai-fake crate — fake-claude engine on gaveldrop-fake"
```

---

### Task A2: `Armadai` adapter + shared `run_in_iso` helper

**Files:**
- Create: `crates/armadai/tests/gaveldrop.rs`
- Modify: `crates/armadai/Cargo.toml` (dev-deps + `[[test]]`)

**Interfaces:**
- Consumes: `armadai_fake::{Scenario, SCENARIO_ENV}` (A1); gaveldrop's `Adapter`/`Case`/`Isolation`/`Observations`/`AdapterError` (see the exact shapes below).
- Produces (consumed by A5, B1): `struct Armadai;` implementing `gaveldrop::adapters::Adapter`; `fn run_in_iso(iso: &Isolation, argv: &[String]) -> Result<Observations, AdapterError>`.

**Exact gaveldrop shapes (verify at `~/work/misc/gaveldrop` before writing):**
- `Adapter` trait (`crates/gaveldrop/src/adapters.rs:18`): `fn claims(&self, case: &Case) -> bool;` and `fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError>;`
- `Case.setup.extra: BTreeMap<String, serde_json::Value>` (via `#[serde(flatten)]`, `case.rs:85`). `Case.setup.run: Option<Vec<String>>`.
- `Isolation` (`iso.rs`): `root() -> &Path`, `env() -> Vec<(String, OsString)>`, `cleared() -> &[String]`, `journal_path() -> PathBuf`, `changes() -> Vec<FileEffect>`.
- `Observations` (`observations.rs:14`): field is **`exit: i32`** (not `exit_code`); `stdout`, `stderr`, `calls: Vec<Call>`, `events` (leave EMPTY — the runner fills it), `files: Vec<FileEffect>`; construct with `..Default::default()`.

- [ ] **Step 1: Add dev-deps + the test target**

`crates/armadai/Cargo.toml` `[dev-dependencies]`:
```toml
gaveldrop = { path = "../../../gaveldrop/crates/gaveldrop" }
gaveldrop-fake = { path = "../../../gaveldrop/crates/gaveldrop-fake" }
gaveldrop-conformance = { path = "../../../gaveldrop/crates/gaveldrop-conformance" }
armadai-fake = { path = "../armadai-fake" }
```
Add:
```toml
[[test]]
name = "gaveldrop"
required-features = ["e2e-fake"]
```
(Verify the `../../../gaveldrop/...` relative paths resolve from `crates/armadai/`.)

- [ ] **Step 2: Write the shared `run_in_iso` helper**

At the top of `tests/gaveldrop.rs`, behind `#![cfg(feature = "e2e-fake")]`:
```rust
use std::path::Path;
use std::process::Command;
use gaveldrop::adapters::Adapter;
use gaveldrop::case::Case;
use gaveldrop::iso::Isolation;
use gaveldrop::observations::Observations;
use gaveldrop::adapters::AdapterError;

/// The single exit both branches of `invoke` funnel through. The conformance kit
/// certifies exactly the isolation plumbing this function performs, so if the real
/// armadai branch did not also end here the kit would be vacant (it would certify code
/// no case runs). Applies the isolation env, clears what isolation cleared, runs `argv`
/// in the isolated root, and reads back stdout/stderr/exit + the journal + file effects.
fn run_in_iso(iso: &Isolation, argv: &[String]) -> Result<Observations, AdapterError> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    for (k, v) in iso.env() {
        cmd.env(k, v);
    }
    for k in iso.cleared() {
        cmd.env_remove(k);
    }
    cmd.current_dir(iso.root());

    let output = cmd.output().map_err(|e| AdapterError::from(e))?;

    Ok(Observations {
        exit: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        calls: gaveldrop_fake::Journal::read(&iso.journal_path()).unwrap_or_default(),
        events: Vec::new(),
        files: iso.changes(),
        ..Default::default()
    })
}
```
Verify `AdapterError`'s variants / `From<std::io::Error>` at `adapters.rs`; if there is no `From<io::Error>`, use the constructor gaveldrop exposes (e.g. `AdapterError::invoke(...)` or `.map_err(|e| AdapterError::…(e.to_string()))`) — match whatever the built-in `Process` adapter uses in `adapters.rs`.

- [ ] **Step 3: Write helpers reading `setup.extra`**

```rust
fn str_field<'a>(case: &'a Case, key: &str) -> Option<&'a str> {
    case.setup.extra.get(key).and_then(|v| v.as_str())
}
fn arr_field(case: &Case, key: &str) -> Vec<String> {
    case.setup.extra.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}
```

- [ ] **Step 4: Write the project writer (ported from `harness.rs::write_project`/`project_yaml`/`agent_markdown`)**

Port `project_yaml` (harness.rs:185-245), `agent_markdown` (harness.rs:254-268) verbatim, reading fields from `setup.extra` instead of the old `Setup` struct: `pattern` via `str_field`, `agents` via `arr_field`, `nested_team` via `case.setup.extra.get("nested_team")` deserialized into a local `NestedTeam { lead, pattern, agents }` struct, `token_budget` via `.get("token_budget").and_then(|v| v.as_u64())`. Write `armadai.yaml` + `agents/<a>.md` into `iso.root()`. Write the scenario: serialize `setup.extra["scenario"]` (a `serde_json::Value`) into `armadai_fake::Scenario` (`serde_json::from_value`) then to YAML, write to `iso.root().join("armadai-scenario.yaml")`.

- [ ] **Step 5: Write the command builder (ported from `harness.rs::configure_invocation`)**

Return the argv `Vec<String>` for the real path. Port the flag logic (harness.rs:113-141): argv starts `[armadai_bin, "run", coordinator, input]`; then per pattern — `hierarchical` → nothing; `blackboard`/`ring` → `--pipe <rest…>` (if rest non-empty) then `--orchestrate <pattern>`; else → `--pipe <rest…>` only; then append `flags` verbatim. `armadai_bin = env!("CARGO_BIN_EXE_armadai").to_string()`.

- [ ] **Step 6: Implement `Adapter for Armadai`**

```rust
struct Armadai;

impl Adapter for Armadai {
    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("pattern")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        // Conformance-probe branch: a `probe_script` in setup.extra means the kit is
        // checking the isolation contract, not running a fleet. Same run_in_iso exit.
        if let Some(script) = str_field(case, "probe_script") {
            return run_in_iso(iso, &[
                "sh".into(), "-c".into(), script.into(),
            ]);
        }
        // Real branch: write the armadai project + scenario, point fake-claude at it,
        // build `armadai run …`, run through the SAME helper.
        write_project(case, iso).map_err(|e| /* AdapterError */ )?;
        let argv = build_command(case);
        // fake-claude reads ARMADAI_FAKE_SCENARIO; set it (adapter-owned, not GAVELDROP_*).
        // run_in_iso applies iso.env() then this must also be applied — so extend run_in_iso
        // OR set it here. Cleanest: pass extra env into run_in_iso (see note).
        run_in_iso_with_env(iso, &argv, &[(armadai_fake::SCENARIO_ENV, scenario_path)])
    }
}
```
Refactor `run_in_iso` to take an extra `env: &[(&str, PathBuf)]` slice (applied after `iso.env()`), and have the conformance branch pass `&[]`. This keeps ONE helper (the F3 condition) while letting the real branch add `ARMADAI_FAKE_SCENARIO`. Name it `run_in_iso` with that signature from the start; adjust Step 2 accordingly.

- [ ] **Step 7: Unit tests for the adapter (no real run)**

```rust
#[test]
fn claims_only_cases_carrying_a_pattern() {
    // build a Case with setup.extra {"pattern": "direct"} → claims == true
    // build a Case with empty extra → claims == false
}
#[test]
fn builds_pipe_before_orchestrate_for_ring() {
    // assert build_command(case_with pattern=ring, agents=[a,b,c]) ==
    //   [armadai, run, a, <input>, --pipe, b, c, --orchestrate, ring, <flags…>]
}
#[test]
fn hierarchical_passes_no_orchestration_flags() { /* … */ }
```

- [ ] **Step 8: Run + commit**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop`
Expected: the adapter unit tests PASS (no real cases yet).
```bash
git add crates/armadai/tests/gaveldrop.rs crates/armadai/Cargo.toml
git commit -m "test(e2e): Armadai gaveldrop adapter + shared run_in_iso helper"
```

---

### Task A3: `gaveldrop.yaml` config

**Files:**
- Create: `gaveldrop.yaml` (repo root)

**Interfaces:**
- Consumes: gaveldrop's `Config` schema (`config.rs`) — `cases`, `fake.bins`, `clear_env`, `events.type_field`, `invariants` (`NamedInvariants`).

- [ ] **Step 1: Write `gaveldrop.yaml`**
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Dr0drigues/gaveldrop/main/docs/case.schema.json
cases: crates/armadai/tests/cases/**/*.yaml

fake:
  bins: [claude]           # isolation shadows `claude` with fake-claude

clear_env: [ARMADAI_CONFIG_DIR]   # config_dir() checks this ahead of XDG/HOME

events:
  type_field: t            # armadai's --json stream keys events on `t`

invariants:
  agent_start_end_symmetric: { shape: paired, start: agent_start, end: agent_end, key: agent }
  single_result:             { shape: exactly_one, type: result }
  # F4 split: the old single `prov_model_non_empty` becomes TWO field_non_empty
  # invariants — field_non_empty keeps exactly one field, and this is a better
  # diagnostic (a missing `prov` and a missing `model` now fail distinctly).
  prov_non_empty:            { shape: field_non_empty, type: agent_start, field: prov }
  model_non_empty:           { shape: field_non_empty, type: agent_start, field: model }
  no_orphan_events:          { shape: no_orphan, key: agent, root: agent_start }
```
Verify each shape's exact parameter names against `~/work/misc/gaveldrop/crates/gaveldrop/src/verdict/invariants.rs` and the config test at `config.rs:437-451` (which shows `paired{start,end,key}`, `exactly_one{type}`, `field_non_empty{type,field}`, `no_orphan{key,root}`). Do NOT add a `gate:` block — a failing case already fails the run, and no threshold beyond "all pass" is wanted.

- [ ] **Step 2: Test the config parses**

Add to `tests/gaveldrop.rs`:
```rust
#[test]
fn config_loads() {
    let cfg = gaveldrop::config::Config::load(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../gaveldrop.yaml")));
    let cfg = cfg.expect("gaveldrop.yaml parses");
    assert_eq!(cfg.events.as_ref().unwrap().type_field, "t");
    assert_eq!(cfg.invariants.len(), 5);
}
```
Run: `cargo test …,e2e-fake --test gaveldrop config_loads`
Expected: PASS.

- [ ] **Step 3: Commit**
```bash
git add gaveldrop.yaml crates/armadai/tests/gaveldrop.rs
git commit -m "test(e2e): gaveldrop.yaml — events, five named invariants, faked claude"
```

---

### Task A4: Migrate the 9 cases

**Files:**
- Create: `crates/armadai/tests/cases/{direct,blackboard,budget-halt-visible,hierarchical,nested,no-tui,quiet-orchestrated,ring,ring-budget-reaches-vote}.yaml`

**Interfaces:**
- Consumes: gaveldrop's `Case` schema. Each source is `crates/armadai/tests/e2e/cases/<name>.yaml`.

**The transform (mechanical, applied to each of the 9):**
1. Move the whole `fake:` block UNDER `setup:` as `setup.scenario:` (gaveldrop's top-level `fake:` reads only its four criteria and REFUSES unknown keys like `respond`/`agent`; armadai's scenario is opaque data under `setup:`).
2. In `expect.invariants`, replace `prov_model_non_empty` with `prov_non_empty, model_non_empty` (F4 split).
3. Everything else is unchanged: `name`, `weight`, `setup.{pattern,agents,flags,input,nested_team,token_budget}`, `expect.{exit_code,events,event_counts}`. gaveldrop's `expect.events` (subsequence, subset fields) and `expect.event_counts` (exact) have identical semantics to the old harness (verified: `verdict/events.rs::check_subsequence`/`check_counts`).

- [ ] **Step 1: Migrate `direct.yaml` as the worked example**

Source `tests/e2e/cases/direct.yaml`. Result `tests/cases/direct.yaml`:
```yaml
name: direct
weight: 5
setup:
  pattern: direct
  agents: [t-writer]
  flags: ["--json"]
  input: "hi"
  scenario:
    rules:
      - match: { agent: t-writer }
        respond: "done"
      - match: {}
        respond: "unexpected — catch-all should never be hit in this case"
expect:
  exit_code: 0
  events:
    - { t: run_start }
    - { t: agent_start, agent: t-writer }
    - { t: result }
  event_counts: { agent_start: 1, agent_end: 1 }
  invariants: [agent_start_end_symmetric, prov_non_empty, model_non_empty, single_result]
```
(Adjust the exact `setup`/`expect` values to whatever the real `tests/e2e/cases/direct.yaml` contains — read it first. The shape above is the pattern.)

- [ ] **Step 2: Migrate the remaining 8** applying the same transform. For `nested.yaml` and `budget-halt-visible.yaml`, keep the rich comments — they document real engine behavior (the ES isolation boundary, the budget halt). `nested.yaml`'s `nested_team:` stays under `setup:` exactly as today (the adapter reads it from `setup.extra`).

- [ ] **Step 3: Test every case loads as a gaveldrop `Case`**

Add to `tests/gaveldrop.rs`:
```rust
#[test]
fn all_cases_load() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/cases");
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "yaml") {
            gaveldrop::case::Case::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            n += 1;
        }
    }
    assert_eq!(n, 9, "expected 9 migrated cases");
}
```
Run: `cargo test …,e2e-fake --test gaveldrop all_cases_load`
Expected: PASS (9 cases load; note any case gaveldrop refuses — a refusal is a finding for the report).

- [ ] **Step 4: Commit**
```bash
git add crates/armadai/tests/cases
git commit -m "test(e2e): migrate the 9 e2e cases to gaveldrop format"
```

---

### Task A5: Conformance test

**Files:**
- Modify: `crates/armadai/tests/gaveldrop.rs`

**Interfaces:**
- Consumes: `gaveldrop_conformance::run_with(adapter: &dyn Adapter, fake_binary: &Path, invocation: &Invocation) -> ConformanceReport` where `Invocation = dyn Fn(&str) -> Case`; `ConformanceReport::{is_conformant, render}`.

- [ ] **Step 1: Write the invocation factory**

The default `as_command_line` builds a `Case` with `run:` set and empty `extra` — `Armadai` would NOT claim it. Supply a factory that puts the script in `setup.extra["probe_script"]` and adds a `pattern` marker so `Armadai::claims` fires and the built-in `Process` adapter (which needs `run:`) does not:
```rust
fn as_armadai_probe(script: &str) -> Case {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert("pattern".to_string(), serde_json::json!("conformance"));
    extra.insert("probe_script".to_string(), serde_json::json!(script));
    Case {
        name: "conformance".into(),
        weight: 1,
        allow_fail: false,
        setup: gaveldrop::case::Setup { run: None, exec: None, extra },
        fake: None,
        expect: Default::default(),
        steps: Vec::new(),
    }
}
```
Verify `Setup`'s exact fields (`run`, `exec`, `extra`) and whether `Case`/`Setup`/`Expect` are constructible from the test crate (all `pub`). If a field is private, build the case by deserializing a small YAML string instead.

- [ ] **Step 2: Write the conformance test**
```rust
#[test]
fn armadai_adapter_is_conformant() {
    let fake = Path::new(env!("CARGO_BIN_EXE_fake-claude"));
    let invocation: gaveldrop_conformance::Invocation = as_armadai_probe; // or &as_armadai_probe
    let report = gaveldrop_conformance::run_with(&Armadai, fake, &as_armadai_probe);
    assert!(report.is_conformant(), "\n{}", report.render());
}
```
`fake-claude` is the conformance fake: with no `ARMADAI_FAKE_SCENARIO`, its `run()` journals a catch-all and exits 127 — exactly what the kit's catch-all check needs.

- [ ] **Step 3: Run**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop armadai_adapter_is_conformant`
Expected: PASS (`is_conformant()` true). If a check fails, `render()` names it — a genuine isolation gap in the adapter is a bug to fix; a gap in the KIT is a finding for the report.

- [ ] **Step 4: Commit**
```bash
git add crates/armadai/tests/gaveldrop.rs
git commit -m "test(e2e): armadai adapter passes the gaveldrop conformance kit"
```

**End of Lot A: the old `tests/e2e/` suite still runs and passes; the new adapter, config, cases, and conformance test all exist and pass, side by side.**

---

## LOT B — Atomic flip (delete the old harness)

### Task B1: Suite-run test via `run_all_with`

**Files:**
- Modify: `crates/armadai/tests/gaveldrop.rs`

**Interfaces:**
- Consumes: `gaveldrop::runner::run_all_with(&Config, &Path, &Path, &mut dyn Sink, Option<Shard>, Option<&str>, &[Box<dyn Adapter>]) -> Result<Report, ConfigError>`; `gaveldrop::report::terminal::Terminal::plain(W)`; `report.is_success()`; `report.summary().failed`.

- [ ] **Step 1: Write the suite-run test**
```rust
#[test]
fn e2e_suite_passes_through_gaveldrop() {
    use gaveldrop::adapters::{self, Adapter};
    use gaveldrop::report::terminal::Terminal;

    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let config = gaveldrop::config::Config::load(&root.join("gaveldrop.yaml")).unwrap();
    let fake = Path::new(env!("CARGO_BIN_EXE_fake-claude"));

    let mut chain: Vec<Box<dyn Adapter>> = vec![Box::new(Armadai)];
    chain.extend(adapters::registry());

    let mut sink = Terminal::plain(std::io::stdout());
    let report = gaveldrop::runner::run_all_with(&config, root, fake, &mut sink, None, None, &chain).unwrap();

    assert!(report.is_success(), "{} case(s) failed", report.summary().failed);
}
```
Verify `Config::load` returns `Result` (unwrap accordingly) and the exact module path of `Terminal`. `root` must be the repo root (where `gaveldrop.yaml` and the `cases:` glob resolve from) — `CARGO_MANIFEST_DIR` is `crates/armadai`, so `../..` is the workspace root.

- [ ] **Step 2: Run — the real gate**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop e2e_suite_passes_through_gaveldrop -- --nocapture`
Expected: PASS — all 9 cases green, verdicts identical to the old harness. If a case fails, diff its `expect` against the old harness's evaluation of the same stream; a genuine semantic divergence in gaveldrop is a finding (record it, do not paper over it by weakening the case).

- [ ] **Step 3: Commit**
```bash
git add crates/armadai/tests/gaveldrop.rs
git commit -m "test(e2e): run the 9-case suite through gaveldrop run_all_with"
```

---

### Task B2: Delete the old harness, keep `hook_stdout`

**Files:**
- Delete: `crates/armadai/tests/e2e.rs`, `tests/e2e/{mod,harness,runner,case,report}.rs`, `tests/e2e/cases/*.yaml`
- Create: `crates/armadai/tests/hook_stdout.rs` (rehomed from `tests/e2e/hook_stdout.rs`)
- Delete: `crates/armadai/tests/e2e/hook_stdout.rs` (after rehoming)

- [ ] **Step 1: Rehome `hook_stdout.rs`**

Read `tests/e2e/hook_stdout.rs`. It is a standalone test of armadai's `__claude-register-session` stdout contract — it does NOT depend on the harness. Move it to `tests/hook_stdout.rs` (a top-level integration test target). Fix any `use super::…`/module-path references (it currently lives under the `e2e` module tree via `tests/e2e.rs`). Confirm it compiles as its own target.

- [ ] **Step 2: Delete the harness files**
```bash
git rm crates/armadai/tests/e2e.rs
git rm -r crates/armadai/tests/e2e
```
(After Step 1 has copied `hook_stdout.rs` out. If `git rm -r tests/e2e` would also remove the not-yet-moved file, do Step 1's `git add tests/hook_stdout.rs` first.)

- [ ] **Step 3: Drop now-unused `[dev-dependencies]`**

`schemars` was only used by the old `case.rs` JSON-schema emitter. If nothing else uses it, remove `schemars` from `crates/armadai/Cargo.toml` `[dev-dependencies]`. Verify with `grep -rn schemars crates/armadai` first. Also delete the committed `docs/e2e-case.schema.json` if the schema-emitting test was its only producer (grep for other references first).

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake`
Expected: PASS — `gaveldrop` suite green, `hook_stdout` tests green, no reference to the deleted `e2e` target.
Run also: `cargo test --no-default-features --features tui` — PASS (the `gaveldrop` target is skipped without `e2e-fake`; `hook_stdout` still runs).

- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "test(e2e): delete the hand-rolled harness (~1655 lines), keep hook_stdout"
```

---

### Task B3: CI workflow update

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the current test job matrix (modes `tui` and `tui,storage`) and the `actions/upload-artifact` step for `e2e-report.{json,html}`.

- [ ] **Step 1: Read the current CI test + artifact steps**

Read `.github/workflows/ci.yml`. Find the test job(s) and the e2e-report artifact upload.

- [ ] **Step 2: Enable `e2e-fake` where the e2e suite runs**

The `tui,storage` test mode is where the old e2e target lived. Change its features to `tui,storage,e2e-fake` so the `gaveldrop` test target (and the `fake-claude` bin) build and run. Leave the `tui`-only mode as is (it correctly skips the gaveldrop target).

- [ ] **Step 3: Swap the report artifact**

gaveldrop writes its own reports (per `~/work/misc/gaveldrop/docs/ci.md` — JSONL/JUnit/HTML). The old `target/e2e-report/e2e-report.{json,html}` no longer exists. Either (a) drop the artifact-upload step, or (b) point it at gaveldrop's report output path. Read `docs/ci.md` to learn where gaveldrop writes reports and whether a flag is needed to emit them from `run_all_with` (the terminal `Sink` prints to stdout; file reports may need a different sink). If emitting file reports from a Rust-test invocation is not supported by gaveldrop's public API, drop the artifact step and note it as a finding (armadai loses the uploaded HTML diff artifact the old harness produced).

- [ ] **Step 4: Verify the local gate in all modes**

Run each: `cargo fmt --all -- --check`; the 3 clippy modes; `cargo test --no-default-features --features tui`; `cargo test --no-default-features --features tui,storage,e2e-fake`.
Expected: all green.

- [ ] **Step 5: Commit**
```bash
git add .github/workflows/ci.yml
git commit -m "ci: run the gaveldrop e2e suite (e2e-fake) and adjust report artifact"
```

---

### Task B4: Defects / divergence report

**Files:**
- Create: `docs/superpowers/gaveldrop-defects-report.md`

- [ ] **Step 1: Write the report** covering, with concrete evidence (file:line in both repos):
  - **G1** (fixed): the runner had no adapter-injection seam; `run_all_with` (PR 70/71) resolved it.
  - **F2** (fixed): the inversion where an unknown `fake:` key became a catch-all; gaveldrop now refuses it. armadai's divergence from the briefing's top-level `fake:` example: the scenario lives under `setup.scenario:` (opaque), NOT `fake:`.
  - **F3**: the single `run_in_iso` exit shared by the conformance and real branches — the load-bearing condition, and confirmation armadai's adapter satisfies it.
  - **F4**: the `prov_model_non_empty` → `prov_non_empty` + `model_non_empty` split (semantic split, not a rename; the old suite had 4 invariants, the new config has 5).
  - **events semantics**: the comparison of gaveldrop's `check_subsequence`/`check_counts` vs the old `check_events_order_and_fields`/`check_event_counts` — confirmed identical, or any difference found while migrating.
  - **Scope drops**: `expect.storage` (armadai had a SQLite row-count assertion capability; no case exercised it and gaveldrop does not offer it — deliberate drop, not a gap). The uploaded HTML report artifact, if dropped in B3.
  - **Line count**: old harness (~1655) vs new armadai-side code (`armadai-fake/src/lib.rs` + `tests/gaveldrop.rs` + `gaveldrop.yaml` + 9 cases) — report the total and what gaveldrop now owns that armadai used to.
  - **Anything reimplemented** rather than delegated, and why.

- [ ] **Step 2: Commit**
```bash
git add docs/superpowers/gaveldrop-defects-report.md
git commit -m "docs(test): gaveldrop migration defects + divergence report"
```

---

## Self-Review

**Spec coverage** (against the briefing's 6 deliverables): #1 adapter+conformance = A2/A5; #2 fake-claude on gaveldrop-fake = A1; #3 gaveldrop.yaml = A3; #4 9 cases identical verdicts = A4/B1; #5 delete 1655 lines = B2; #6 defects report = B4. CI = B3. ✅

**Placeholder scan:** the `invoke` body (A2 Step 6) and `run_in_iso` env-extension are described precisely but the final `AdapterError` construction + the `run_in_iso_with_env` merge are marked "verify at gaveldrop / adjust signature" — these are genuine API-shape confirmations the implementer must do against the pinned gaveldrop, not hand-waved logic. Every case transform (A4) has a full worked example + a mechanical rule. No "TODO"/"handle edge cases".

**Type consistency:** `run_in_iso` gains an extra-env parameter in A2 Step 6 (fold that signature into Step 2 when implementing — one helper, the F3 condition). `select_response` returns `Option` (A1) and every caller handles `None`. `Observations.exit` (not `exit_code`). Config invariants count = 5 (A3, checked in A3 Step 2 and implicitly by B1).

**Known verification points the implementer MUST resolve at the pinned gaveldrop (`9ed05ec`), not guess:** `AdapterError` construction; `Isolation` accessor names (`env`/`cleared`/`journal_path`/`changes`/`root`); `Case`/`Setup`/`Expect` public constructibility (else build via YAML); `Terminal` module path; `Config::load` return type; each invariant shape's exact parameter keys. rust-analyzer is unreliable in this repo — verify at the compiler.
