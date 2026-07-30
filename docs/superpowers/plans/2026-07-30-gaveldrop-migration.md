# Gaveldrop Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace armadai's ~1655-line hand-rolled e2e harness with a thin `gaveldrop::Adapter` + a `gaveldrop.yaml` + 9 migrated cases, delegating discovery/isolation/evaluation/reporting to gaveldrop, while armadai becomes gaveldrop's first real consumer.

**Architecture:** gaveldrop (external YAML test engine at `~/work/misc/gaveldrop`, path dep pinned at `9ed05ec`) owns isolation, event extraction (`EventsConfig.type_field: t`), subsequence/count matching, named invariants, and reporting. armadai supplies: (1) an `Armadai` adapter that turns a case's `setup` into an `armadai run …` invocation inside gaveldrop's `Isolation`; (2) `fake-claude` rebuilt on the `gaveldrop-fake` library (Counter + Journal + env), keeping its Claude Code `stream-json` byte shape; (3) config + cases.

**Sequencing — why it is NOT a clean "additive then flip":** `fake-claude` is a single shared binary. The old harness invokes it with the env contract `FAKE_SCENARIO`/`FAKE_STATE_DIR`; the new engine reads `ARMADAI_FAKE_SCENARIO` + `GAVELDROP_STATE`/`GAVELDROP_JOURNAL`. The moment the `crates/armadai` `fake-claude` bin is rewritten, the old e2e suite breaks. So the bin rewrite and the old-suite deletion are **one atomic task (T3)**: the gate is green before it (old bin + old suite) and green after it (new bin, old suite gone). Everything that does NOT touch the shared bin (the `armadai-fake` crate, the config, the cases) is prepared in safe tasks before T3; everything that exercises the new bin (the adapter/conformance/suite tests) is added after T3. **Every task ends with a green local gate.**

**Tech Stack:** Rust edition 2024; `gaveldrop`, `gaveldrop-fake`, `gaveldrop-conformance` (path deps); `serde`/`serde_yaml_ng`/`serde_json`; `assert_cmd`.

## Global Constraints

- **NEVER modify `~/work/misc/gaveldrop`.** Depend on it by **path**, pinned to revision `9ed05ec`. Any needed gaveldrop change is a written finding, not an edit.
- Branch: `feat/gaveldrop-migration`. **Do NOT merge to `master`** until gaveldrop ships a definitive/released version (then path dep → version dep). The whole plan lives on this branch.
- French for all user communication; **English** for code, comments, commit messages.
- Commit trailer: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Local gate (all must pass, at every task): `cargo fmt --all -- --check`; clippy in 3 modes (`--no-default-features --features tui`; `…tui,providers-api`; `…tui,web,storage`) with `-D warnings`; `cargo test --no-default-features --features tui` and `…tui,storage`. From T3 on, ALSO `cargo test --no-default-features --features tui,storage,e2e-fake`.
- **`fake-claude` must emit exactly two `stream-json` lines, byte-identical to today**: an `assistant` line `{"type":"assistant","message":{"content":[{"type":"text","text":<respond>}]}}` and a `result` line `{"type":"result","subtype":"success","is_error":false,"result":<respond>,"total_cost_usd":<cost|0.0>,"usage":{"input_tokens":<tokens_in|0>,"output_tokens":<tokens_out|0>},"modelUsage":{<model|"fake-model">:{"outputTokens":<tokens_out|0>}}}`. `respond` appears twice. Defaults: `model`→`"fake-model"`, tokens→0, cost→0.0.
- The released `armadai` binary crate must NOT pull `gaveldrop-fake`/`gaveldrop` into a default `cargo build --release`. External gaveldrop deps enter only under the `e2e-fake` feature (bin) and as dev-deps (test).
- **All 9 migrated cases must PASS** (none is `allow_fail`) with verdicts identical to the current harness.
- Keep `tests/e2e/hook_stdout.rs`'s two tests (armadai's `__claude-register-session` stdout contract) — they are NOT part of the e2e suite and must survive the deletion (rehomed to `tests/hook_stdout.rs` in T3).

