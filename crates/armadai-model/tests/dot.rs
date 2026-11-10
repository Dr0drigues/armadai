//! DOT export: exact goldens, hostile ids, determinism.

use std::collections::BTreeMap;

use armadai_model::*;

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn step(agent: &str) -> Step {
    Step {
        agent: agent.into(),
        states: Vec::new(),
        on_failure: None,
        optional: false,
    }
}

fn tpl(t: FlowTemplate, a: &[&str]) -> Flow {
    flow_from_template(t, &ids(a), None).unwrap()
}

#[test]
fn pipeline_golden() {
    let f = tpl(FlowTemplate::Pipeline, &["writer", "reviewer", "publisher"]);
    assert_eq!(
        flow_to_dot("ship", &f),
        r#"// flow: ship
digraph "ship" {
  "writer" [label="writer", shape=box, style="rounded,bold"];
  "reviewer" [label="reviewer", shape=box];
  "publisher" [label="publisher", shape=box, peripheries=2];
  "writer" -> "reviewer";
  "reviewer" -> "publisher";
}
"#
    );
}

#[test]
fn review_loop_golden() {
    let f = tpl(FlowTemplate::ReviewLoop, &["dev", "rev", "ops"]);
    assert_eq!(
        flow_to_dot("review", &f),
        r#"// flow: review
digraph "review" {
  "dev" [label="dev", shape=box, style="rounded,bold"];
  "rev" [label="rev", shape=box];
  "ops" [label="ops", shape=box, peripheries=2];
  "dev" -> "rev";
  "rev" -> "dev" [label="rejected"];
  "rev" -> "ops" [label="approved"];
}
"#
    );
}

#[test]
fn plan_build_verify_golden_draws_on_failure_dashed() {
    let f = tpl(FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]);
    assert_eq!(
        flow_to_dot("pbv", &f),
        r#"// flow: pbv
digraph "pbv" {
  "plan" [label="plan", shape=box, style="rounded,bold"];
  "build" [label="build", shape=box];
  "check" [label="check", shape=box];
  "plan" -> "build";
  "build" -> "check";
  "check" -> "build" [label="failed", style=dashed];
}
"#
    );
}

#[test]
fn fan_out_golden() {
    let f = tpl(FlowTemplate::FanOut, &["lead", "schema", "cli"]);
    assert_eq!(
        flow_to_dot("delivery", &f),
        r#"// flow: delivery
digraph "delivery" {
  "lead" [label="lead", shape=box, style="rounded,bold"];
  "schema" [label="schema", shape=box, peripheries=2];
  "cli" [label="cli", shape=box, peripheries=2];
  "lead" -> "schema";
  "lead" -> "cli";
}
"#
    );
}

/// A step that is start and end carries both marks; a named step shows its agent.
#[test]
fn a_lone_step_is_start_and_end() {
    let f = Flow {
        description: None,
        start: None,
        steps: BTreeMap::from([("solo".to_string(), step("worker"))]),
        links: vec![],
    };
    assert_eq!(
        flow_to_dot("s", &f),
        "// flow: s\ndigraph \"s\" {\n  \"solo\" [label=\"solo (worker)\", shape=box, style=\"rounded,bold\", peripheries=2];\n}\n"
    );
}

/// Quotes, backslashes and keywords stay inside quoted strings.
#[test]
fn hostile_ids_are_quoted() {
    let f = Flow {
        description: None,
        start: None,
        steps: BTreeMap::from([
            ("a\"b".to_string(), step("graph")),
            ("c\\d".to_string(), step("c\\d")),
            ("end".to_string(), step("end")),
            ("graph".to_string(), step("graph")),
        ]),
        links: vec![Link {
            from: "a\"b".into(),
            to: "c\\d".into(),
            when: Some("x\"y".into()),
        }],
    };
    assert_eq!(
        flow_to_dot("t", &f),
        r#"// flow: t
digraph "t" {
  "a\"b" [label="a\"b (graph)", shape=box, style="rounded,bold"];
  "end" [label="end", shape=box, style="rounded,bold", peripheries=2];
  "graph" [label="graph", shape=box, style="rounded,bold", peripheries=2];
  "c\\d" [label="c\\d", shape=box, peripheries=2];
  "a\"b" -> "c\\d" [label="x\"y"];
}
"#
    );
}

#[test]
fn the_dot_export_is_deterministic() {
    let f = tpl(FlowTemplate::ReviewLoop, &["a", "b", "c"]);
    let first = flow_to_dot("x", &f);
    for _ in 0..5 {
        assert_eq!(first, flow_to_dot("x", &f.clone()));
    }
}

/// `when: failed` is dashed like `on_failure`; `when: done` is a plain arrow.
#[test]
fn failed_and_done_links() {
    let f = Flow {
        description: None,
        start: None,
        steps: BTreeMap::from([
            ("a".to_string(), step("p")),
            ("b".to_string(), step("q")),
            ("c".to_string(), step("r")),
        ]),
        links: vec![
            Link {
                from: "a".into(),
                to: "b".into(),
                when: Some("done".into()),
            },
            Link {
                from: "a".into(),
                to: "c".into(),
                when: Some("failed".into()),
            },
        ],
    };
    assert_eq!(
        flow_to_dot("w", &f),
        r#"// flow: w
digraph "w" {
  "a" [label="a (p)", shape=box, style="rounded,bold"];
  "b" [label="b (q)", shape=box, peripheries=2];
  "c" [label="c (r)", shape=box, peripheries=2];
  "a" -> "b";
  "a" -> "c" [label="failed", style=dashed];
}
"#
    );
}

/// A newline in the id must not leave the comment line, or the output is no
/// longer DOT (nor Mermaid).
#[test]
fn a_newline_in_the_id_stays_in_the_comment() {
    let f = Flow {
        description: None,
        start: None,
        steps: BTreeMap::from([("a".to_string(), step("a"))]),
        links: vec![],
    };
    let dot = flow_to_dot("x\ny", &f);
    assert_eq!(dot.lines().next(), Some("// flow: x y"));
    assert!(dot.contains("\ndigraph \"x\ny\" {\n"), "{dot}");
    let mer = flow_to_mermaid("x\r\ny", &f);
    assert_eq!(mer.lines().next(), Some("%% flow: x  y"));
    assert_eq!(mer.lines().nth(1), Some("flowchart TD"));
}
