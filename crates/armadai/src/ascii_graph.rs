//! ASCII boxes for a flow or for the team, as pure functions of the graphs of
//! `armadai-model` (no terminal, no colour: reusable by `flow status --watch`).
//!
//! `ascii-dag` only draws acyclic graphs well, so it is given the "forward" edges
//! (the target comes after the source in the node order of the graph) and every
//! other edge (a return path, a self loop, an edge to an unknown step) is listed
//! under `links not drawn:`.

use ascii_dag::Graph;

/// `(from, to, label)` as indexes into the labels; names past `boxes` only name
/// unknown targets.
type Edge<'a> = (usize, usize, Option<&'a str>);

/// `names` are what the list of undrawn links calls the nodes (no start or end mark).
fn draw(labels: &[String], names: &[String], boxes: usize, edges: &[Edge<'_>]) -> String {
    let mut dag = Graph::new();
    for (i, l) in labels.iter().enumerate().take(boxes) {
        dag.add_node(i, l.as_str());
    }
    let mut not_drawn = Vec::new();
    for &(from, to, when) in edges {
        if from < to && to < boxes {
            dag.add_edge(from, to, when);
        } else {
            let w = when.map_or(String::new(), |w| format!(" ({w})"));
            not_drawn.push(format!("  {} → {}{w}", names[from], names[to]));
        }
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
            out.push_str(&l);
            out.push('\n');
        }
    }
    out
}

pub fn flow_to_ascii(g: &armadai_model::FlowGraph) -> String {
    let mut names: Vec<String> = g.nodes.iter().map(|n| n.id.clone()).collect();
    let labels: Vec<String> = g
        .nodes
        .iter()
        .map(|n| {
            let base = format!("{} ({})", n.id, n.agent);
            match (n.start, n.end) {
                (true, _) => format!("▶ {base}"),
                (false, true) => format!("{base} ■"),
                _ => base,
            }
        })
        .collect();
    let boxes = labels.len();
    let index = |id: &str| g.nodes.iter().position(|n| n.id == id);
    let mut edges: Vec<Edge<'_>> = Vec::new();
    for e in &g.edges {
        let Some(from) = index(&e.from) else { continue };
        // an edge to an unknown step has no box: its label sits past `boxes`, so it is listed
        let to = index(&e.to).unwrap_or_else(|| {
            names.push(e.to.clone());
            names.len() - 1
        });
        let when = match (&e.when, e.kind) {
            (Some(w), _) => Some(w.as_str()),
            (None, armadai_model::EdgeKind::OnFailure) => Some("failed"),
            _ => None,
        };
        edges.push((from, to, when));
    }
    format!("flow: {}\n{}", g.id, draw(&labels, &names, boxes, &edges))
}

pub fn team_to_ascii(g: &armadai_model::TeamGraph) -> String {
    let mut ids: Vec<Option<&str>> = vec![None];
    let mut labels = vec![match &g.coordinator {
        Some(c) => format!("main thread ({c})"),
        None => "main thread".to_string(),
    }];
    let label = |a: &armadai_model::TeamAgent| {
        if a.flows.is_empty() {
            a.id.clone()
        } else {
            format!("{} · {}", a.id, a.flows.join(", "))
        }
    };
    for (a, free) in
        (g.agents.iter().map(|a| (a, false))).chain(g.free_agents.iter().map(|a| (a, true)))
    {
        ids.push(Some(&a.id));
        // the group of free agents is a subgraph in Mermaid; a box has none
        labels.push(if free {
            format!("{} (free)", label(a))
        } else {
            label(a)
        });
    }
    let enforced = if g.delegation == "strict" {
        "enforced by the gate"
    } else {
        "not enforced"
    };
    let edges: Vec<Edge<'_>> = g
        .edges
        .iter()
        .filter_map(|e| {
            Some((
                ids.iter().position(|i| *i == e.from.as_deref())?,
                ids.iter().position(|i| *i == Some(e.to.as_str()))?,
                None,
            ))
        })
        .collect();
    format!(
        "team (delegation: {}, {enforced})\n{}",
        g.delegation,
        draw(&labels, &labels, labels.len(), &edges)
    )
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
  [▶ plan (plan)]
         │
         ↓
  [build (build)]
         │
         ↓
  [check (check)]

links not drawn:
  check → build (failed)
";

    const REVIEW_LOOP: &str = "flow: review-loop
      [▶ dev (dev)]
            │
            ↓
       [rev (rev)]
            │
       \"approved\"
            ↓
      [ops (ops) ■]

links not drawn:
  rev → dev (rejected)
";

    const PIPELINE: &str = "flow: pipeline
  [▶ a (a)]
      │
      ↓
   [b (b)]
      │
      ↓
  [c (c) ■]
";

    const FAN_OUT: &str = "flow: fan-out
     [▶ lead (lead)]
      ┌─────┴─────┐
      ↓           ↓
  [x (x) ■]   [y (y) ■]
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
        assert!(rl.lines().any(|l| l == "       \"approved\""), "{rl}");
    }

    #[test]
    fn plan_build_verify_reads_top_down() {
        let out = template(FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]);
        let line = |needle: &str| out.lines().position(|l| l.trim() == needle).unwrap();
        let (plan, build, check) = (
            line("[▶ plan (plan)]"),
            line("[build (build)]"),
            line("[check (check)]"),
        );
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
            └──────────┬───────┐
                       │       ↓
                       │ [dev · ship]
                       │       └──┐
                       └──────────┤
                                  ↓
                            [rev · ship]

links not drawn:
  rev · ship → dev · ship
"
        );
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
