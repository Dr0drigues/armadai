//! Declarative case file format for the e2e harness.
//!
//! A **case file** is one YAML document = one test: it describes how to invoke the
//! real `armadai` binary (`setup`), how the `fake-claude` stub should respond
//! (`fake`), and what the run is expected to produce (`expect`). Case files are meant
//! to be safe to hand-write or to generate with an agent, which is why the format is
//! backed by a [`schemars`] JSON Schema (see [`case_json_schema`] and
//! `docs/e2e-case.schema.json`).
//!
//! # Sync with `src/bin/fake-claude.rs`
//!
//! `armadai` is a binary-only crate (no `lib.rs`), so `src/bin/fake-claude.rs` cannot
//! be imported from the integration test crate. `tests/e2e/harness.rs` serializes
//! the `fake` block of a case file back to YAML (`serde_yaml_ng::to_string`) for
//! `fake-claude` to deserialize, so [`Rule`] and [`Match`] below **must keep the exact
//! same serde shape** (field names,
//! `match` → `match_` rename, optionality) as their counterparts in
//! `src/bin/fake-claude.rs`. They are duplicated here — not shared — because the bin
//! target isn't a library crate. **If you change one, change the other.** A shape
//! mismatch would only surface at runtime (a case file's `fake` block failing to
//! deserialize inside `fake-claude`), so review both files together on any edit.
//!
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One end-to-end test case: setup + scripted fake responses + expectations.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CaseFile {
    /// Human-readable case name (also used as the identifier in reports).
    pub name: String,
    /// Weight used by the report aggregator to prioritize failures.
    pub weight: u32,
    /// A known/tolerated failing case does not fail the overall suite.
    #[serde(default)]
    pub allow_fail: bool,
    pub setup: Setup,
    pub fake: FakeSpec,
    pub expect: Expect,
}

/// How to invoke the real `armadai` binary for this case.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Setup {
    /// Orchestration pattern under test (e.g. `direct`, `hierarchical`, `blackboard`).
    pub pattern: String,
    /// Agent ids involved in the run, in invocation order.
    #[serde(default)]
    pub agents: Vec<String>,
    /// Extra CLI flags passed to `armadai run` (e.g. `--json`).
    #[serde(default)]
    pub flags: Vec<String>,
    /// Input piped/passed to the run.
    #[serde(default)]
    pub input: String,
    /// C9 override for `hierarchical` cases: when set, `harness::project_yaml`
    /// emits a single `orchestration.teams` entry with this `lead`/`pattern`/
    /// `agents` (a nested blackboard/ring sub-team) instead of the default flat
    /// team made of every non-coordinator agent. `serde(default)` keeps every
    /// existing (non-nested) case file parsing exactly as before.
    #[serde(default)]
    pub nested_team: Option<NestedTeamSetup>,
    /// OH1 Lot 4 Task 4: a low global token budget, rendered by
    /// `harness::project_yaml` as `defaults.orchestration.token_budget`.
    /// That's the key `blackboard`/`ring` actually read (via
    /// `OrchestrationDefaults`/`apply_blackboard_overrides`/
    /// `apply_ring_overrides` in `src/cli/run.rs`) — NOT the top-level
    /// `orchestration.token_budget`, which only feeds the `hierarchical`
    /// pattern's own `OrchestrationConfig`. Used to deterministically trigger
    /// a graceful budget halt (`ExecutionEvent::Warned{code: "token_budget"}`
    /// + a partial `Complete`) for `cases/budget-halt-visible.yaml`.
    #[serde(default)]
    pub token_budget: Option<u64>,
}

/// Describes a C9 nested team for a `hierarchical` [`Setup`]: `lead` runs the
/// sub-`pattern` (`blackboard`/`ring`) over `agents` instead of delegating to
/// them individually. `lead` and every name in `agents` must also appear in
/// the parent [`Setup::agents`] list (that's what makes `harness::write_project`
/// generate an `agents/<name>.md` file and a `fake-claude` provider for them).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NestedTeamSetup {
    /// Agent id that arbitrates the sub-run (the team's `lead`).
    pub lead: String,
    /// Sub-pattern the team runs: `blackboard` or `ring`.
    pub pattern: String,
    /// Team members that actually run the sub-pattern (excludes `lead`).
    pub agents: Vec<String>,
}

/// The scripted `fake-claude` scenario for this case.
///
/// `Serialize` is needed so the harness can round-trip this block back to YAML for
/// `fake-claude` to read (see the module doc) — that round-trip is itself the proof
/// that this shape stays compatible with `src/bin/fake-claude.rs::Scenario`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FakeSpec {
    pub rules: Vec<Rule>,
}