---

## File Structure

**New:**
- `crates/armadai-fake/` — new lib crate. `src/lib.rs` = shared fake scenario types (`Scenario`/`Rule`/`Match`) + `select_response` + `emit_claude_jsonl` + `run()` (the engine, on `gaveldrop_fake::{Counter, Journal, env}`). Isolates the external `gaveldrop-fake` dep from the shipped `armadai`/`armadai-core` crates.
- `crates/armadai/tests/gaveldrop.rs` — the new integration test target: the `Armadai` adapter, the shared `run_in_iso` helper, `config_loads`/`all_cases_load`, the conformance test, the suite-run test. Feature-gated `#![cfg(feature = "e2e-fake")]`.
- `crates/armadai/tests/cases/*.yaml` — the 9 migrated cases (gaveldrop format).
- `gaveldrop.yaml` — repo-root gaveldrop config.
- `docs/superpowers/gaveldrop-defects-report.md` — the deliverable defects/divergence report.

**Modified:**
- `crates/armadai/Cargo.toml` — `e2e-fake` feature, optional `armadai-fake` dep, `fake-claude` bin `required-features`, gaveldrop dev-deps, `[[test]] name="gaveldrop"`.
- `crates/armadai/src/bin/fake-claude.rs` — becomes a thin `fn main() { armadai_fake::run() }` (T3).
- `Cargo.toml` (workspace root) — add `crates/armadai-fake` to `members`.
- `.github/workflows/ci.yml` — enable `e2e-fake` in the e2e test job; swap the report artifact.

**Deleted (T3):** `crates/armadai/tests/e2e.rs`, `tests/e2e/{mod,harness,runner,case,report}.rs`, `tests/e2e/cases/*.yaml`. **Kept:** `tests/e2e/hook_stdout.rs` → `tests/hook_stdout.rs`.

---

### Task T1: `armadai-fake` crate (library + engine, no bin rewrite yet)

