//! `flow_graph`: node order, roles, edge kinds and order. Called directly, as
//! `flow status --watch` will, not through the `graph` command.

use std::collections::BTreeMap;

use armadai_model::*;

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn step(agent: &str, on_failure: Option<&str>) -> Step {
    Step {
        agent: agent.into(),
        states: Vec::new(),
        on_failure: on_failure.map(Into::into),
        optional: false,
    }
}

fn link(from: &str, to: &str, when: Option<&str>) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        when: when.map(Into::into),
    }
}

fn flow(steps: &[(&str, Step)], start: Option<&str>, links: Vec<Link>) -> Flow {
    Flow {
        session_link: None,
        description: None,
        start: start.map(Into::into),
        steps: steps
            .iter()
            .map(|(id, s)| (id.to_string(), s.clone()))
            .collect::<BTreeMap<_, _>>(),
        links,
    }
}

fn order(g: &FlowGraph) -> Vec<&str> {
    g.nodes.iter().map(|n| n.id.as_str()).collect()
}

fn template(t: FlowTemplate, agents: &[&str]) -> Flow {
    flow_from_template(t, &ids(agents), None).unwrap()
}

#[test]
fn nodes_follow_the_flow_not_the_alphabet() {
    let f = flow(
        &[
            ("alpha", step("a", None)),
            ("mid", step("m", None)),
            ("zeta", step("z", None)),
        ],
        None,
        vec![link("zeta", "alpha", None), link("alpha", "mid", None)],
    );
    assert_eq!(order(&flow_graph("f", &f)), ["zeta", "alpha", "mid"]);
}

#[test]
fn a_declared_start_comes_first_in_a_cycle() {
    let f = flow(
        &[
            ("fix", step("a", None)),
            ("review", step("b", None)),
            ("write", step("c", None)),
        ],
        Some("write"),
        vec![
            link("write", "review", None),
            link("review", "fix", None),
            link("fix", "write", None),
        ],
    );
    assert_eq!(order(&flow_graph("f", &f)), ["write", "review", "fix"]);
}

#[test]
fn links_are_followed_before_on_failure() {
    let f = flow(
        &[
            ("a", step("p", Some("b"))),
            ("b", step("q", None)),
            ("z", step("r", None)),
        ],
        Some("a"),
        vec![link("a", "z", None)],
    );
    assert_eq!(order(&flow_graph("f", &f)), ["a", "z", "b"]);
}

#[test]
fn unreachable_steps_come_last_alphabetically() {
    let f = flow(
        &[
            ("b", step("p", None)),
            ("c", step("q", None)),
            ("m", step("r", None)),
            ("s", step("t", None)),
            ("y", step("u", None)),
        ],
        Some("s"),
        vec![link("s", "m", None)],
    );
    assert_eq!(order(&flow_graph("f", &f)), ["s", "m", "b", "c", "y"]);
}

#[test]
fn cycles_terminate_with_each_step_once() {
    let g = flow_graph(
        "pbv",
        &template(FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]),
    );
    assert_eq!(order(&g), ["plan", "build", "check"]);
    let g = flow_graph(
        "rl",
        &template(FlowTemplate::ReviewLoop, &["dev", "rev", "ops"]),
    );
    assert_eq!(order(&g), ["dev", "rev", "ops"]);
}

#[test]
fn start_and_end_roles() {
    let g = flow_graph(
        "p",
        &template(FlowTemplate::Pipeline, &["one", "two", "three"]),
    );
    let roles: Vec<_> = g
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.start, n.end))
        .collect();
    assert_eq!(
        roles,
        [
            ("one", true, false),
            ("two", false, false),
            ("three", false, true)
        ]
    );
    // a step whose only way out is `on_failure` is not an end
    let f = flow(
        &[("a", step("p", Some("b"))), ("b", step("q", None))],
        Some("a"),
        vec![],
    );
    let g = flow_graph("f", &f);
    assert!(!g.nodes[0].end && g.nodes[1].end);
    // a lone step is both
    let g = flow_graph("s", &flow(&[("solo", step("s", None))], None, vec![]));
    assert_eq!((g.nodes[0].start, g.nodes[0].end), (true, true));
}

#[test]
fn edges_keep_their_kind_and_order() {
    let f = flow(
        &[
            ("write", step("dev", None)),
            ("check", step("rev", Some("write"))),
        ],
        None,
        vec![
            link("write", "check", None),
            link("check", "write", Some("rejected")),
        ],
    );
    let e = |from: &str, to: &str, when: Option<&str>, kind| GraphEdge {
        from: from.into(),
        to: to.into(),
        when: when.map(Into::into),
        kind,
    };
    assert_eq!(
        flow_graph("review", &f).edges,
        [
            e("write", "check", None, EdgeKind::Link),
            e("check", "write", Some("rejected"), EdgeKind::Link),
            e("check", "write", None, EdgeKind::OnFailure),
        ]
    );
}

#[test]
fn an_unknown_link_target_keeps_its_edge() {
    let f = flow(
        &[("a", step("p", None))],
        None,
        vec![link("a", "ghost", None)],
    );
    let g = flow_graph("f", &f);
    assert_eq!(order(&g), ["a"]);
    assert_eq!(
        (g.edges[0].from.as_str(), g.edges[0].to.as_str()),
        ("a", "ghost")
    );
    assert_eq!(g.edges.len(), 1);
}

#[test]
fn an_empty_flow_has_no_node_nor_edge() {
    let g = flow_graph("e", &flow(&[], None, vec![]));
    assert_eq!((g.id.as_str(), g.nodes.len(), g.edges.len()), ("e", 0, 0));
}

/// A step reached by two paths enters the queue once, so it is listed once.
#[test]
fn a_join_is_listed_once() {
    let f = flow(
        &[
            ("a", step("p", None)),
            ("b", step("q", None)),
            ("c", step("r", None)),
            ("d", step("s", None)),
        ],
        None,
        vec![
            link("a", "b", None),
            link("a", "c", None),
            link("b", "d", None),
            link("c", "d", None),
        ],
    );
    assert_eq!(order(&flow_graph("f", &f)), ["a", "b", "c", "d"]);
}

#[test]
fn flow_graph_json_golden() {
    let f = flow(
        &[
            ("write", step("dev", None)),
            ("check", step("rev", Some("write"))),
        ],
        None,
        vec![
            link("write", "check", None),
            link("check", "write", Some("rejected")),
        ],
    );
    let json = serde_json::to_string_pretty(&flow_graph("review", &f)).unwrap();
    assert_eq!(
        json,
        r#"{
  "id": "review",
  "nodes": [
    {
      "id": "write",
      "agent": "dev",
      "states": [],
      "start": true,
      "end": false
    },
    {
      "id": "check",
      "agent": "rev",
      "states": [],
      "start": false,
      "end": false
    }
  ],
  "edges": [
    {
      "from": "write",
      "to": "check",
      "when": null,
      "kind": "link"
    },
    {
      "from": "check",
      "to": "write",
      "when": "rejected",
      "kind": "link"
    },
    {
      "from": "check",
      "to": "write",
      "when": null,
      "kind": "on_failure"
    }
  ]
}"#
    );
}

#[test]
fn a_node_label_is_its_id_when_the_step_bears_its_agent_name() {
    let f = flow(
        &[("plan", step("plan", None)), ("write", step("dev", None))],
        Some("plan"),
        vec![link("plan", "write", None)],
    );
    let labels: Vec<String> = flow_graph("f", &f)
        .nodes
        .iter()
        .map(GraphNode::label)
        .collect();
    assert_eq!(labels, ["plan", "write (dev)"]);
}
