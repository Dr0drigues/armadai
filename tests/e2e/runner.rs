//! Declarative evaluation of a case's `expect` block against the captured
//! stdout/exit code of a real `armadai run … --json` invocation.
//!
//! [`evaluate`] is pure (no filesystem/process access): it takes the raw stdout text
//! and the process exit code and checks them against [`crate::e2e::case::Expect`].
//! The one exception is `expect.storage`, which needs the tempdir root to open the
//! isolated SQLite DB — that check lives in [`check_storage`] and is invoked
//! separately by `harness::run_case` (which owns the tempdir), then merged into the
//! [`CaseOutcome`] returned by `evaluate`.

use std::collections::BTreeMap;
#[cfg(feature = "storage")]
use std::path::Path;

use serde_json::Value;

use super::case::{CaseFile, ExpectedEvent};

/// Outcome of running one case: pass/fail plus enough context to render a report.
///
/// `name`/`weight`/`allow_fail`/`expected`/`observed` are read back out by
/// `report::write_reports` (the report aggregator) to build both the JSON summary
/// and the HTML expected-vs-observed blocks; `passed`/`diffs` are also exercised by
/// this module's own tests.
#[derive(Debug, Clone, Default)]
pub struct CaseOutcome {
    pub name: String,
    pub weight: u32,
    pub passed: bool,
    pub allow_fail: bool,
    /// Debug rendering of `expect`, for report output.
    pub expected: String,
    /// Raw stdout captured from the `armadai` invocation.
    pub observed: String,
    /// Human-readable reasons the case failed (empty when `passed`).
    pub diffs: Vec<String>,
}

/// Parse `stdout` as JSONL (ignoring non-JSON / plain-text log lines — anything that
/// doesn't parse as a JSON object with a `t` field is skipped), then check `exit` and
/// every part of `case.expect` against it.
///
/// Does **not** check `expect.storage` (see module doc) — callers that care about it
/// should also call [`check_storage`] and fold its diffs in.
pub fn evaluate(case: &CaseFile, stdout: &str, exit: i32) -> CaseOutcome {
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(|v| v.get("t").is_some())
        .collect();

    let mut diffs = Vec::new();

    if exit != case.expect.exit_code {
        diffs.push(format!(
            "exit_code: expected {}, got {exit}",
            case.expect.exit_code
        ));
    }

    check_events_order_and_fields(&case.expect.events, &events, &mut diffs);
    check_event_counts(&case.expect.event_counts, &events, &mut diffs);
    check_invariants(&case.expect.invariants, &events, &mut diffs);

    CaseOutcome {
        name: case.name.clone(),
        weight: case.weight,
        allow_fail: case.allow_fail,
        passed: diffs.is_empty(),
        expected: format!("{:#?}", case.expect),
        observed: stdout.to_string(),
        diffs,
    }
}

/// Each [`ExpectedEvent`] must appear in `observed`, in the same relative order as
/// `expected` (a subsequence match — unrelated events may appear in between, and
/// events not listed in `expected` at all are ignored here; see `check_event_counts`
/// for exhaustive counting). A matched event's index becomes the new search floor so
/// two identical `expected` entries require two distinct `observed` occurrences.
fn check_events_order_and_fields(
    expected: &[ExpectedEvent],
    observed: &[Value],
    diffs: &mut Vec<String>,
) {
    let mut cursor = 0usize;
    for exp in expected {
        let found = observed
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, obs)| event_matches(exp, obs));
        match found {
            Some((i, _)) => cursor = i + 1,
            None => diffs.push(format!(
                "expected event t={:?} fields={:?} not found (in order) at/after position {cursor} \
                 in observed stream ({} events)",
                exp.t,
                exp.fields,
                observed.len()
            )),
        }
    }
}

/// Whether `obs` matches `exp`: same `t`, and every field named in `exp.fields` is
/// present in `obs` with the exact same value (fields absent from `exp` are not
/// checked — this is a subset match, not an exact-equality match).
fn event_matches(exp: &ExpectedEvent, obs: &Value) -> bool {
    let Some(t) = obs.get("t").and_then(Value::as_str) else {
        return false;
    };
    if t != exp.t {
        return false;
    }
    exp.fields
        .iter()
        .all(|(k, v)| obs.get(k.as_str()) == Some(v))
}

