//! ASCII boxes for a flow or for the team, as pure functions of the graphs of
//! `armadai-model` (no terminal, no colour: reusable by `flow status --watch`).
//!
//! `ascii-dag` only draws acyclic graphs well, and one arrow per pair of boxes, so it
//! is given a forward, acyclic subset of the edges; every other edge (a return path,
//! a self loop, a parallel edge, an edge with an unknown end) is listed under
//! `links not drawn:`, so that every edge appears once.

use ascii_dag::Graph;

/// `(from, to, label)` as indexes into the labels.
type Edge<'a> = (usize, usize, Option<&'a str>);

/// The edge as the list of undrawn links writes it, `from` and `to` being plain names.
fn link_line(from: &str, to: &str, when: Option<&str>) -> String {
    let w = when.map_or(String::new(), |w| format!(" ({w})"));
    format!("  {from} → {to}{w}")
}

fn draw(labels: &[String], drawn: &[Edge<'_>], not_drawn: &[String]) -> String {
    let mut dag = Graph::new();
    for (i, l) in labels.iter().enumerate() {
        dag.add_node(i, l.as_str());
    }
    for &(from, to, when) in drawn {
        dag.add_edge(from, to, when);
    }
    let mut o = ascii_dag::RenderOptions::plain();
    o.emit.render_legend = true;
    let mut out = dag
        .compute_layout()
        .render_string(&o)
        .trim_end()
        .to_string();
    out.push('\n');
    if !not_drawn.is_empty() {
        out.push_str("\nlinks not drawn:\n");
        for l in not_drawn {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

pub fn flow_to_ascii(g: &armadai_model::FlowGraph) -> String {
    let labels: Vec<String> = g
        .nodes
        .iter()
        .map(|n| match (n.start, n.end) {
            (true, _) => format!("▶ {}", n.label()),
            (false, true) => format!("{} ■", n.label()),
            _ => n.label(),
        })
        .collect();
    let index = |id: &str| g.nodes.iter().position(|n| n.id == id);
    let (mut drawn, mut not_drawn): (Vec<Edge<'_>>, Vec<String>) = (Vec::new(), Vec::new());
    for e in &g.edges {
        let when = match (&e.when, e.kind) {
            (Some(w), _) => Some(w.as_str()),
            (None, armadai_model::EdgeKind::OnFailure) => Some("failed"),
            _ => None,
        };
        // forward, and the first edge of its pair: the crate draws one arrow per pair
        match (index(&e.from), index(&e.to)) {
            (Some(f), Some(t)) if f < t && !drawn.iter().any(|d| (d.0, d.1) == (f, t)) => {
                drawn.push((f, t, when))
            }
            _ => not_drawn.push(link_line(&e.from, &e.to, when)),
        }
    }
    format!("flow: {}\n{}", g.id, draw(&labels, &drawn, &not_drawn))
}

pub fn team_to_ascii(g: &armadai_model::TeamGraph) -> String {
    let main = g.main_label();
    let mut ids: Vec<Option<&str>> = vec![None];
    let mut labels = vec![main.clone()];
    for (a, free) in
        (g.agents.iter().map(|a| (a, false))).chain(g.free_agents.iter().map(|a| (a, true)))
    {
        ids.push(Some(&a.id));
        // the group of free agents is a subgraph in Mermaid; a box has none
        labels.push(if free {
            format!("{} (free)", a.label())
        } else {
            a.label()
        });
    }
    // In the order of `edges`, an edge is drawn when its target has not appeared in an
    // earlier edge: the first delegation that leads to an agent. Each drawn edge reaches a
    // new box, so the drawing is acyclic; the others are listed.
    let name = |id: Option<&str>| id.map_or(main.clone(), String::from);
    let mut seen = vec![false; ids.len()];
    seen[0] = true;
    let (mut drawn, mut not_drawn): (Vec<Edge<'_>>, Vec<String>) = (Vec::new(), Vec::new());
    for e in &g.edges {
        let pos = |id: Option<&str>| ids.iter().position(|i| *i == id);
        match (pos(e.from.as_deref()), pos(Some(e.to.as_str()))) {
            (Some(f), Some(t)) => {
                seen[f] = true;
                if seen[t] {
                    not_drawn.push(link_line(&name(e.from.as_deref()), &e.to, None));
                } else {
                    seen[t] = true;
                    drawn.push((f, t, None));
                }
            }
            // an end that is no box of the graph: listed, never dropped
            _ => not_drawn.push(link_line(&name(e.from.as_deref()), &e.to, None)),
        }
    }
    format!("{}\n{}", g.policy_note(), draw(&labels, &drawn, &not_drawn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use armadai_model::FlowTemplate;

    fn template(t: FlowTemplate, agents: &[&str]) -> String {
        let agents: Vec<String> = agents.iter().map(|a| a.to_string()).collect();
        let flow = armadai_model::flow_from_template(t, &agents, None).unwrap();
        flow_to_ascii(&armadai_model::flow_graph(t.as_str(), &flow))
    }

    const PBV: &str = "flow: plan-build-verify
  [▶ plan]
     ┌┘
     ↓
  [build]
     │
     ↓
  [check]

links not drawn:
  check → build (failed)
";

    const REVIEW_LOOP: &str = "flow: review-loop
      [▶ dev]
         │
         ↓
       [rev]
         │
    \"approved\"
         ↓
      [ops ■]

links not drawn:
  rev → dev (rejected)
";

    const PIPELINE: &str = "flow: pipeline
  [▶ a]
    │
    ↓
   [b]
    │
    ↓
  [c ■]
";

    const FAN_OUT: &str = "flow: fan-out
    [▶ lead]
    ┌───┴───┐
    ↓       ↓
  [x ■]   [y ■]
";

    fn all() -> [(String, &'static str); 4] {
        [
            (
                template(FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]),
                PBV,
            ),
            (
                template(FlowTemplate::ReviewLoop, &["dev", "rev", "ops"]),
                REVIEW_LOOP,
            ),
            (template(FlowTemplate::Pipeline, &["a", "b", "c"]), PIPELINE),
            (template(FlowTemplate::FanOut, &["lead", "x", "y"]), FAN_OUT),
        ]
    }

    #[test]
    fn every_step_is_named() {
        for (got, want) in all() {
            assert_eq!(got, want);
        }
    }

    #[test]
    fn a_cycle_names_every_link() {
        let pbv = template(FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]);
        assert!(
            pbv.lines().any(|l| l == "  check → build (failed)"),
            "{pbv}"
        );
        let rl = template(FlowTemplate::ReviewLoop, &["dev", "rev", "ops"]);
        assert!(rl.lines().any(|l| l == "  rev → dev (rejected)"), "{rl}");
        assert!(rl.lines().any(|l| l == "    \"approved\""), "{rl}");
    }

    #[test]
    fn plan_build_verify_reads_top_down() {
        let out = template(FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]);
        let line = |needle: &str| out.lines().position(|l| l.trim() == needle).unwrap();
        let (plan, build, check) = (line("[▶ plan]"), line("[build]"), line("[check]"));
        assert!(plan < build && build < check);
        // an arrow reaches `check` from `build`, and the return path is listed
        assert!(
            out.lines()
                .skip(build + 1)
                .take(check - build - 1)
                .any(|l| l.trim() == "↓")
        );
        assert_eq!(out, PBV);
    }

    #[test]
    fn an_edge_to_an_unknown_step_is_listed() {
        let flow = armadai_model::Flow {
            description: None,
            start: None,
            steps: [(
                "a".to_string(),
                armadai_model::Step {
                    agent: "x".into(),
                    states: vec![],
                    on_failure: None,
                },
            )]
            .into(),
            links: vec![armadai_model::Link {
                from: "a".into(),
                to: "ghost".into(),
                when: None,
            }],
        };
        let out = flow_to_ascii(&armadai_model::flow_graph("f", &flow));
        assert_eq!(
            out,
            "flow: f\n  [▶ a (x)]\n\nlinks not drawn:\n  a → ghost\n"
        );
    }

    #[test]
    fn the_team_in_boxes() {
        let a = |id: &str, flows: &[&str]| armadai_model::TeamAgent {
            id: id.into(),
            flows: flows.iter().map(|f| f.to_string()).collect(),
        };
        let e = |from: Option<&str>, to: &str| armadai_model::TeamEdge {
            from: from.map(String::from),
            to: to.into(),
        };
        let g = armadai_model::TeamGraph {
            delegation: "strict".into(),
            coordinator: Some("lead".into()),
            agents: vec![a("dev", &["ship"]), a("rev", &["ship"])],
            edges: vec![
                e(None, "dev"),
                e(None, "rev"),
                e(Some("dev"), "rev"),
                e(Some("rev"), "dev"),
            ],
            free_agents: vec![a("Explore", &[]), a("helper", &["task"])],
        };
        assert_eq!(
            team_to_ascii(&g),
            "team (delegation: strict, enforced by the gate)
  [main thread (lead)]   [Explore (free)]   [helper · task (free)]
            └─────────────┬──────────────┐
                          ↓              ↓
                    [dev · ship]   [rev · ship]

links not drawn:
  dev → rev
  rev → dev
"
        );
    }

    #[test]
    fn a_caller_met_first_is_not_a_target_again() {
        // `team_graph` of a project without coordinator and with the flow s1(z) -> s2(a) -> s3(b):
        // `a` delegates (a -> b) before `z -> a` is met
        let ag = |id: &str| armadai_model::TeamAgent {
            id: id.into(),
            flows: vec!["f".into()],
        };
        let e = |f: Option<&str>, t: &str| armadai_model::TeamEdge {
            from: f.map(String::from),
            to: t.into(),
        };
        let g = armadai_model::TeamGraph {
            delegation: "off".into(),
            coordinator: None,
            agents: vec![ag("a"), ag("b"), ag("z")],
            edges: vec![e(None, "z"), e(Some("a"), "b"), e(Some("z"), "a")],
            free_agents: vec![],
        };
        assert_eq!(
            team_to_ascii(&g),
            "team (delegation: off, not enforced)
  [main thread]   [a · f]
        │         ┌──┘
        ↓         ↓
     [z · f]   [b · f]

links not drawn:
  z → a
"
        );
    }

    #[test]
    fn a_mutual_delegation_is_not_drawn_as_a_cycle() {
        let ag = |id: &str| armadai_model::TeamAgent {
            id: id.into(),
            flows: vec![],
        };
        let e = |f: &str, t: &str| armadai_model::TeamEdge {
            from: Some(f.into()),
            to: t.into(),
        };
        let g = armadai_model::TeamGraph {
            delegation: "off".into(),
            coordinator: None,
            agents: vec![ag("a"), ag("b")],
            edges: vec![e("a", "b"), e("b", "a")],
            free_agents: vec![],
        };
        assert!(team_to_ascii(&g).ends_with("\nlinks not drawn:\n  b → a\n"));
    }

    #[test]
    fn a_team_edge_with_an_unknown_end_is_listed() {
        let g = armadai_model::TeamGraph {
            delegation: "off".into(),
            coordinator: None,
            agents: vec![armadai_model::TeamAgent {
                id: "dev".into(),
                flows: vec![],
            }],
            edges: vec![
                armadai_model::TeamEdge {
                    from: None,
                    to: "dev".into(),
                },
                armadai_model::TeamEdge {
                    from: Some("dev".into()),
                    to: "ghost".into(),
                },
            ],
            free_agents: vec![],
        };
        assert_eq!(
            team_to_ascii(&g),
            "team (delegation: off, not enforced)
  [main thread]
        │
        ↓
      [dev]

links not drawn:
  dev → ghost
"
        );
    }

    #[test]
    fn parallel_edges_are_drawn_once_and_the_others_listed() {
        let step = |states: &[&str], on_failure: Option<&str>| armadai_model::Step {
            agent: "x".into(),
            states: states.iter().map(|s| s.to_string()).collect(),
            on_failure: on_failure.map(String::from),
        };
        let link = |when: Option<&str>| armadai_model::Link {
            from: "a".into(),
            to: "b".into(),
            when: when.map(String::from),
        };
        let flow = armadai_model::Flow {
            description: None,
            start: None,
            steps: [
                ("a".to_string(), step(&["ok", "ko"], Some("b"))),
                ("b".to_string(), step(&[], None)),
            ]
            .into(),
            links: vec![link(Some("ok")), link(Some("ko"))],
        };
        let out = flow_to_ascii(&armadai_model::flow_graph("f", &flow));
        // the first edge of the pair is drawn, the two others are listed
        assert!(out.lines().any(|l| l.trim() == "\"ok\""), "{out}");
        assert!(
            out.ends_with("\nlinks not drawn:\n  a → b (ko)\n  a → b (failed)\n"),
            "{out}"
        );
        // an unconditional link first, then `on_failure` on the same pair
        let mut flow = flow;
        flow.links = vec![link(None)];
        let out = flow_to_ascii(&armadai_model::flow_graph("f", &flow));
        assert!(
            out.ends_with("\nlinks not drawn:\n  a → b (failed)\n"),
            "{out}"
        );
        assert!(!out.lines().any(|l| l.trim() == "\"failed\""), "{out}");
    }

    #[test]
    fn no_escape_sequence() {
        for (got, _) in all() {
            assert!(!got.contains('\x1b'), "{got:?}");
        }
    }

    #[test]
    fn it_terminates() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(all().len());
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
            4
        );
    }
}