**Files:**
- Create: `crates/armadai-fake/Cargo.toml`, `crates/armadai-fake/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces:**
- Produces (consumed by T4/T5/T6 and, at T3, by the `fake-claude` shim): `armadai_fake::{Scenario, Rule, Match, select_response, emit_claude_jsonl, run, SCENARIO_ENV}`.
  - `pub struct Scenario { pub rules: Vec<Rule> }` — `Serialize + Deserialize + Clone`.
  - `pub struct Rule { #[serde(rename="match", default)] pub match_: Match, pub respond: String, pub tokens_in: Option<u32>, pub tokens_out: Option<u32>, pub cost: Option<f64>, pub model: Option<String>, pub latency_ms: Option<u64>, pub exit_code: Option<i32> }` — metric fields `#[serde(default)]`.
  - `pub struct Match { pub agent: Option<String>, pub call: Option<u32>, pub prompt_contains: Option<String> }` — `Default`, all `#[serde(default)]`.
  - `pub fn select_response<'s>(scenario: &'s Scenario, agent: &str, call: u32, prompt: &str) -> Option<&'s Rule>`.
  - `pub fn emit_claude_jsonl(rule: &Rule) -> String` — the two `stream-json` lines (see Global Constraints for exact bytes).
  - `pub fn run()` — binary entry point.
  - `pub const SCENARIO_ENV: &str = "ARMADAI_FAKE_SCENARIO";`

**Why a dedicated scenario env, not `GAVELDROP_SCENARIO`:** gaveldrop's `Isolation` always writes a fallback catch-all scenario in *its* vocabulary and points `GAVELDROP_SCENARIO` at it (even when `case.fake` is `None`). armadai's scenario lives under `setup:` (opaque to gaveldrop), so the adapter (T4) writes it and points `fake-claude` at it via `ARMADAI_FAKE_SCENARIO`. State/journal DO come from gaveldrop: `Counter::from_env()` reads `GAVELDROP_STATE`, `Journal::from_env()` reads `GAVELDROP_JOURNAL` — both set by `Isolation` and inherited by the `claude` subprocess.

- [ ] **Step 1: Workspace member + manifest**

Add `"crates/armadai-fake"` to `members` in the root `Cargo.toml`.

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
Verify the relative path resolves: from `crates/armadai-fake/`, `ls ../../../gaveldrop/crates/gaveldrop-fake/Cargo.toml` (armadai at `~/work/misc/armadai`, gaveldrop at `~/work/misc/gaveldrop`). Copy the exact `serde_yaml_ng`/`serde_json` versions from `crates/armadai/Cargo.toml`.

- [ ] **Step 2: `src/lib.rs` — types + select + emit (ported verbatim from the current bin)**

Port `Scenario`/`Rule`/`Match`, `agent_id_from_prompt`, `emit_claude_jsonl` from `crates/armadai/src/bin/fake-claude.rs` (lines 18-154, 61-74) EXACTLY (this is the on-wire scenario format). Change `select_response` to return `Option<&Rule>` (no panic). Add `run()`:
```rust
use gaveldrop_fake::{Counter, Journal, Invocation, Call};

pub const SCENARIO_ENV: &str = "ARMADAI_FAKE_SCENARIO";

/// Binary entry point: journal the call, select a rule, emit stream-json, exit.
pub fn run() {
    let inv = Invocation::from_env(false);
    let prompt = std::env::args().next_back().unwrap_or_default();
    let agent = agent_id_from_prompt(&prompt).unwrap_or_else(|| "unknown".to_string());

    let counter = Counter::from_env().expect("GAVELDROP_STATE set by isolation");
    let call = counter.next(&agent).unwrap_or(1);

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
            let exit = rule.exit_code.unwrap_or(0);
            if let Some(j) = &journal {
                let _ = j.record(&Call::from_invocation(&inv, call, &agent, false, false, exit));
            }
            if let Some(ms) = rule.latency_ms {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            println!("{}", emit_claude_jsonl(rule));
            std::process::exit(exit);
        }
        None => {
            // No scenario (conformance probe) or no matching rule: journal a catch-all
            // and exit non-zero WITHOUT stream-json. This is what makes fake-claude usable
            // as the conformance kit's fake, and turns a missing catch-all rule into an
            // observable failed call rather than a panic.
            if let Some(j) = &journal {
                let _ = j.record(&Call::from_invocation(&inv, call, &agent, true, false, 127));
            }
            std::process::exit(127);
        }
    }
}
```
Verify `Call::from_invocation` (`~/work/misc/gaveldrop/crates/gaveldrop-fake/src/journal.rs:36`: `inv, call, key, catch_all, passthrough, exit`) and `Invocation::from_env(read_stdin: bool)` (`invocation.rs:32`).

- [ ] **Step 3: Port the unit tests into `lib.rs`**

Move `#[cfg(test)] mod tests` from the current `fake-claude.rs` (lines 201-359). Adjust: `select_response` now returns `Option` (`.unwrap()`/`.is_none()`). Keep `emits_parseable_claude_jsonl`, `emits_defaults_when_metrics_absent`, `extracts_agent_id_from_prompt`, `extracts_agent_id_ignores_leading_whitespace`, `selects_by_agent_and_call`, `selects_by_prompt_contains`. Rewrite the counter test against `gaveldrop_fake::Counter::new(dir.path()).next("t-a")`.

- [ ] **Step 4: Run + commit**

Run: `cargo test -p armadai-fake` — Expected: PASS. Run `cargo fmt --all` + the 3 clippy modes + `cargo test --no-default-features --features tui,storage` (old e2e still uses the OLD bin — still green).
```bash
git add crates/armadai-fake Cargo.toml
git commit -m "test(e2e): armadai-fake crate — scenario engine on gaveldrop-fake"
```

---

### Task T2: `gaveldrop.yaml` + the 9 migrated cases (files only)

**Files:**
- Create: `gaveldrop.yaml` (repo root)
- Create: `crates/armadai/tests/cases/{direct,blackboard,budget-halt-visible,hierarchical,nested,no-tui,quiet-orchestrated,ring,ring-budget-reaches-vote}.yaml`

These are inert files until T4/T6 run them; T2's gate is unaffected (old e2e untouched).

- [ ] **Step 1: Write `gaveldrop.yaml`**
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Dr0drigues/gaveldrop/main/docs/case.schema.json
cases: crates/armadai/tests/cases/**/*.yaml

fake:
  bins: [claude]

clear_env: [ARMADAI_CONFIG_DIR]

events:
  type_field: t

invariants:
  agent_start_end_symmetric: { shape: paired, start: agent_start, end: agent_end, key: agent }
  single_result:             { shape: exactly_one, type: result }
  # F4 split: the old single `prov_model_non_empty` becomes TWO field_non_empty
  # invariants — field_non_empty keeps exactly one field, a better diagnostic.
  prov_non_empty:            { shape: field_non_empty, type: agent_start, field: prov }
  model_non_empty:           { shape: field_non_empty, type: agent_start, field: model }
  no_orphan_events:          { shape: no_orphan, key: agent, root: agent_start }
```
Verify each shape's exact parameter names against `~/work/misc/gaveldrop/crates/gaveldrop/src/verdict/invariants.rs` and the config test `config.rs:437-451`. No `gate:` block (a failing case already fails the run; no threshold beyond "all pass" is wanted).

- [ ] **Step 2: Migrate the 9 cases**

The transform, per case (source = `crates/armadai/tests/e2e/cases/<name>.yaml`):
1. Move the whole `fake:` block UNDER `setup:` as `setup.scenario:` (gaveldrop's top-level `fake:` reads only its four criteria and REFUSES unknown keys like `respond`/`agent`; armadai's scenario is opaque data under `setup:`).
2. In `expect.invariants`, replace `prov_model_non_empty` with `prov_non_empty, model_non_empty`.
3. Everything else unchanged: `name`, `weight`, `setup.{pattern,agents,flags,input,nested_team,token_budget}`, `expect.{exit_code,events,event_counts}` (gaveldrop's `check_subsequence`/`check_counts` have identical semantics to the old harness).

Worked example — `direct.yaml`:
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
Read each real source file and reproduce its exact `setup`/`expect` values. Keep the rich comments in `nested.yaml`/`budget-halt-visible.yaml` (they document real engine behavior). `nested_team:` stays under `setup:` verbatim.

- [ ] **Step 3: Commit** (no test step — nothing runs these yet; gate unchanged from T1)
```bash
git add gaveldrop.yaml crates/armadai/tests/cases
git commit -m "test(e2e): gaveldrop.yaml + 9 cases migrated to gaveldrop format"
```

---

### Task T3: Atomic flip — rewrite the bin, wire the feature, delete the old harness

**This is the one task where the gate transitions from "old bin + old suite" to "new bin, old suite gone". It MUST be a single commit: the gate is green immediately before (old suite passes) and immediately after (old suite deleted, new bin builds but is unused until T4).**

**Files:**
- Modify: `crates/armadai/Cargo.toml`
- Rewrite: `crates/armadai/src/bin/fake-claude.rs`
- Create: `crates/armadai/tests/hook_stdout.rs` (rehomed)
- Delete: `crates/armadai/tests/e2e.rs`, `tests/e2e/{mod,harness,runner,case,report}.rs`, `tests/e2e/cases/*.yaml`, `tests/e2e/hook_stdout.rs`

**Interfaces:**
- Consumes: `armadai_fake::run` (T1).

- [ ] **Step 1: Cargo wiring**

`crates/armadai/Cargo.toml`:
- `[features]`: add `e2e-fake = ["dep:armadai-fake"]`.
- `[dependencies]`: add `armadai-fake = { path = "../armadai-fake", optional = true }`.
- The `fake-claude` `[[bin]]` block (lines 15-16): add `required-features = ["e2e-fake"]`.
- Add `[dev-dependencies]`: `gaveldrop = { path = "../../../gaveldrop/crates/gaveldrop" }`, `gaveldrop-fake = { path = "../../../gaveldrop/crates/gaveldrop-fake" }`, `gaveldrop-conformance = { path = "../../../gaveldrop/crates/gaveldrop-conformance" }`, `armadai-fake = { path = "../armadai-fake" }`.
- Add `[[test]]` block: `name = "gaveldrop"`, `required-features = ["e2e-fake"]`.
- If `schemars` in `[dev-dependencies]` was used ONLY by the old `case.rs`, remove it (verify `grep -rn schemars crates/armadai` shows only the to-be-deleted files).

- [ ] **Step 2: Rewrite the bin as a shim**
```rust
//! `fake-claude` — deterministic stand-in for the `claude` CLI, used by the gaveldrop
//! e2e suite. The engine lives in the `armadai-fake` crate (built on `gaveldrop-fake`);
//! this binary is only its entry point. Built only under the `e2e-fake` feature so a
//! default release build never pulls the external gaveldrop deps.
fn main() {
    armadai_fake::run();
}
```

- [ ] **Step 3: Rehome `hook_stdout.rs`**

Read `tests/e2e/hook_stdout.rs` (a standalone test of armadai's `__claude-register-session` stdout contract — independent of the harness). Move it to `tests/hook_stdout.rs` (its own top-level integration test target). Fix any `use super::…` / module-path references it had from living under the `e2e` module tree. Confirm it compiles as its own target.

- [ ] **Step 4: Delete the old harness**
```bash
git rm crates/armadai/tests/e2e.rs
git rm -r crates/armadai/tests/e2e
```
(Do Step 3's copy-out + `git add tests/hook_stdout.rs` first so the rehomed file survives.) Also delete `docs/e2e-case.schema.json` if the old schema-emitting test was its only producer (grep first).

- [ ] **Step 5: Verify the gate in every mode**

Run:
- `cargo fmt --all -- --check`
- the 3 clippy modes (`tui`; `tui,providers-api`; `tui,web,storage`)
- `cargo test --no-default-features --features tui` — PASS (`hook_stdout` runs; no `e2e`/`gaveldrop` target)
- `cargo test --no-default-features --features tui,storage` — PASS (old `e2e` gone, `hook_stdout` runs)
- `cargo build --no-default-features --features tui,storage,e2e-fake --bin fake-claude` — builds the new bin
- `cargo build --release --no-default-features --features tui,storage` then `cargo tree --no-default-features --features tui,storage -i gaveldrop-fake` — expect "not found in dependency graph" (release stays clean)

- [ ] **Step 6: Commit (the atomic flip)**
```bash
git add -A
git commit -m "test(e2e): flip fake-claude onto armadai-fake; delete the old harness (~1655 lines)"
```

---

### Task T4: `Armadai` adapter + shared `run_in_iso` + config/case load tests

**Files:**
- Create: `crates/armadai/tests/gaveldrop.rs`

**Interfaces:**
- Consumes: `armadai_fake::{Scenario, SCENARIO_ENV}`; gaveldrop's `Adapter`/`Case`/`Isolation`/`Observations`/`AdapterError`.
- Produces (consumed by T5/T6): `struct Armadai;` impl `Adapter`; `fn run_in_iso(iso: &Isolation, argv: &[String], extra_env: &[(&str, std::path::PathBuf)]) -> Result<Observations, AdapterError>`.

**Exact gaveldrop shapes (verify at the pinned checkout before writing):**
- `Adapter` (`adapters.rs:18`): `fn claims(&self, case: &Case) -> bool;`, `fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError>;`
- `Case.setup.extra: BTreeMap<String, serde_json::Value>` (`#[serde(flatten)]`, `case.rs:85`); `Case.setup.run: Option<Vec<String>>`.
- `Isolation`: `root()`, `env() -> Vec<(String, OsString)>`, `cleared() -> &[String]`, `journal_path() -> PathBuf`, `changes() -> Vec<FileEffect>`.
- `Observations` (`observations.rs:14`): field is **`exit: i32`** (not `exit_code`); `stdout`, `stderr`, `calls: Vec<Call>`, `events` (leave EMPTY — the runner fills it), `files`; construct with `..Default::default()`.

- [ ] **Step 1: File header + `run_in_iso` (the single exit both branches share — F3)**
```rust
#![cfg(feature = "e2e-fake")]

use std::path::{Path, PathBuf};
use std::process::Command;
use gaveldrop::adapters::{Adapter, AdapterError};
use gaveldrop::case::Case;
use gaveldrop::iso::Isolation;
use gaveldrop::observations::Observations;

/// The single exit both branches of `invoke` funnel through. The conformance kit
/// certifies exactly this isolation plumbing, so if the real armadai branch did not also
/// end here the kit would be vacant. Applies the isolation env (+ any adapter-owned extra
/// env), clears what isolation cleared, runs `argv` in the isolated root, reads back
/// stdout/stderr/exit + the journal + file effects.
fn run_in_iso(iso: &Isolation, argv: &[String], extra_env: &[(&str, PathBuf)]) -> Result<Observations, AdapterError> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    for (k, v) in iso.env() { cmd.env(k, v); }
    for k in iso.cleared() { cmd.env_remove(k); }
    for (k, v) in extra_env { cmd.env(k, v); }
    cmd.current_dir(iso.root());

    let output = cmd.output().map_err(|e| /* AdapterError ctor — match Process adapter */ )?;

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
Verify how the built-in `Process` adapter (`adapters.rs`) constructs `AdapterError` from a spawn error and mirror it.

