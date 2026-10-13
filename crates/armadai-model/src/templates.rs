//! Flow templates: pure functions from an ordered list of agents to a [`Flow`].
//! Step ids are the agent ids; the result is checked by the validator like any flow.

use std::collections::{BTreeMap, BTreeSet};

use crate::load::Flow;
use crate::types::{Link, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowTemplate {
    /// `a → b → c`.
    Pipeline,
    /// author → reviewer; `rejected` back to the author, `approved` on to the
    /// shipper (none: the flow ends).
    ReviewLoop,
    /// planner → builder → verifier; the verifier's failure goes back to the builder.
    PlanBuildVerify,
    /// lead → each other agent (a lead handing work to its specialists).
    FanOut,
}

pub const FLOW_TEMPLATES: [FlowTemplate; 4] = [
    FlowTemplate::Pipeline,
    FlowTemplate::ReviewLoop,
    FlowTemplate::PlanBuildVerify,
    FlowTemplate::FanOut,
];

impl FlowTemplate {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowTemplate::Pipeline => "pipeline",
            FlowTemplate::ReviewLoop => "review-loop",
            FlowTemplate::PlanBuildVerify => "plan-build-verify",
            FlowTemplate::FanOut => "fan-out",
        }
    }

    /// How many agents the template takes: `(min, max)`.
    fn arity(self) -> (usize, Option<usize>) {
        match self {
            FlowTemplate::Pipeline => (2, None),
            FlowTemplate::ReviewLoop => (2, Some(3)),
            FlowTemplate::PlanBuildVerify => (3, Some(3)),
            FlowTemplate::FanOut => (2, None),
        }
    }
}

impl std::str::FromStr for FlowTemplate {
    type Err = TemplateError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FLOW_TEMPLATES
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| TemplateError::UnknownTemplate(s.into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error(
        "unknown flow template `{0}` (known: pipeline, review-loop, plan-build-verify, fan-out)"
    )]
    UnknownTemplate(String),
    #[error("the {template} template needs at least {min} agents")]
    TooFewAgents { template: &'static str, min: usize },
    #[error("the {template} template takes at most {max} agents")]
    TooManyAgents { template: &'static str, max: usize },
    #[error("agent `{0}` is listed twice")]
    DuplicateAgent(String),
    #[error("`{0}` is not a valid agent id (ids match [a-z0-9][a-z0-9-]{{0,63}})")]
    InvalidAgentId(String),
}

fn step(agent: &str) -> Step {
    Step {
        agent: agent.into(),
        states: Vec::new(),
        on_failure: None,
    }
}

fn link(from: &str, to: &str, when: Option<&str>) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        when: when.map(Into::into),
    }
}

/// The flow `t` lays over `agents`, in the given order.
pub fn flow_from_template(
    t: FlowTemplate,
    agents: &[String],
    description: Option<String>,
) -> Result<Flow, TemplateError> {
    if let Some(bad) = agents.iter().find(|a| !crate::is_valid_id(a)) {
        return Err(TemplateError::InvalidAgentId(bad.clone()));
    }
    let mut seen = BTreeSet::new();
    if let Some(dup) = agents.iter().find(|a| !seen.insert(a.as_str())) {
        return Err(TemplateError::DuplicateAgent(dup.clone()));
    }
    let template = t.as_str();
    match t.arity() {
        (min, _) if agents.len() < min => {
            return Err(TemplateError::TooFewAgents { template, min });
        }
        (_, Some(max)) if agents.len() > max => {
            return Err(TemplateError::TooManyAgents { template, max });
        }
        _ => {}
    }

    let mut steps: BTreeMap<String, Step> = agents.iter().map(|a| (a.clone(), step(a))).collect();
    let a = |i: usize| agents[i].as_str();
    let links = match t {
        FlowTemplate::Pipeline => agents
            .windows(2)
            .map(|w| link(&w[0], &w[1], None))
            .collect(),
        FlowTemplate::ReviewLoop => {
            if let Some(r) = steps.get_mut(a(1)) {
                r.states = vec!["approved".into(), "rejected".into()];
            }
            let mut links = vec![link(a(0), a(1), None), link(a(1), a(0), Some("rejected"))];
            if agents.len() == 3 {
                links.push(link(a(1), a(2), Some("approved")));
            }
            links
        }
        FlowTemplate::PlanBuildVerify => {
            if let Some(v) = steps.get_mut(a(2)) {
                v.on_failure = Some(a(1).into());
            }
            vec![link(a(0), a(1), None), link(a(1), a(2), None)]
        }
        FlowTemplate::FanOut => agents[1..].iter().map(|b| link(a(0), b, None)).collect(),
    };
    Ok(Flow {
        description,
        steps,
        links,
    })
}
