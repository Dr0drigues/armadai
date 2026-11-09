//! ASCII boxes for a flow, and a list that fits 80 columns for the team, as pure functions of the graphs of
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
    flow_to_ascii_marked(g, &|_| (String::new(), String::new()))
}

/// `flow_to_ascii` with each node's label written `{before}{label}{after}`,
/// `(before, after) = wrap(step id)`; the label keeps its `▶ ` / ` ■`.
pub fn flow_to_ascii_marked(
    g: &armadai_model::FlowGraph,
    wrap: &dyn Fn(&str) -> (String, String),
) -> String {
    let labels: Vec<String> = g
        .nodes
        .iter()
        .map(|n| {
            let label = match (n.start, n.end) {
                (true, _) => format!("▶ {}", n.label()),
                (false, true) => format!("{} ■", n.label()),
                _ => n.label(),
            };
            let (before, after) = wrap(&n.id);
            format!("{before}{label}{after}")
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

const WIDTH: usize = 80;
/// The id column of the team list: a longer id is written alone on its line.
const ID_COLUMN: usize = 24;

fn cols(s: &str) -> usize {
    s.chars().count()
}

/// `first`, then the items joined by `, `, wrapped at `WIDTH` columns with `rest` as the
/// indent of the next lines. A word that does not fit a line alone is cut at the margin.
fn wrap(first: &str, rest: &str, items: &[String]) -> String {
    let mut lines = vec![first.to_string()];
    let mut fresh = true; // nothing but the indent on the current line
    for (i, item) in items.iter().enumerate() {
        let word = if i + 1 < items.len() {
            format!("{item},")
        } else {
            item.clone()
        };
        let cur = lines.last().unwrap();
        let sep = usize::from(!fresh);
        if cols(cur) + sep + cols(&word) <= WIDTH {
            let cur = lines.last_mut().unwrap();
            if !fresh {
                cur.push(' ');
            }
            cur.push_str(&word);
        } else {
            // the word starts a line, and is cut at the margin if it is longer
            let mut line = if fresh {
                lines.pop().unwrap()
            } else {
                rest.to_string()
            };
            let mut chars = word.chars().peekable();
            loop {
                let room = WIDTH - cols(&line);
                line.extend(chars.by_ref().take(room));
                lines.push(std::mem::take(&mut line));
                if chars.peek().is_none() {
                    break;
                }
                line = rest.to_string();
            }
        }
        fresh = false;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// One agent: its id and flows on a line, the agents it delegates to under it.
fn team_entry(id: &str, width: usize, flows: &[String], targets: &[String]) -> String {
    let mut out = String::new();
    if flows.is_empty() {
        out.push_str(&wrap("", "", &[id.to_string()]));
    } else if cols(id) > ID_COLUMN {
        out.push_str(&wrap("", "", &[id.to_string()]));
        out.push_str(&wrap("  flows: ", "         ", flows));
    } else {
        let head = format!("{id:<width$}  flows: ");
        let indent = " ".repeat(cols(&head));
        out.push_str(&wrap(&head, &indent, flows));
    }
    if !targets.is_empty() {
        out.push_str(&wrap("  → ", "    ", targets));
    }
    out
}

/// The team as a list that fits 80 columns: the main thread, then one entry per agent
/// with its flows and the agents it may delegate to, then the free agents.
pub fn team_to_ascii(g: &armadai_model::TeamGraph) -> String {
    let targets = |from: Option<&str>| -> Vec<String> {
        (g.edges.iter())
            .filter(|e| e.from.as_deref() == from)
            .map(|e| e.to.clone())
            .collect()
    };
    let mut callers: Vec<(&str, &[String])> = (g.agents.iter())
        .map(|a| (a.id.as_str(), a.flows.as_slice()))
        .collect();
    // a caller that is no agent of the graph is listed, never dropped
    for e in &g.edges {
        if let Some(f) = e.from.as_deref()
            && !callers.iter().any(|c| c.0 == f)
            && !g.free_agents.iter().any(|a| a.id == f)
        {
            callers.push((f, &[]));
        }
    }
    let width = (callers.iter().map(|c| cols(c.0)))
        .filter(|&n| n <= ID_COLUMN)
        .max()
        .unwrap_or(0);
    let mut out = format!("{}\n\n", g.policy_note());
    out.push_str(&wrap("", "", &[g.main_label()]));
    let main_targets = targets(None);
    if !main_targets.is_empty() {
        out.push_str(&wrap("  → ", "    ", &main_targets));
    }
    for (id, flows) in callers {
        out.push('\n');
        out.push_str(&team_entry(id, width, flows, &targets(Some(id))));
    }
    if !g.free_agents.is_empty() {
        let ids: Vec<String> = g.free_agents.iter().map(|a| a.id.clone()).collect();
        out.push('\n');
        out.push_str(&wrap("free agents, callable from anywhere: ", "    ", &ids));
        // a free agent of the project has flows and delegations of its own
        for a in &g.free_agents {
            let t = targets(Some(&a.id));
            if !a.flows.is_empty() || !t.is_empty() {
                out.push('\n');
                out.push_str(&team_entry(&a.id, width, &a.flows, &t));
            }
        }
    }
    out
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
    fn a_marked_drawing_wraps_each_label() {
        let agents: Vec<String> = ["a", "b", "c"].map(String::from).into();
        let flow =
            armadai_model::flow_from_template(FlowTemplate::Pipeline, &agents, None).unwrap();
        let g = armadai_model::flow_graph("pipeline", &flow);
        let out = flow_to_ascii_marked(&g, &|id| match id {
            "a" => ("✓ ".into(), "  ".into()),
            "b" => ("● ".into(), " |".into()),
            _ => ("○ ".into(), "  ".into()),
        });
        // trailing spaces are kept inside the box: the cell of the spinner is reserved
        assert_eq!(
            out,
            "flow: pipeline\n  [✓ ▶ a  ]\n      │\n      ↓\n   [● b |]\n      │\n      ↓\n  [○ c ■  ]\n"
        );
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

    fn ag(id: &str, flows: &[&str]) -> armadai_model::TeamAgent {
        armadai_model::TeamAgent {
            id: id.into(),
            flows: flows.iter().map(|f| f.to_string()).collect(),
        }
    }

    fn ed(from: Option<&str>, to: &str) -> armadai_model::TeamEdge {
        armadai_model::TeamEdge {
            from: from.map(String::from),
            to: to.into(),
        }
    }

    fn team(
        agents: Vec<armadai_model::TeamAgent>,
        edges: Vec<armadai_model::TeamEdge>,
        free: Vec<armadai_model::TeamAgent>,
    ) -> armadai_model::TeamGraph {
        armadai_model::TeamGraph {
            delegation: "strict".into(),
            coordinator: Some("lead".into()),
            agents,
            edges,
            free_agents: free,
        }
    }

    #[test]
    fn the_team_is_a_list_main_thread_first_then_one_entry_per_agent() {
        let g = team(
            vec![ag("dev", &["ship", "fix"]), ag("rev", &["ship"])],
            vec![
                ed(None, "dev"),
                ed(None, "rev"),
                ed(Some("dev"), "rev"),
                ed(Some("rev"), "dev"),
                ed(Some("rev"), "rev"),
            ],
            vec![ag("Explore", &[])],
        );
        assert_eq!(
            team_to_ascii(&g),
            "team (delegation: strict, enforced by the gate)

main thread (lead)
  → dev, rev

dev  flows: ship, fix
  → rev

rev  flows: ship
  → dev, rev

free agents, callable from anywhere: Explore
"
        );
    }

    #[test]
    fn an_agent_without_flow_or_delegation_is_its_id_alone() {
        let g = team(vec![ag("solo", &[])], vec![], vec![]);
        assert_eq!(
            team_to_ascii(&g),
            "team (delegation: strict, enforced by the gate)

main thread (lead)

solo
"
        );
    }

    #[test]
    fn an_id_longer_than_the_column_is_alone_on_its_line() {
        let long = "a-very-long-agent-identifier";
        let g = team(vec![ag(long, &["f"]), ag("b", &["f"])], vec![], vec![]);
        assert_eq!(
            team_to_ascii(&g),
            format!(
                "team (delegation: strict, enforced by the gate)

main thread (lead)

{long}
  flows: f

b  flows: f
"
            )
        );
    }

    #[test]
    fn a_free_agent_of_the_project_has_its_own_entry_and_an_unknown_caller_is_kept() {
        let g = team(
            vec![ag("dev", &["f"])],
            vec![
                ed(Some("dev"), "ghost"),
                ed(Some("ghost"), "dev"),
                ed(Some("fr"), "dev"),
            ],
            vec![ag("fr", &["g"]), ag("Explore", &[])],
        );
        assert_eq!(
            team_to_ascii(&g),
            "team (delegation: strict, enforced by the gate)

main thread (lead)

dev    flows: f
  → ghost

ghost
  → dev

free agents, callable from anywhere: fr, Explore

fr     flows: g
  → dev
"
        );
    }

    #[test]
    fn a_long_list_wraps_and_nothing_goes_past_80_columns() {
        let many: Vec<String> = (0..30).map(|i| format!("agent-number-{i:02}")).collect();
        let word = "x".repeat(100);
        let long_id = "i".repeat(100);
        let agents: Vec<_> = (many.iter().map(String::as_str))
            .map(|n| {
                ag(
                    n,
                    &[
                        "flow-one",
                        "flow-two",
                        "flow-three",
                        "flow-four",
                        "flow-five",
                    ],
                )
            })
            .chain([ag(&word, &[word.as_str()]), ag(&long_id, &[])])
            .collect();
        let edges: Vec<_> = (many.iter().map(String::as_str))
            .chain([word.as_str()])
            .map(|n| ed(None, n))
            .chain(many.iter().map(|n| ed(Some(&word), n)))
            .collect();
        let out = team_to_ascii(&team(agents, edges, vec![ag(&word, &[])]));
        for l in out.lines() {
            assert!(
                l.chars().count() <= 80,
                "{} columns: {l}",
                l.chars().count()
            );
        }
        // a cut word is cut, not dropped
        // as agent, in its flows, as a target of the main thread, in the free list and as a free entry
        assert_eq!(out.matches('x').count(), 100 * 5);
        assert!(out.contains("agent-number-29"), "{out}");
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