- [ ] **Step 2: `setup.extra` readers + project writer + command builder**

Add `str_field`/`arr_field` helpers (read `serde_json::Value` from `case.setup.extra`). Port `project_yaml` (`tests/e2e/harness.rs:185-245`), `agent_markdown` (harness.rs:254-268) verbatim, reading `pattern`/`agents`/`nested_team`/`token_budget` from `setup.extra`. `write_project(case, iso)` writes `armadai.yaml` + `agents/<a>.md` into `iso.root()`, and the scenario: `serde_json::from_value::<armadai_fake::Scenario>(case.setup.extra["scenario"].clone())` → YAML → `iso.root().join("armadai-scenario.yaml")`. Port `configure_invocation`'s flag logic (harness.rs:113-141) into `build_command(case) -> Vec<String>` starting `[env!("CARGO_BIN_EXE_armadai").into(), "run".into(), coordinator, input]` then pattern flags (⚠️ `--pipe` BEFORE `--orchestrate`; hierarchical = no flags) then `flags`.

- [ ] **Step 3: `impl Adapter for Armadai`**
```rust
struct Armadai;

impl Adapter for Armadai {
    fn claims(&self, case: &Case) -> bool {
        case.setup.extra.contains_key("pattern")
    }

    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        // Conformance-probe branch: a `probe_script` means the kit is checking the
        // isolation contract, not running a fleet. Same run_in_iso exit, no extra env.
        if let Some(script) = str_field(case, "probe_script") {
            return run_in_iso(iso, &["sh".into(), "-c".into(), script.into()], &[]);
        }
        // Real branch: write the project + scenario, point fake-claude at it, build the
        // `armadai run …` argv, run through the SAME helper.
        let scenario_path = write_project(case, iso)?; // returns the armadai-scenario.yaml path
        let argv = build_command(case);
        run_in_iso(iso, &argv, &[(armadai_fake::SCENARIO_ENV, scenario_path)])
    }
}
```
`write_project` returns the scenario path (or `AdapterError` on IO failure — map IO errors the same way as Step 1).