/// Exact per-type counts across the whole observed stream (unlike
/// `check_events_order_and_fields`, which only checks presence/order of the events
/// named in `expect.events`).
fn check_event_counts(
    expected: &BTreeMap<String, usize>,
    observed: &[Value],
    diffs: &mut Vec<String>,
) {
    for (t, want) in expected {
        let got = count_of(observed, t);
        if got != *want {
            diffs.push(format!("event_counts[{t}]: expected {want}, got {got}"));
        }
    }
}

fn count_of(observed: &[Value], t: &str) -> usize {
    observed
        .iter()
        .filter(|v| v.get("t").and_then(Value::as_str) == Some(t))
        .count()
}

/// Run each named invariant against the observed stream, collecting failures.
fn check_invariants(names: &[String], observed: &[Value], diffs: &mut Vec<String>) {
    for name in names {
        let result = match name.as_str() {
            "agent_start_end_symmetric" => agent_start_end_symmetric(observed),
            "prov_model_non_empty" => prov_model_non_empty(observed),
            "single_result" => single_result(observed),
            "no_orphan_events" => no_orphan_events(observed),
            other => Err(format!("unknown invariant '{other}'")),
        };
        if let Err(msg) = result {
            diffs.push(format!("invariant '{name}' failed: {msg}"));
        }
    }
}

/// `agent_start` and `agent_end` counts must match — every agent that started must
/// also have ended (success or failure both emit `agent_end` upstream; a mismatch
/// here means a run was aborted mid-agent, e.g. a panic or a timeout).
fn agent_start_end_symmetric(observed: &[Value]) -> Result<(), String> {
    let starts = count_of(observed, "agent_start");
    let ends = count_of(observed, "agent_end");
    if starts == ends {
        Ok(())
    } else {
        Err(format!("agent_start={starts} != agent_end={ends}"))
    }
}

/// Every `agent_start` event must carry non-empty `prov` and `model` — these come
/// straight from the agent's `## Metadata` (see `src/cli/run.rs`), so an empty value
/// means the harness generated (or the CLI resolved) a malformed agent definition.
fn prov_model_non_empty(observed: &[Value]) -> Result<(), String> {
    for ev in observed
        .iter()
        .filter(|v| v.get("t").and_then(Value::as_str) == Some("agent_start"))
    {
        let prov = ev.get("prov").and_then(Value::as_str).unwrap_or("");
        let model = ev.get("model").and_then(Value::as_str).unwrap_or("");
        if prov.is_empty() || model.is_empty() {
            return Err(format!("agent_start with empty prov/model: {ev}"));
        }
    }
    Ok(())
}

/// Exactly one `result` event — `armadai run` emits one final aggregate result per
/// invocation regardless of how many agents/rounds ran.
fn single_result(observed: &[Value]) -> Result<(), String> {
    let n = count_of(observed, "result");
    if n == 1 {
        Ok(())
    } else {
        Err(format!("expected exactly 1 result event, got {n}"))
    }
}

/// No `agent_end` for an agent that never had a matching `agent_start`, and no agent
/// left "open" (started but never ended) by the end of the stream. This is a
/// per-agent balance check, not a strict global interleaving check (a well-behaved
/// run never interleaves two `agent_start`s for the same agent without an `agent_end`
/// in between, so balance is sufficient here).
fn no_orphan_events(observed: &[Value]) -> Result<(), String> {
    use std::collections::HashMap;
    let mut open: HashMap<&str, i32> = HashMap::new();
    for ev in observed {
        match ev.get("t").and_then(Value::as_str) {
            Some("agent_start") => {
                if let Some(a) = ev.get("agent").and_then(Value::as_str) {
                    *open.entry(a).or_insert(0) += 1;
                }
            }
            Some("agent_end") => {
                if let Some(a) = ev.get("agent").and_then(Value::as_str) {
                    let c = open.entry(a).or_insert(0);
                    *c -= 1;
                    if *c < 0 {
                        return Err(format!("agent_end for '{a}' with no matching agent_start"));
                    }
                }
            }
            _ => {}
        }
    }
    if let Some((a, _)) = open.iter().find(|&(_, &c)| c != 0) {
        return Err(format!(
            "agent '{a}' has an unmatched agent_start (no agent_end)"
        ));
    }
    Ok(())
}

