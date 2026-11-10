//! The JSON of `armadai flow status --json` (schema `armadai.flow-status/1`).
//!
//! The schema only grows: a key is never renamed, retyped or removed, a new one
//! is added (`docs/wiki/flow-status.md`, "Machine-readable output").

use serde_json::{Value, json};

use super::record::FlowEvent;
use super::runs::Run;

pub const SCHEMA: &str = "armadai.flow-status/1";

/// `session`: (id, events of its log), `None` when no log exists at all. The
/// runs keep the order they started in, not the display order of the text.
pub fn document(session: Option<(&str, &[FlowEvent])>, runs: &[Run]) -> Value {
    let session = session.map(|(id, events)| {
        json!({
            "id": id,
            "started_at": events.first().map(|e| rfc3339(e.ts)),
            "last_event_at": events.last().map(|e| rfc3339(e.ts)),
        })
    });
    json!({
        "schema": SCHEMA,
        "session": session,
        "runs": runs.iter().map(run).collect::<Vec<_>>(),
    })
}

fn run(r: &Run) -> Value {
    let status = if r.finished {
        "finished"
    } else if r.interrupted {
        "interrupted"
    } else {
        "in-progress"
    };
    json!({
        "flow": r.flow,
        "status": status,
        "steps": r.steps.iter().map(|v| json!({
            "step": v.step,
            "mark": v.mark.name(),
            "sign": v.mark.sign(),
            "state": v.state,
            "note": v.note,
        })).collect::<Vec<_>>(),
        "loops": r.loops.iter().map(|(from, to, times)| json!({
            "from": from, "to": to, "times": times,
        })).collect::<Vec<_>>(),
        "transitions": r.transitions.iter().map(|(ts, from, to, state)| json!({
            "at": rfc3339(*ts), "from": from, "to": to, "state": state,
        })).collect::<Vec<_>>(),
    })
}