- [ ] **Step 4: `config_loads` + `all_cases_load` tests**
```rust
#[test]
fn config_loads() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let cfg = gaveldrop::config::Config::load(&root.join("gaveldrop.yaml")).expect("parses");
    assert_eq!(cfg.events.as_ref().unwrap().type_field, "t");
    assert_eq!(cfg.invariants.len(), 5);
}

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
Add small adapter unit tests: `claims` true iff `pattern` present; `build_command` puts `--pipe` before `--orchestrate` for ring; hierarchical adds no orchestration flags.

- [ ] **Step 5: Run + commit**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop` — Expected: adapter unit tests + `config_loads` + `all_cases_load` PASS (a case gaveldrop refuses to load = a finding). Run the full gate (all modes).
```bash
git add crates/armadai/tests/gaveldrop.rs
git commit -m "test(e2e): Armadai gaveldrop adapter + run_in_iso + config/case load tests"
```

---

### Task T5: Conformance test

**Files:**
- Modify: `crates/armadai/tests/gaveldrop.rs`

**Interfaces:**
- Consumes: `gaveldrop_conformance::run_with(&dyn Adapter, &Path, &Invocation) -> ConformanceReport` where `Invocation = dyn Fn(&str) -> Case`; `ConformanceReport::{is_conformant, render}`.

