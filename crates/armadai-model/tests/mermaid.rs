//! Mermaid export: exact goldens, hostile ids and labels, determinism.

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
    }
}

fn link(from: &str, to: &str, when: Option<&str>) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        when: when.map(Into::into),
    }
}

fn flow(steps: &[(&str, Step)], links: Vec<Link>) -> Flow {
    Flow {
        description: None,
        steps: steps
            .iter()
            .map(|(id, s)| (id.to_string(), s.clone()))
            .collect::<BTreeMap<_, _>>(),
        links,
    }
}

#[test]
fn pipeline_golden() {
    let f = flow_from_template(
        FlowTemplate::Pipeline,
        &ids(&["writer", "reviewer", "publisher"]),
        None,
    )
    .unwrap();
    assert_eq!(
        flow_to_mermaid("ship", &f),
        "%% flow: ship
flowchart TD
  n_publisher[\"publisher\"]
  n_reviewer[\"reviewer\"]
  n_writer[\"writer\"]
  n_writer --> n_reviewer
  n_reviewer --> n_publisher
"
    );
}

#[test]
fn review_loop_golden() {
    let f =
        flow_from_template(FlowTemplate::ReviewLoop, &ids(&["dev", "rev", "ops"]), None).unwrap();
    assert_eq!(
        flow_to_mermaid("review", &f),
        "%% flow: review
flowchart TD
  n_dev[\"dev\"]
  n_ops[\"ops\"]
  n_rev[\"rev\"]
  n_dev --> n_rev
  n_rev -->|\"rejected\"| n_dev
  n_rev -->|\"approved\"| n_ops
"
    );
}

#[test]
fn plan_build_verify_golden_draws_on_failure_dotted() {
    let f = flow_from_template(
        FlowTemplate::PlanBuildVerify,
        &ids(&["plan", "build", "check"]),
        None,
    )
    .unwrap();
    assert_eq!(
        flow_to_mermaid("pbv", &f),
        "%% flow: pbv
flowchart TD
  n_build[\"build\"]
  n_check[\"check\"]
  n_plan[\"plan\"]
  n_plan --> n_build
  n_build --> n_check
  n_check -.->|\"failed\"| n_build
"
    );
}

#[test]
fn fan_out_golden() {
    let f =
        flow_from_template(FlowTemplate::FanOut, &ids(&["lead", "schema", "cli"]), None).unwrap();
    assert_eq!(
        flow_to_mermaid("delivery", &f),
        "%% flow: delivery
flowchart TD
  n_cli[\"cli\"]
  n_lead[\"lead\"]
  n_schema[\"schema\"]
  n_lead --> n_schema
  n_lead --> n_cli
"
    );
}

/// `when: failed` is dotted like `on_failure`; `when: done` is a plain arrow;
/// a step named apart from its agent shows both.
#[test]
fn step_labels_and_link_kinds() {
    let f = flow(
        &[
            ("premier-jet", step("developpeur")),
            ("revue", step("relecteur")),
            ("corrections", step("developpeur")),
        ],
        vec![
            link("premier-jet", "revue", Some("done")),
            link("revue", "corrections", Some("failed")),
        ],
    );
    assert_eq!(
        flow_to_mermaid("f", &f),
        "%% flow: f
flowchart TD
  n_corrections[\"corrections (developpeur)\"]
  n_premier_jet[\"premier-jet (developpeur)\"]
  n_revue[\"revue (relecteur)\"]
  n_premier_jet --> n_revue
  n_revue -.->|\"failed\"| n_corrections
"
    );
}

/// Ids Mermaid would misread (`end`, a leading `o`/`x` read as an edge head, a
/// digit, spaces, unicode) and labels holding quotes, brackets and pipes.
#[test]
fn hostile_ids_and_labels_are_neutralised() {
    let mut odd = step("x\"[y](z)");
    odd.states = vec!["a|b\"c".into()];
    let f = flow(
        &[
            ("end", step("end")),
            ("o", step("o")),
            ("xray", step("graph")),
            ("1-a", step("subgraph")),
            ("a b", step("a b")),
            ("é", odd),
        ],
        vec![
            link("o", "end", None),
            link("xray", "o", None),
            link("1-a", "a b", None),
            link("é", "end", Some("a|b\"c")),
        ],
    );
    assert_eq!(
        flow_to_mermaid("t", &f),
        "%% flow: t
flowchart TD
  n_1_a[\"1-a (subgraph)\"]
  n_a_b[\"a b\"]
  n_end[\"end\"]
  n_o[\"o\"]
  n_xray[\"xray (graph)\"]
  n__[\"é (x#quot;[y](z))\"]
  n_o --> n_end
  n_xray --> n_o
  n_1_a --> n_a_b
  n__ -->|\"a#124;b#quot;c\"| n_end
"
    );
}

/// Two step ids sanitised to the same node get distinct nodes, in step order.
#[test]
fn colliding_node_ids_are_told_apart() {
    let f = flow(
        &[("a-b", step("p")), ("a_b", step("q")), ("a_b_2", step("r"))],
        vec![link("a-b", "a_b", None), link("a_b", "a_b_2", None)],
    );
    assert_eq!(
        flow_to_mermaid("c", &f),
        "%% flow: c
flowchart TD
  n_a_b[\"a-b (p)\"]
  n_a_b_2[\"a_b (q)\"]
  n_a_b_2_2[\"a_b_2 (r)\"]
  n_a_b --> n_a_b_2
  n_a_b_2 --> n_a_b_2_2
"
    );
}

#[test]
fn an_empty_flow_is_a_bare_flowchart() {
    let f = flow(&[], vec![]);
    assert_eq!(flow_to_mermaid("e", &f), "%% flow: e\nflowchart TD\n");
}

#[test]
fn the_export_is_deterministic() {
    let f = flow_from_template(FlowTemplate::ReviewLoop, &ids(&["a", "b", "c"]), None).unwrap();
    assert_eq!(flow_to_mermaid("x", &f), flow_to_mermaid("x", &f.clone()));
}
