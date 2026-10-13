//! Mermaid export of a flow: `flowchart TD`, deterministic (nodes by step id,
//! edges in link order then the `on_failure` paths in step order).
//!
//! Every node id is `n_` + the step id with `[^A-Za-z0-9_]` turned into `_`: the
//! prefix keeps clear of Mermaid's keywords (`end`, `graph`, ...) and of ids a
//! leading `o`/`x` would turn into an edge head. Labels are quoted, with `"` and
//! `|` written as Mermaid entities.

use std::collections::{BTreeMap, BTreeSet};

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
    let mut out = format!("%% flow: {id}\nflowchart TD\n");
    let mut taken = BTreeSet::new();
    let mut node: BTreeMap<&str, String> = BTreeMap::new();
    for (sid, step) in &flow.steps {
        let mut n = base(sid);
        while !taken.insert(n.clone()) {
            n.push_str("_2");
        }
        let label = if *sid == step.agent {
            sid.clone()
        } else {
            format!("{sid} ({})", step.agent)
        };
        out.push_str(&format!("  {n}[\"{}\"]\n", escape(&label)));
        node.insert(sid, n);
    }
    // a link to an unknown step is the validator's to report: drawn with a node named after it
    let id_of = |s: &str| node.get(s).cloned().unwrap_or_else(|| base(s));
    let edges = (flow.links.iter())
        .map(|l| (l.from.as_str(), l.to.as_str(), l.when.as_deref()))
        .chain(flow.steps.iter().filter_map(|(sid, s)| {
            (s.on_failure.as_deref()).map(|t| (sid.as_str(), t, Some("failed")))
        }));
    for (from, to, when) in edges {
        let arrow = match when {
            None | Some("done") => "-->".to_string(),
            Some("failed") => "-.->|\"failed\"|".to_string(),
            Some(w) => format!("-->|\"{}\"|", escape(w)),
        };
        out.push_str(&format!("  {} {arrow} {}\n", id_of(from), id_of(to)));
    }
    out
}
