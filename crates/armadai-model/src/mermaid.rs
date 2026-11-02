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
    let mut out = format!("%% flow: {}\nflowchart TD\n", g.id);
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