fn rfc3339(ts: u64) -> Option<String> {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::super::record::EventKind;
    use super::super::runs::{Mark, StepView};
    use super::*;

    /// 2026-10-22 09:15:00 UTC
    const T0: u64 = 1_792_660_500;

    fn v(step: &str, mark: Mark, state: Option<&str>, note: Option<&str>) -> StepView {
        StepView {
            step: step.into(),
            mark,
            state: state.map(str::to_string),
            note: note.map(str::to_string),
        }
    }

    fn t(
        ts: u64,
        from: &str,
        to: &str,
        state: Option<&str>,
    ) -> (u64, String, String, Option<String>) {
        (ts, from.into(), to.into(), state.map(str::to_string))
    }

    fn ev(ts: u64) -> FlowEvent {
        FlowEvent {
            ts,
            event: EventKind::Start,
            agent_id: String::new(),
            agent_type: String::new(),
            state: None,
            command: None,
            flow: None,
            step: None,
            lot: None,
            reference: None,
        }
    }

    /// An in-progress run with a loop, a finished one and an interrupted one,
    /// in start order (the text shows the in-progress one last).
    fn three_runs() -> Vec<Run> {
        vec![
            Run {
                flow: "deliver".into(),
                steps: vec![
                    v("design", Mark::Done, Some("done"), None),
                    v("build", Mark::Done, Some("done"), None),
                    v("guard", Mark::Running, None, None),
                    v("validate", Mark::Pending, None, None),
                ],
                loops: vec![("build".into(), "guard".into(), 2)],
                transitions: vec![
                    t(T0 + 20, "design", "build", Some("done")),
                    t(T0 + 40, "guard", "build", Some("rejected")),
                ],
                finished: false,
                interrupted: false,
            },
            Run {
                flow: "hotfix".into(),
                steps: vec![
                    v("triage", Mark::Done, Some("done"), None),
                    v("check", Mark::Done, Some("approved"), None),
                ],
                loops: vec![],
                transitions: vec![t(T0 + 5, "triage", "check", None)],
                finished: true,
                interrupted: false,
            },
            Run {
                flow: "bugfix".into(),
                steps: vec![
                    v(
                        "frame",
                        Mark::Relay,
                        None,
                        Some("coordinator, not recorded"),
                    ),
                    v("check", Mark::Unsure, None, Some("no STATE line")),
                    v(
                        "verify",
                        Mark::Unexpected,
                        Some("scope_changed"),
                        Some("undeclared state"),
                    ),
                    v("ship", Mark::NotRun, None, None),
                ],
                loops: vec![],
                transitions: vec![],
                finished: false,
                interrupted: true,
            },
        ]
    }

    fn expected() -> Value {
        json!({
            "schema": "armadai.flow-status/1",
            "session": {
                "id": "s1",
                "started_at": "2026-10-22T09:15:00Z",
                "last_event_at": "2026-10-22T09:15:50Z"
            },
            "runs": [
                {
                    "flow": "deliver",
                    "status": "in-progress",
                    "steps": [
                        {"step": "design", "mark": "done", "sign": "✓", "state": "done", "note": null},
                        {"step": "build", "mark": "done", "sign": "✓", "state": "done", "note": null},
                        {"step": "guard", "mark": "running", "sign": "●", "state": null, "note": null},
                        {"step": "validate", "mark": "pending", "sign": "○", "state": null, "note": null}
                    ],
                    "loops": [{"from": "build", "to": "guard", "times": 2}],
                    "transitions": [
                        {"at": "2026-10-22T09:15:20Z", "from": "design", "to": "build", "state": "done"},
                        {"at": "2026-10-22T09:15:40Z", "from": "guard", "to": "build", "state": "rejected"}
                    ]
                },
                {
                    "flow": "hotfix",
                    "status": "finished",
                    "steps": [
                        {"step": "triage", "mark": "done", "sign": "✓", "state": "done", "note": null},
                        {"step": "check", "mark": "done", "sign": "✓", "state": "approved", "note": null}
                    ],
                    "loops": [],
                    "transitions": [
                        {"at": "2026-10-22T09:15:05Z", "from": "triage", "to": "check", "state": null}
                    ]
                },
                {
                    "flow": "bugfix",
                    "status": "interrupted",
                    "steps": [
                        {"step": "frame", "mark": "coordinator", "sign": "~", "state": null, "note": "coordinator, not recorded"},
                        {"step": "check", "mark": "unsure", "sign": "?", "state": null, "note": "no STATE line"},
                        {"step": "verify", "mark": "unexpected", "sign": "✗", "state": "scope_changed", "note": "undeclared state"},
                        {"step": "ship", "mark": "not-run", "sign": "-", "state": null, "note": null}
                    ],
                    "loops": [],
                    "transitions": []
                }
            ]
        })
    }

    fn doc() -> Value {
        document(Some(("s1", &[ev(T0), ev(T0 + 50)])), &three_runs())
    }

    #[test]
    fn three_runs_are_rendered_exactly() {
        assert_eq!(doc(), expected());
    }

    #[test]
    fn without_a_log_there_is_a_schema_and_nothing_else() {
        assert_eq!(
            document(None, &[]),
            json!({"schema": SCHEMA, "session": null, "runs": []})
        );
    }

    #[test]
    fn a_session_without_events_has_no_dates() {
        assert_eq!(
            document(Some(("S", &[])), &[])["session"],
            json!({"id": "S", "started_at": null, "last_event_at": null})
        );
    }

    fn kind(v: &Value) -> &'static str {
        match v {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// Every key path of the golden exists in `now` with the same JSON type
    /// (additions are allowed).
    fn check(path: &str, golden: &Value, now: &Value, missing: &mut Vec<String>) {
        if kind(golden) != kind(now) {
            missing.push(format!("{path}: {} became {}", kind(golden), kind(now)));
            return;
        }
        match (golden, now) {
            (Value::Object(g), Value::Object(n)) => {
                for (k, gv) in g {
                    match n.get(k) {
                        Some(nv) => check(&format!("{path}.{k}"), gv, nv, missing),
                        None => missing.push(format!("{path}.{k}: missing")),
                    }
                }
            }
            (Value::Array(g), Value::Array(n)) => {
                for (i, gv) in g.iter().enumerate() {
                    match n.get(i) {
                        Some(nv) => check(&format!("{path}[{i}]"), gv, nv, missing),
                        None => missing.push(format!("{path}[{i}]: missing")),
                    }
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_v1_golden_is_still_a_subset_of_the_output() {
        let golden: Value =
            serde_json::from_str(include_str!("../../tests/golden/flow-status/v1.json")).unwrap();
        let mut missing = Vec::new();
        check("$", &golden, &doc(), &mut missing);
        assert!(missing.is_empty(), "schema v1 broke: {missing:#?}");
    }
}