- [ ] **Step 1: Invocation factory (adapter claims it, `Process` does not)**
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
Verify `Setup`/`Case`/`Expect` are constructible from the test crate (all fields `pub`); if any is private, build the case by deserializing a YAML string instead. The script goes in `extra["probe_script"]` (NOT `setup.run`) so only `Armadai` claims it (the built-in `Process` needs `run:`).

- [ ] **Step 2: The test**
```rust
#[test]
fn armadai_adapter_is_conformant() {
    let fake = Path::new(env!("CARGO_BIN_EXE_fake-claude"));
    let report = gaveldrop_conformance::run_with(&Armadai, fake, &as_armadai_probe);
    assert!(report.is_conformant(), "\n{}", report.render());
}
```
`fake-claude` is the conformance fake: with no `ARMADAI_FAKE_SCENARIO`, its `run()` journals a catch-all and exits 127 — exactly what the kit's catch-all check needs.

- [ ] **Step 3: Run + commit**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop armadai_adapter_is_conformant` — Expected: PASS (`is_conformant()` true). A failed CHECK that reflects an adapter isolation bug → fix it; a failed check that reflects a KIT gap → finding for the report. Run the full gate.
```bash
git add crates/armadai/tests/gaveldrop.rs
git commit -m "test(e2e): armadai adapter passes the gaveldrop conformance kit"
```

---

### Task T6: Suite-run via `run_all_with` (the real end-to-end gate)

**Files:**
- Modify: `crates/armadai/tests/gaveldrop.rs`

**Interfaces:**
- Consumes: `gaveldrop::runner::run_all_with(&Config, &Path, &Path, &mut dyn Sink, Option<Shard>, Option<&str>, &[Box<dyn Adapter>]) -> Result<Report, ConfigError>`; `gaveldrop::report::terminal::Terminal::plain(W)`; `report.is_success()`; `report.summary().failed`.

- [ ] **Step 1: The suite-run test**
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
`root` = workspace root (`CARGO_MANIFEST_DIR` is `crates/armadai`, so `../..`). Verify `Config::load`'s return type and `Terminal`'s module path.

- [ ] **Step 2: Run — the decisive gate**

Run: `cargo test --no-default-features --features tui,storage,e2e-fake --test gaveldrop e2e_suite_passes_through_gaveldrop -- --nocapture` — Expected: PASS, all 9 cases green, verdicts identical to the old harness. A genuine semantic divergence from the old harness = a finding (record it; do NOT weaken a case to hide it). Run the full gate.

- [ ] **Step 3: Commit**
```bash
git add crates/armadai/tests/gaveldrop.rs
git commit -m "test(e2e): run the 9-case suite through gaveldrop run_all_with"
```

---

### Task T7: CI workflow update

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Read the current test job matrix + the e2e-report artifact upload.**
- [ ] **Step 2:** In the `tui,storage` test mode (where the old e2e lived), change features to `tui,storage,e2e-fake` so the `gaveldrop` target + `fake-claude` bin build and run. Leave the `tui`-only mode as is.
- [ ] **Step 3: Swap the report artifact.** The old `target/e2e-report/e2e-report.{json,html}` is gone. Read `~/work/misc/gaveldrop/docs/ci.md` for where gaveldrop writes reports and whether the Rust-test path (`Terminal::plain` → stdout) can also emit file reports. If yes, point the artifact step there; if the public API can't emit file reports from a test invocation, drop the artifact step and record it as a finding (armadai loses the uploaded HTML diff the old harness produced).
- [ ] **Step 4:** Verify the full local gate in all modes (fmt, 3 clippy, `tui`, `tui,storage`, `tui,storage,e2e-fake`).
- [ ] **Step 5: Commit**
```bash
git add .github/workflows/ci.yml
git commit -m "ci: run the gaveldrop e2e suite (e2e-fake) and adjust the report artifact"
```

---

### Task T8: Defects / divergence report

**Files:**
- Create: `docs/superpowers/gaveldrop-defects-report.md`

- [ ] **Step 1: Write the report** with concrete evidence (file:line in both repos):
  - **G1** (fixed): no adapter-injection seam on the runner; `run_all_with` (PR 70/71) resolved it.
  - **F2** (fixed): the inversion where an unknown `fake:` key became a catch-all; gaveldrop now refuses it. armadai's divergence from the briefing's top-level `fake:` example: the scenario lives under `setup.scenario:` (opaque), NOT `fake:`.
  - **F3**: the single `run_in_iso` exit shared by the conformance and real branches — the load-bearing condition, and confirmation armadai's adapter satisfies it.
  - **F4**: `prov_model_non_empty` → `prov_non_empty` + `model_non_empty` (semantic split, not rename; old suite had 4 invariants, new config has 5).
  - **events semantics**: gaveldrop's `check_subsequence`/`check_counts` vs the old `check_events_order_and_fields`/`check_event_counts` — confirmed identical, or any difference found while migrating.
  - **Scope drops**: `expect.storage` (armadai had a SQLite row-count assertion capability; no case exercised it and gaveldrop does not offer it — deliberate drop, not a gap). The uploaded HTML report artifact, if dropped in T7.
  - **Line count**: old harness (~1655) vs new armadai-side code (`armadai-fake/src/lib.rs` + `tests/gaveldrop.rs` + `gaveldrop.yaml` + 9 cases) — the total and what gaveldrop now owns that armadai used to.
  - **Anything reimplemented** rather than delegated, and why.

- [ ] **Step 2: Commit**
```bash
git add docs/superpowers/gaveldrop-defects-report.md
git commit -m "docs(test): gaveldrop migration defects + divergence report"
```

---

## Self-Review

**Spec coverage** (briefing's 6 deliverables): #1 adapter+conformance = T4/T5; #2 fake-claude on gaveldrop-fake = T1/T3; #3 gaveldrop.yaml = T2; #4 9 cases identical verdicts = T2/T6; #5 delete 1655 lines = T3; #6 defects report = T8. CI = T7. ✅

**Sequencing invariant:** the shared-bin rewrite (T3 Step 2) is committed atomically with the old-suite deletion (T3 Step 4). Before T3: old bin + old suite green. After T3: new bin (unused) + old suite gone, green. T4/T5/T6 add tests that exercise the new bin — each already green because the flip is done. No task leaves the gate red. There is a temporary end-to-end coverage gap between T3 and T6 (only `all_cases_load` runs), closed by T6; acceptable on a non-merging experimental branch.

**Placeholder scan:** `run_in_iso`'s `AdapterError` construction and `write_project`'s error mapping are marked "match the Process adapter / verify at gaveldrop" — genuine API-shape confirmations against the pinned checkout, not hand-waved logic. Each case transform (T2) has a full worked example + a mechanical rule. No "TODO"/"handle edge cases".

**Type consistency:** `run_in_iso(iso, argv, extra_env)` — one signature, defined in T4 Step 1, used by both `invoke` branches (F3) and unchanged in T5/T6. `select_response` returns `Option` (T1); every caller handles `None`. `Observations.exit` (not `exit_code`). Config invariants = 5 (T2, asserted in T4).

**Verification points the implementer MUST resolve at the pinned gaveldrop (`9ed05ec`), never guess (rust-analyzer is unreliable here — verify at the compiler):** `AdapterError` construction; `Isolation` accessor names; `Case`/`Setup`/`Expect` public constructibility (else build via YAML); `Terminal` module path; `Config::load` return type; each invariant shape's exact parameter keys.