/// A single rule: a `match` predicate plus the scripted response/metrics.
///
/// Mirrors `Rule` in `src/bin/fake-claude.rs` — see the module-level doc comment on
/// why this is a duplicate rather than a shared type, and keep the two in sync.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    #[serde(rename = "match", default)]
    pub match_: Match,
    pub respond: String,
    #[serde(default)]
    pub tokens_in: Option<u32>,
    #[serde(default)]
    pub tokens_out: Option<u32>,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

/// Match predicate for a [`Rule`]. All fields are optional; an empty `Match` (all
/// `None`) is a catch-all that matches any agent/call/prompt.
///
/// Mirrors `Match` in `src/bin/fake-claude.rs` — keep in sync (see module doc).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Match {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub call: Option<u32>,
    #[serde(default)]
    pub prompt_contains: Option<String>,
}

/// The expected outcome of a run.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Expect {
    pub exit_code: i32,
    #[serde(default)]
    pub events: Vec<ExpectedEvent>,
    #[serde(default)]
    pub event_counts: BTreeMap<String, usize>,
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub storage: Option<BTreeMap<String, usize>>,
}

/// A partial match against one emitted event: `t` is the event type, and any other
/// field present is matched against the corresponding field on the actual event
/// (fields absent here are not checked).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExpectedEvent {
    pub t: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// Load and parse a case file from `path`.
pub fn load_case(path: &Path) -> anyhow::Result<CaseFile> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading case file {}", path.display()))?;
    load_case_str(&contents).with_context(|| format!("parsing case file {}", path.display()))
}

/// Parse a case file directly from a YAML string (inline fixtures in tests, or any
/// other in-memory source — `load_case` is just this plus a file read).
pub fn load_case_str(yaml: &str) -> anyhow::Result<CaseFile> {
    serde_yaml_ng::from_str(yaml).context("parsing case YAML")
}

/// Render the JSON Schema for [`CaseFile`] as a pretty-printed string.
pub fn case_json_schema() -> String {
    let schema = schemars::schema_for!(CaseFile);
    serde_json::to_string_pretty(&schema).expect("schema serializes to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_valid_case_file() {
        let yaml = r#"
name: direct-happy
weight: 5
setup: { pattern: direct, agents: [t-writer], flags: ["--json"], input: "hi" }
fake: { rules: [ { match: { agent: t-writer }, respond: "done" } ] }
expect:
  exit_code: 0
  events: [ { t: run_start }, { t: agent_start, agent: t-writer }, { t: result } ]
  event_counts: { agent_start: 1, agent_end: 1 }
  invariants: [agent_start_end_symmetric, single_result]
"#;
        let c: CaseFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(c.name, "direct-happy");
        assert_eq!(c.weight, 5);
        assert_eq!(c.setup.agents, vec!["t-writer"]);
        assert_eq!(c.expect.events.len(), 3);
        assert!(c.expect.invariants.contains(&"single_result".to_string()));
    }

    #[test]
    fn rejects_unknown_pattern_or_missing_field() {
        // missing `name` → serde error
        assert!(serde_yaml_ng::from_str::<CaseFile>("weight: 1").is_err());
    }

    #[test]
    fn load_case_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("case.yaml");
        std::fs::write(
            &path,
            r#"
name: from-disk
weight: 1
setup: { pattern: direct, agents: [], flags: [], input: "" }
fake: { rules: [ { match: {}, respond: "ok" } ] }
expect: { exit_code: 0 }
"#,
        )
        .unwrap();
        let c = load_case(&path).unwrap();
        assert_eq!(c.name, "from-disk");
    }

    #[test]
    fn load_case_reports_error_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.yaml");
        assert!(load_case(&missing).is_err());
    }

    /// Generates `docs/e2e-case.schema.json` from the current `CaseFile` shape. This
    /// intentionally writes into the repo (not a tempdir): the schema is a committed
    /// doc artifact, regenerated by running this test whenever the format changes.
    #[test]
    fn emit_schema() {
        let schema = case_json_schema();
        assert!(schema.contains("\"CaseFile\""));
        // `docs/` lives at the workspace root, two levels above this crate
        // (`crates/armadai/`), not under `CARGO_MANIFEST_DIR` itself.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/e2e-case.schema.json");
        std::fs::write(path, schema).unwrap();
    }
}
