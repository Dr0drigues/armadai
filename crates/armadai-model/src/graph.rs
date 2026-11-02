//! Intermediate graph of a flow: nodes in reading order, edges in declaration
//! order, `start`/`end` roles. A pure value the renderers (Mermaid, DOT, ASCII,
//! JSON) all start from, so none of them re-derives the order or the roles.

use std::collections::{BTreeSet, VecDeque};

use serde::Serialize;

use crate::check::entry_steps;
use crate::load::Flow;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlowGraph {
    pub id: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub agent: String,
    pub states: Vec<String>,
    /// An entry step of the flow (`entry_steps`).
    pub start: bool,
    /// No edge, link or `on_failure`, leaves the step.
    pub end: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Link,
    OnFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub when: Option<String>,
    pub kind: EdgeKind,
}

/// Nodes: the entry steps, then a breadth-first walk (successors in edge
/// order, each step once), then the unreached steps alphabetically. Edges:
/// links in declared order, then each step's `on_failure` in step order; an
/// edge to an unknown step is kept (the validator reports it), without a node.
pub fn flow_graph(id: &str, flow: &Flow) -> FlowGraph {
    let edges: Vec<GraphEdge> = (flow.links.iter())
        .map(|l| GraphEdge {
            from: l.from.clone(),
            to: l.to.clone(),
            when: l.when.clone(),
            kind: EdgeKind::Link,
        })
        .chain(flow.steps.iter().filter_map(|(sid, s)| {
            s.on_failure.as_ref().map(|t| GraphEdge {
                from: sid.clone(),
                to: t.clone(),
                when: None,
                kind: EdgeKind::OnFailure,
            })
        }))
        .collect();

    let entries = entry_steps(flow);
    let mut order: Vec<&str> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    let known = |s: &str| flow.steps.contains_key(s);
    for e in entries.iter().filter(|e| known(e)) {
        if seen.insert(e) {
            queue.push_back(e);
        }
    }
    while let Some(cur) = queue.pop_front() {
        order.push(cur);
        for e in edges.iter().filter(|e| e.from == cur) {
            if known(&e.to) && seen.insert(&e.to) {
                queue.push_back(&e.to);
            }
        }
    }
    order.extend(
        flow.steps
            .keys()
            .map(String::as_str)
            .filter(|s| !seen.contains(s)),
    );

    let nodes = order
        .into_iter()
        .map(|sid| GraphNode {
            id: sid.to_string(),
            agent: flow.steps[sid].agent.clone(),
            states: flow.steps[sid].states.clone(),
            start: entries.iter().any(|e| e == sid),
            end: !edges.iter().any(|e| e.from == sid),
        })
        .collect();
    FlowGraph {
        id: id.to_string(),
        nodes,
        edges,
    }
}
