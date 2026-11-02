//! Who may delegate to whom: `allowed_targets` (the policy gate's rule, shared
//! with the OpenCode adapter) and the team graph drawn from it.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::check::entry_steps;
use crate::load::Project;
use crate::types::Delegation;

/// Agents `caller` may delegate to (`None` = the main thread).
///
/// - Main thread: with a coordinator, it IS the coordinator (it lives in
///   `CLAUDE.md`) and drives the flows: every step agent. Without one, the
///   agents of entry steps (`entry_steps`, the validator's own).
/// - The coordinator as `--agent`: same as the main thread.
/// - Agent X: the agents of the steps a step of X leads to (link or `on_failure`).
/// - `policy.free_agents`: always.
/// - The coordinator never, as a step agent: it is no sub-agent.
pub fn allowed_targets(project: &Project, caller: Option<&str>) -> BTreeSet<String> {
    let coordinator = project.manifest.coordinator.as_deref();
    let main = caller.is_none() || caller == coordinator;
    let mut out: BTreeSet<String> = project
        .manifest
        .policy
        .free_agents
        .iter()
        .cloned()
        .collect();
    for flow in project.flows.values() {
        let edges: Vec<(&str, &str)> = (flow.links.iter())
            .map(|l| (l.from.as_str(), l.to.as_str()))
            .chain(
                flow.steps
                    .iter()
                    .filter_map(|(sid, s)| s.on_failure.as_deref().map(|t| (sid.as_str(), t))),
            )
            .collect();
        let entries = entry_steps(flow);
        for (sid, step) in &flow.steps {
            let reached = if main {
                coordinator.is_some() || entries.contains(sid)
            } else {
                edges.iter().any(|(from, to)| {
                    to == sid
                        && flow
                            .steps
                            .get(*from)
                            .is_some_and(|s| Some(s.agent.as_str()) == caller)
                })
            };
            // the coordinator lives in CLAUDE.md: never a sub-agent to delegate to
            if reached && Some(step.agent.as_str()) != coordinator {
                out.insert(step.agent.clone());
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TeamGraph {
    /// `"off"` or `"strict"`, from `policy.delegation`.
    pub delegation: String,
    pub coordinator: Option<String>,
    /// Project agents but the coordinator, by id.
    pub agents: Vec<TeamAgent>,
    /// The main thread first, then the agents by id; targets sorted.
    pub edges: Vec<TeamEdge>,
    /// `policy.free_agents`, sorted: callable from anywhere, so drawn as a group.
    pub free_agents: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TeamAgent {
    pub id: String,
    /// The flows where the agent runs a step, sorted.
    pub flows: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TeamEdge {
    /// `None` is the main thread.
    pub from: Option<String>,
    pub to: String,
}

/// The delegations of the team, one edge per target `allowed_targets` gives,
/// less the free agents (a group of their own, reachable from everywhere).
pub fn team_graph(project: &Project) -> TeamGraph {
    let coordinator = project.manifest.coordinator.clone();
    let free: BTreeSet<String> = project
        .manifest
        .policy
        .free_agents
        .iter()
        .cloned()
        .collect();
    let agents: Vec<TeamAgent> = project
        .agents
        .keys()
        .filter(|id| Some(id.as_str()) != coordinator.as_deref())
        .map(|id| TeamAgent {
            id: id.clone(),
            flows: (project.flows.iter())
                .filter(|(_, f)| f.steps.values().any(|s| &s.agent == id))
                .map(|(fid, _)| fid.clone())
                .collect(),
        })
        .collect();
    let callers = std::iter::once(None).chain(agents.iter().map(|a| Some(a.id.as_str())));
    let edges = callers
        .flat_map(|caller| {
            allowed_targets(project, caller)
                .into_iter()
                .filter(|t| !free.contains(t))
                .map(move |to| TeamEdge {
                    from: caller.map(String::from),
                    to,
                })
        })
        .collect();
    TeamGraph {
        delegation: match project.manifest.policy.delegation {
            Delegation::Off => "off",
            Delegation::Strict => "strict",
        }
        .into(),
        coordinator,
        agents,
        edges,
        free_agents: free.into_iter().collect(),
    }
}

impl TeamGraph {
    /// The policy mention opening every rendering, e.g. `team (delegation: strict, enforced by the gate)`.
    pub(crate) fn policy_note(&self) -> String {
        let enforced = if self.delegation == "strict" {
            "enforced by the gate"
        } else {
            "not enforced"
        };
        format!("team (delegation: {}, {enforced})", self.delegation)
    }

    /// `main thread (lead)`, or `main thread` without a coordinator.
    pub(crate) fn main_label(&self) -> String {
        match &self.coordinator {
            Some(c) => format!("main thread ({c})"),
            None => "main thread".into(),
        }
    }
}

impl TeamAgent {
    /// `guardian · task`, or the bare id when no flow runs it.
    pub(crate) fn label(&self) -> String {
        if self.flows.is_empty() {
            self.id.clone()
        } else {
            format!("{} · {}", self.id, self.flows.join(", "))
        }
    }
}