/// Best-effort check of `expect.storage`: exact row counts for the tables listed
/// (currently `runs`, `orchestration_runs`, `board_entries`, `ring_contributions`,
/// `ring_votes` — the tables `src/storage/schema.rs` creates). Opens the SQLite DB at
/// `<project_root>/.local/share/armadai/armadai.sqlite` — the path `armadai` itself
/// resolves to given the harness's `XDG_DATA_HOME=<project_root>/.local/share`
/// override (see `harness::run_case`).
///
/// Returns an empty `Vec` (no-op) when `expect.storage` is `None`. When the `storage`
/// feature is off, also returns empty: this repo's e2e cases are meant to run in CI's
/// `--features tui,storage` mode where the check is meaningful, so treating "feature
/// off" as "not applicable" here (rather than a failure) avoids breaking cases in the
/// `tui`-only / `tui,providers-api` clippy/build modes that never execute this check
/// anyway. TODO: if a case ever needs to assert storage counts under a build that
/// doesn't have `storage`, that's a signal the case belongs in a storage-gated test
/// file instead.
#[cfg(feature = "storage")]
pub fn check_storage(expect: &Option<BTreeMap<String, usize>>, project_root: &Path) -> Vec<String> {
    const KNOWN_TABLES: &[&str] = &[
        "runs",
        "orchestration_runs",
        "board_entries",
        "ring_contributions",
        "ring_votes",
    ];

    let Some(expect) = expect else {
        return Vec::new();
    };

    let mut diffs = Vec::new();
    let db_path = project_root.join(".local/share/armadai/armadai.sqlite");
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            diffs.push(format!(
                "storage: cannot open db at {}: {e}",
                db_path.display()
            ));
            return diffs;
        }
    };

    for (table, want) in expect {
        if !KNOWN_TABLES.contains(&table.as_str()) {
            diffs.push(format!(
                "storage: '{table}' is not a table src/storage/schema.rs creates \
                 (known: {KNOWN_TABLES:?})"
            ));
            continue;
        }
        let got: usize = match conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(n) => n as usize,
            Err(e) => {
                diffs.push(format!("storage: query on '{table}' failed: {e}"));
                continue;
            }
        };
        if got != *want {
            diffs.push(format!("storage[{table}]: expected {want} rows, got {got}"));
        }
    }

    diffs
}

