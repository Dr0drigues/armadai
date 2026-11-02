//! Graphviz DOT export of a flow, from `FlowGraph`: ids and labels are quoted
//! strings (`\` and `"` escaped), a start step is `rounded,bold`, an end step
//! has a double outline, `failed` edges are dashed.

use crate::delegation::TeamGraph;
use crate::graph::{EdgeKind, FlowGraph, flow_graph};
use crate::load::Flow;

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The flow as one `digraph`, opened by a `// flow: <id>` comment line.
pub fn flow_to_dot(id: &str, flow: &Flow) -> String {
    flow_graph_to_dot(&flow_graph(id, flow))
}

pub fn flow_graph_to_dot(g: &FlowGraph) -> String {
    let mut out = format!(
        "// flow: {}\ndigraph {} {{\n",
        g.id.replace(['\n', '\r'], " "),
        q(&g.id)
    );
    for n in &g.nodes {
        let label = if n.id == n.agent {
            n.id.clone()
        } else {
            format!("{} ({})", n.id, n.agent)
        };
        let mut attrs = format!("label={}, shape=box", q(&label));
        if n.start {
            attrs.push_str(", style=\"rounded,bold\"");
        }
        if n.end {
            attrs.push_str(", peripheries=2");
        }
        out.push_str(&format!("  {} [{attrs}];\n", q(&n.id)));
    }
    for e in &g.edges {
        let attrs = match (e.kind, e.when.as_deref()) {
            (EdgeKind::OnFailure, _) | (_, Some("failed")) => {
                " [label=\"failed\", style=dashed]".into()
            }
            (_, None | Some("done")) => String::new(),
            (_, Some(w)) => format!(" [label={}]", q(w)),
        };
        out.push_str(&format!("  {} -> {}{attrs};\n", q(&e.from), q(&e.to)));
    }
    out.push_str("}\n");
    out
}

/// The team as one `digraph "team"`, opened by its policy mention.
pub fn team_to_dot(g: &TeamGraph) -> String {
    let mut out = format!("// {}\ndigraph \"team\" {{\n", g.policy_note());
    out.push_str(&format!(
        "  \"main thread\" [label={}, shape=box, style=\"rounded,bold\"];\n",
        q(&g.main_label())
    ));
    for a in &g.agents {
        out.push_str(&format!(
            "  {} [label={}, shape=box];\n",
            q(&a.id),
            q(&a.label())
        ));
    }
    if !g.free_agents.is_empty() {
        out.push_str(
            "  subgraph \"cluster_free\" {\n    label=\"free agents, callable from anywhere\";\n",
        );
        for f in &g.free_agents {
            out.push_str(&format!("    {} [shape=box];\n", q(f)));
        }
        out.push_str("  }\n");
    }
    for e in &g.edges {
        out.push_str(&format!(
            "  {} -> {};\n",
            q(e.from.as_deref().unwrap_or("main thread")),
            q(&e.to)
        ));
    }
    out.push_str("}\n");
    out
}
