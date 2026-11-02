//! Mermaid export of a flow: `flowchart TD`, deterministic (nodes in the order
//! of `FlowGraph`, edges in link order then the `on_failure` paths in step
//! order). A start step is a stadium, an end step a subroutine box (a step that
//! is both takes the stadium); `startStep`/`endStep` classes thicken them, and
//! are declared only when a node carries one (`end` itself is a Mermaid keyword).
//!
//! Every node id is `n_` + the step id with `[^A-Za-z0-9_]` turned into `_`: the
//! prefix keeps clear of Mermaid's keywords (`end`, `graph`, ...) and of ids a
//! leading `o`/`x` would turn into an edge head. Labels are quoted, with `"` and
//! `|` written as Mermaid entities.

use std::collections::{BTreeMap, BTreeSet};

use crate::delegation::TeamGraph;
use crate::graph::{EdgeKind, FlowGraph, flow_graph};
use crate::load::Flow;

fn escape(label: &str) -> String {
    label.replace('"', "#quot;").replace('|', "#124;")
}

fn base(step: &str) -> String {
    "n_".chars()
        .chain(step.chars().map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' => c,
            _ => '_',
        }))
        .collect()
}

/// The flow as one Mermaid block, opened by a `%% flow: <id>` comment line.
pub fn flow_to_mermaid(id: &str, flow: &Flow) -> String {
    flow_graph_to_mermaid(&flow_graph(id, flow))
}

pub fn flow_graph_to_mermaid(g: &FlowGraph) -> String {
    let mut out = format!(
        "%% flow: {}\nflowchart TD\n",
        g.id.replace(['\n', '\r'], " ")
    );
    let mut taken = BTreeSet::new();
    let mut node: BTreeMap<&str, String> = BTreeMap::new();
    let (mut starts, mut ends) = (Vec::new(), Vec::new());
    for nd in &g.nodes {
        let mut n = base(&nd.id);
        while !taken.insert(n.clone()) {
            n.push_str("_2");
        }
        let label = escape(&if nd.id == nd.agent {
            nd.id.clone()
        } else {
            format!("{} ({})", nd.id, nd.agent)
        });
        let shape = if nd.start {
            format!("([\"{label}\"])")
        } else if nd.end {
            format!("[[\"{label}\"]]")
        } else {
            format!("[\"{label}\"]")
        };
        out.push_str(&format!("  {n}{shape}\n"));
        if nd.start {
            starts.push(n.clone());
        }
        if nd.end {
            ends.push(n.clone());
        }
        node.insert(&nd.id, n);
    }
    // a link to an unknown step is the validator's to report: drawn with a node named after it
    let id_of = |s: &str| node.get(s).cloned().unwrap_or_else(|| base(s));
    for e in &g.edges {
        let arrow = match (e.kind, e.when.as_deref()) {
            (EdgeKind::OnFailure, _) | (_, Some("failed")) => "-.->|\"failed\"|".to_string(),
            (_, None | Some("done")) => "-->".to_string(),
            (_, Some(w)) => format!("-->|\"{}\"|", escape(w)),
        };
        out.push_str(&format!("  {} {arrow} {}\n", id_of(&e.from), id_of(&e.to)));
    }
    for (class, def, ids) in [
        ("startStep", "stroke-width:3px", &starts),
        ("endStep", "stroke-width:3px,stroke-dasharray:4 2", &ends),
    ] {
        if !ids.is_empty() {
            out.push_str(&format!("  classDef {class} {def}\n"));
        }
    }
    for (class, ids) in [("startStep", &starts), ("endStep", &ends)] {
        if !ids.is_empty() {
            out.push_str(&format!("  class {} {class}\n", ids.join(",")));
        }
    }
    out
}

/// The team as one Mermaid block, opened by its policy mention. The main thread
/// is `main`; each agent's flows follow its name; the free agents are a group
/// without edges.
pub fn team_to_mermaid(g: &TeamGraph) -> String {
    let mut out = format!("%% {}\nflowchart TD\n", g.policy_note());
    let mut taken = BTreeSet::from(["main".to_string()]);
    let mut unique = |id: &str| {
        let mut n = base(id);
        while !taken.insert(n.clone()) {
            n.push_str("_2");
        }
        n
    };
    out.push_str(&format!("  main([\"{}\"])\n", escape(&g.main_label())));
    let mut node: BTreeMap<&str, String> = BTreeMap::new();
    for a in &g.agents {
        let n = unique(&a.id);
        out.push_str(&format!("  {n}[\"{}\"]\n", escape(&a.label())));
        node.insert(&a.id, n);
    }
    if !g.free_agents.is_empty() {
        out.push_str("  subgraph free[\"free agents, callable from anywhere\"]\n");
        for f in &g.free_agents {
            let n = unique(f);
            out.push_str(&format!("    {n}[\"{}\"]\n", escape(f)));
        }
        out.push_str("  end\n");
    }
    for e in &g.edges {
        let from = e.from.as_deref().map_or("main".to_string(), |f| {
            node.get(f).cloned().unwrap_or_else(|| base(f))
        });
        let to = node
            .get(e.to.as_str())
            .cloned()
            .unwrap_or_else(|| base(&e.to));
        out.push_str(&format!("  {from} --> {to}\n"));
    }
    out
}