#[cfg(not(feature = "storage"))]
pub fn check_storage(
    _expect: &Option<BTreeMap<String, usize>>,
    _project_root: &std::path::Path,
) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::case::load_case_str;

    fn case_with_expect(expect_yaml: &str) -> CaseFile {
        let yaml = format!(
            "name: t\nweight: 1\nsetup: {{ pattern: direct, agents: [a], flags: [], input: x }}\n\
             fake: {{ rules: [ {{ match: {{}}, respond: ok }} ] }}\n{expect_yaml}"
        );
        load_case_str(&yaml).unwrap()
    }

    #[test]
    fn passes_when_events_and_exit_match() {
        let case = case_with_expect(
            "expect:\n  exit_code: 0\n  events: [ { t: run_start }, { t: result } ]\n",
        );
        let stdout = "{\"t\":\"run_start\",\"v\":1}\n{\"t\":\"agent_start\",\"agent\":\"a\"}\n\
             {\"t\":\"result\",\"content\":\"ok\"}\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(outcome.passed, "diffs: {:?}", outcome.diffs);
    }

    #[test]
    fn fails_on_exit_code_mismatch() {
        let case = case_with_expect("expect:\n  exit_code: 0\n");
        let outcome = evaluate(&case, "", 1);
        assert!(!outcome.passed);
        assert!(outcome.diffs.iter().any(|d| d.contains("exit_code")));
    }

    #[test]
    fn fails_when_expected_event_missing() {
        let case = case_with_expect("expect:\n  exit_code: 0\n  events: [ { t: result } ]\n");
        let outcome = evaluate(&case, "{\"t\":\"run_start\"}\n", 0);
        assert!(!outcome.passed);
        assert!(outcome.diffs.iter().any(|d| d.contains("t=\"result\"")));
    }

    #[test]
    fn fails_when_event_out_of_order() {
        let case = case_with_expect(
            "expect:\n  exit_code: 0\n  events: [ { t: result }, { t: run_start } ]\n",
        );
        let stdout = "{\"t\":\"run_start\"}\n{\"t\":\"result\"}\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(!outcome.passed, "expected out-of-order events to fail");
    }

    #[test]
    fn checks_event_fields_as_subset() {
        let case = case_with_expect(
            "expect:\n  exit_code: 0\n  events: [ { t: agent_start, agent: a } ]\n",
        );
        let stdout =
            "{\"t\":\"agent_start\",\"agent\":\"a\",\"prov\":\"claude\",\"model\":\"m\"}\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(outcome.passed, "diffs: {:?}", outcome.diffs);

        let stdout_wrong = "{\"t\":\"agent_start\",\"agent\":\"b\"}\n";
        let outcome_wrong = evaluate(&case, stdout_wrong, 0);
        assert!(!outcome_wrong.passed);
    }

    #[test]
    fn checks_event_counts_exactly() {
        let case =
            case_with_expect("expect:\n  exit_code: 0\n  event_counts: { agent_start: 2 }\n");
        let stdout = "{\"t\":\"agent_start\",\"agent\":\"a\"}\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(!outcome.passed);
        assert!(
            outcome
                .diffs
                .iter()
                .any(|d| d.contains("event_counts[agent_start]"))
        );
    }

    #[test]
    fn ignores_non_json_lines() {
        let case = case_with_expect("expect:\n  exit_code: 0\n  events: [ { t: result } ]\n");
        let stdout = "not json\n{\"t\":\"result\"}\ntrailing garbage\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(outcome.passed, "diffs: {:?}", outcome.diffs);
    }

    #[test]
    fn invariant_agent_start_end_symmetric_detects_mismatch() {
        let case = case_with_expect(
            "expect:\n  exit_code: 0\n  invariants: [agent_start_end_symmetric]\n",
        );
        let stdout = "{\"t\":\"agent_start\",\"agent\":\"a\"}\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(!outcome.passed);
    }

    #[test]
    fn invariant_prov_model_non_empty_detects_empty_model() {
        let case =
            case_with_expect("expect:\n  exit_code: 0\n  invariants: [prov_model_non_empty]\n");
        let stdout = "{\"t\":\"agent_start\",\"agent\":\"a\",\"prov\":\"claude\",\"model\":\"\"}\n";
        let outcome = evaluate(&case, stdout, 0);
        assert!(!outcome.passed);
    }

    #[test]
    fn invariant_single_result_detects_zero_and_multiple() {
        let case = case_with_expect("expect:\n  exit_code: 0\n  invariants: [single_result]\n");
        assert!(!evaluate(&case, "", 0).passed);
        assert!(!evaluate(&case, "{\"t\":\"result\"}\n{\"t\":\"result\"}\n", 0).passed);
        assert!(evaluate(&case, "{\"t\":\"result\"}\n", 0).passed);
    }

    #[test]
    fn invariant_no_orphan_events_detects_unbalanced_agent() {
        let case = case_with_expect("expect:\n  exit_code: 0\n  invariants: [no_orphan_events]\n");
        // agent_end with no agent_start
        let outcome = evaluate(&case, "{\"t\":\"agent_end\",\"agent\":\"a\"}\n", 0);
        assert!(!outcome.passed);
        // agent_start with no agent_end
        let outcome2 = evaluate(&case, "{\"t\":\"agent_start\",\"agent\":\"a\"}\n", 0);
        assert!(!outcome2.passed);
        // balanced
        let outcome3 = evaluate(
            &case,
            "{\"t\":\"agent_start\",\"agent\":\"a\"}\n{\"t\":\"agent_end\",\"agent\":\"a\"}\n",
            0,
        );
        assert!(outcome3.passed);
    }

    #[test]
    fn unknown_invariant_fails_with_message() {
        let case = case_with_expect("expect:\n  exit_code: 0\n  invariants: [nope]\n");
        let outcome = evaluate(&case, "", 0);
        assert!(!outcome.passed);
        assert!(
            outcome
                .diffs
                .iter()
                .any(|d| d.contains("unknown invariant"))
        );
    }
}
