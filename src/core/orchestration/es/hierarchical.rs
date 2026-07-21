//! Pure transducer: LLM response → planned intentions (hierarchical pattern).
//!
//! `plan_from_response` reuses the existing (pure) delegation parser in
//! `crate::core::orchestration::protocol` and maps each parsed
//! `DelegationAction` onto a `PlannedStep` carrying the `ExecutionEvent` that
//! should be appended for it. No I/O, no async — this is the decision half
//! of the event-sourced hierarchical engine; effects (actually invoking
//! agents) are wired by a later lot.

use std::collections::BTreeMap;

use super::engine::{Action, Decider};
use super::event::ExecutionEvent;
use super::state::ExecutionState;
use crate::core::agent::Agent;
use crate::core::orchestration::OrchestrationConfig;
use crate::core::orchestration::protocol::{DelegationAction, parse_delegations};
use crate::core::routing::{BudgetState, RoutingRules, route};

/// A single step planned from an LLM response, before any effect has run.
///
/// This is the pure output of `plan_from_response`: either an agent to
/// invoke next (paired with the `ExecutionEvent` that documents *why*, e.g.
/// a delegation or an escalation), or a terminal `Complete` when the
/// response carried no delegation directive at all.
#[derive(Debug, Clone)]
pub enum PlannedStep {
    /// Invoke `agent` with `task`, recording `event` alongside the
    /// resulting `AgentInvoked` (see `es::engine::Action::Invoke` +
    /// `Action::Emit`).
    Invoke {
        agent: String,
        task: String,
        event: ExecutionEvent,
    },
    /// The response was a final answer — no further delegation.
    Complete { content: String },
}

/// Turn `sender`'s `response` into an ordered list of planned steps.
///
/// Reuses `parse_delegations` (already pure) to extract `@agent: message`
/// directives, then maps each `DelegationAction` onto a `PlannedStep`:
/// - `Delegate { target, task }` → `Invoke` + `ExecutionEvent::Delegated`
///   (`from: sender`, `to: target`, `depth: depth + 1`).
/// - `AskPeer { target, question }` → `Invoke` + `ExecutionEvent::AskedPeer`.
/// - `Escalate { target, message }` → `Invoke` + `ExecutionEvent::Escalated`.
/// - `FinalAnswer { content }` → `Complete { content }`.
///
/// The order of the returned steps follows the order of the lines in
/// `response` (same guarantee as `parse_delegations`), so replay is
/// deterministic.
///
/// `parse_delegations` needs the full `OrchestrationConfig` (coordinator +
/// team topology) to classify sender→target as Superior/Peer/Subordinate —
/// notably, two agents that only share a team (neither being the
/// coordinator) must classify as `Peer`, which requires walking
/// `config.teams`. This function takes the caller's `config` as-is and
/// forwards it unchanged, so callers must pass the real orchestration
/// config for the run (not a stripped-down stand-in), or peer relationships
/// will silently degrade to `Unknown` → `Delegate`.
pub fn plan_from_response(
    response: &str,
    sender: &str,
    config: &OrchestrationConfig,
    depth: u32,
) -> Vec<PlannedStep> {
    parse_delegations(response, sender, config)
        .into_iter()
        .map(|action| match action {
            DelegationAction::Delegate { target, task } => PlannedStep::Invoke {
                agent: target.clone(),
                task: task.clone(),
                event: ExecutionEvent::Delegated {
                    from: sender.to_string(),
                    to: target,
                    task,
                    depth: depth + 1,
                },
            },
            DelegationAction::AskPeer { target, question } => PlannedStep::Invoke {
                agent: target.clone(),
                task: question.clone(),
                event: ExecutionEvent::AskedPeer {
                    from: sender.to_string(),
                    to: target,
                    question,
                },
            },
            DelegationAction::Escalate { target, message } => PlannedStep::Invoke {
                agent: target.clone(),
                task: message.clone(),
                event: ExecutionEvent::Escalated {
                    from: sender.to_string(),
                    to: target,
                    message,
                },
            },
            DelegationAction::FinalAnswer { content } => PlannedStep::Complete { content },
        })
        .collect()
}

// ── HierarchicalDecider (Task 3): pure decision function ──────────

/// Max delegation depth reached so far, derived from `hier.trace`'s `depth`
/// field (populated by `Delegated` events; `AskedPeer`/`Escalated` entries
/// carry the documented placeholder `0` — see `es::state::apply` — and so
/// never raise this on their own).
fn current_depth(state: &ExecutionState) -> u32 {
    state
        .hier
        .trace
        .iter()
        .map(|(_, _, _, depth)| *depth)
        .max()
        .unwrap_or(0)
}

/// Total number of agent invocations so far. Derived from the number of
/// `user`-role messages across all conversations, since each `Invoke`
/// action records exactly one `AgentInvoked` event, which `apply` turns
/// into exactly one `user` message — a purely structural count, no extra
/// bookkeeping required.
fn invocation_count(state: &ExecutionState) -> usize {
    state
        .conversations
        .values()
        .map(|msgs| msgs.iter().filter(|m| m.role == "user").count())
        .sum()
}

/// Depth at which `agent` was itself invoked: the max `depth` among
/// `hier.trace` entries where `agent` is the delegation target (`to`), or
/// `0` if it was never delegated to (the coordinator, invoked directly from
/// the run's `input`, is the canonical `0` case).
fn depth_of(state: &ExecutionState, agent: &str) -> u32 {
    state
        .hier
        .trace
        .iter()
        .filter(|(_, to, _, _)| to == agent)
        .map(|(_, _, _, depth)| *depth)
        .max()
        .unwrap_or(0)
}

/// Number of assistant responses `agent` has produced so far.
fn response_count(state: &ExecutionState, agent: &str) -> usize {
    state
        .conversations
        .get(agent)
        .map(|msgs| msgs.iter().filter(|m| m.role == "assistant").count())
        .unwrap_or(0)
}

/// Number of delegations/questions/escalations `agent` has already issued
/// (as the `from` side of a `hier.trace` entry).
fn outgoing_count(state: &ExecutionState, agent: &str) -> usize {
    state
        .hier
        .trace
        .iter()
        .filter(|(from, _, _, _)| from == agent)
        .count()
}

/// Find the next unprocessed assistant response to act on: the first agent
/// (in `conversations`' `BTreeMap` iteration order, i.e. sorted by name —
/// deterministic) whose assistant-response count exceeds the number of
/// delegations/questions/escalations it has issued so far.
///
/// This derives "unprocessed" purely structurally, with no extra
/// bookkeeping field: a response is "processed" exactly when its
/// consequences (child `Delegated`/`AskedPeer`/`Escalated` events) have
/// been recorded, so counting the two and comparing is enough. Resolving
/// one pending agent per call (rather than trying to find a single global
/// "latest" across all agents) is what keeps this correct even when a
/// single `decide` batch invoked several children at once (e.g. two
/// sibling delegations): the generic engine loop (`run_event_sourced`)
/// re-invokes `decide` after every batch, so each newly-answered agent
/// becomes "pending" in turn across successive calls, always resolved in
/// the same (name-sorted) order — never dependent on wall-clock event
/// arrival order.
fn pending_response(state: &ExecutionState) -> Option<(&str, &str)> {
    state.conversations.iter().find_map(|(agent, msgs)| {
        if msgs.iter().filter(|m| m.role == "assistant").count() > outgoing_count(state, agent) {
            msgs.iter()
                .rev()
                .find(|m| m.role == "assistant")
                .map(|m| (agent.as_str(), m.content.as_str()))
        } else {
            None
        }
    })
}

/// Reconstruct a best-effort partial final answer from whatever each agent
/// has said so far — used when a guard (depth/iterations/budget) forces
/// early completion. Iterates `state.conversations` (a `BTreeMap`), so the
/// result is ordered by agent name and fully deterministic.
fn build_partial_content(state: &ExecutionState) -> String {
    state
        .conversations
        .iter()
        .filter_map(|(agent, msgs)| {
            msgs.iter()
                .rev()
                .find(|m| m.role == "assistant")
                .map(|m| format!("[{agent}] {}", m.content))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pure hierarchical [`Decider`]: given the current [`ExecutionState`],
/// decides the next batch of [`Action`]s (invoke an agent, emit a
/// bookkeeping event, warn-and-complete on limit breach, or complete the
/// run).
///
/// All fields are immutable inputs captured at construction time (the
/// per-run configuration). `decide` performs no I/O, blocks on nothing, and
/// reads no mutable state: every decision is a pure function of `state`,
/// which is what keeps replay of the event log deterministic.
#[derive(Debug, Clone)]
pub struct HierarchicalDecider {
    /// Name of the coordinator agent, invoked first (from `input`).
    pub coordinator: String,
    /// The original user input/task, given to the coordinator.
    pub input: String,
    /// Orchestration config (topology, teams, nested patterns, limits).
    pub config: OrchestrationConfig,
    /// All known agents by name, for model/tag lookups (routing).
    pub agents: BTreeMap<String, Agent>,
    /// Routing rules for `latest:auto` agents.
    pub routing_rules: RoutingRules,
    /// Max delegation depth before the run is force-completed.
    pub max_depth: u32,
    /// Max total agent invocations before the run is force-completed.
    pub max_iterations: u32,
    /// Optional total token budget (in + out) before the run is
    /// force-completed.
    pub token_budget: Option<u32>,
    /// Optional total cost budget (USD) before the run is force-completed.
    pub cost_limit: Option<f64>,
}

impl HierarchicalDecider {
    /// Construct a new `HierarchicalDecider`. All arguments become
    /// immutable fields read by `decide`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        coordinator: impl Into<String>,
        input: impl Into<String>,
        config: OrchestrationConfig,
        agents: BTreeMap<String, Agent>,
        routing_rules: RoutingRules,
        max_depth: u32,
        max_iterations: u32,
        token_budget: Option<u32>,
        cost_limit: Option<f64>,
    ) -> Self {
        Self {
            coordinator: coordinator.into(),
            input: input.into(),
            config,
            agents,
            routing_rules,
            max_depth,
            max_iterations,
            token_budget,
            cost_limit,
        }
    }

    /// Check depth/iteration/budget guards, returning the `Warned` code for
    /// whichever one has been breached (checked in this order; the first
    /// breach wins — `decide` doesn't need to report more than one).
    fn breached_limit(&self, state: &ExecutionState) -> Option<&'static str> {
        if current_depth(state) >= self.max_depth {
            return Some("max_depth");
        }
        if invocation_count(state) >= self.max_iterations as usize {
            return Some("max_iterations");
        }
        if let Some(budget) = self.token_budget
            && state.budget_tokens_in + state.budget_tokens_out >= u64::from(budget)
        {
            return Some("token_budget");
        }
        if let Some(limit) = self.cost_limit
            && state.budget_cost >= limit
        {
            return Some("cost_limit");
        }
        None
    }

    /// If `agent_name` is a known agent configured with the exact
    /// `"latest:auto"` model placeholder, resolve the tier for
    /// `routing_input` (using `crate::core::routing::route`, pure) and
    /// return the `ModelRouted` event to emit before invoking it. Concrete
    /// models, other `latest:*` placeholders, and unknown agents all return
    /// `None` — nothing to route.
    ///
    /// Note: this only decides *which tier* to record for the `ModelRouted`
    /// bookkeeping event. Resolving a tier to a concrete model string
    /// (`resolve_model_for_tier`, in `crate::linker::model_resolution`) is
    /// an effectful concern for whatever `EffectRunner` actually calls the
    /// provider (a later lot) — `Action::Invoke` carries no model field, so
    /// this pure `Decider` has no need to call it.
    fn model_routed_event(
        &self,
        agent_name: &str,
        routing_input: &str,
        state: &ExecutionState,
    ) -> Option<ExecutionEvent> {
        let agent = self.agents.get(agent_name)?;
        let raw_model = agent.metadata.model.as_deref().unwrap_or("default");
        if raw_model != "latest:auto" {
            return None;
        }
        let tokens_consumed = state.budget_tokens_in + state.budget_tokens_out;
        let budget = self.token_budget.filter(|&b| b > 0).map(|total| {
            let total = u64::from(total);
            BudgetState {
                remaining_ratio: total.saturating_sub(tokens_consumed) as f64 / total as f64,
            }
        });
        let (tier, reason) = route(
            routing_input,
            &agent.metadata.tags,
            budget,
            &self.routing_rules,
        );
        Some(ExecutionEvent::ModelRouted {
            agent: agent_name.to_string(),
            tier: format!("{tier:?}"),
            reason: format!("{reason:?}"),
        })
    }

    /// If `agent_name` leads a team configured with a nested sub-pattern
    /// (C9), return the `NestedStarted` event marking that boundary — the
    /// actual sub-run is executed by a later lot's effect; here we only
    /// record where it should start.
    fn nested_started_event(&self, agent_name: &str) -> Option<ExecutionEvent> {
        self.config
            .teams
            .iter()
            .find_map(|team| {
                if team.lead.as_deref() == Some(agent_name) {
                    team.pattern
                } else {
                    None
                }
            })
            .map(|pattern| ExecutionEvent::NestedStarted {
                team_lead: agent_name.to_string(),
                pattern: pattern.to_string(),
            })
    }

    /// Build the ordered action batch for invoking `agent_name` with
    /// `input`: an optional `ModelRouted` (if it routes `latest:auto`), an
    /// optional `NestedStarted` (if it's a nested-team lead), an optional
    /// bookkeeping event (`Delegated`/`AskedPeer`/`Escalated`, supplied by
    /// `plan_from_response` — absent for the initial coordinator kick-off),
    /// then the `Invoke` itself.
    fn invoke_actions(
        &self,
        agent_name: &str,
        input: &str,
        state: &ExecutionState,
        delegation_event: Option<ExecutionEvent>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(event) = self.model_routed_event(agent_name, input, state) {
            actions.push(Action::Emit(event));
        }
        if let Some(event) = self.nested_started_event(agent_name) {
            actions.push(Action::Emit(event));
        }
        if let Some(event) = delegation_event {
            actions.push(Action::Emit(event));
        }
        actions.push(Action::Invoke {
            agent: agent_name.to_string(),
            input: input.to_string(),
        });
        actions
    }
}

// NOTE (scope of this Task): unlike the legacy recursive `invoke_agent`
// (`crate::core::orchestration::hierarchical::invoke_agent`), which
// re-injects a subordinate's collected result into its *delegator*'s
// conversation for a synthesis turn before deciding anything further, this
// pure `Decider` treats every `PlannedStep::Complete` (from *any* agent's
// response, not just the coordinator's) as ending the whole run. Wiring the
// "feed subordinate results back to the delegator, then re-invoke it"
// synthesis loop is left to a later task in this lot — it requires
// tracking delegator/child result pairing, which isn't needed for the
// depth/iteration/budget guards or the nested-team boundary this task
// covers. The anti-loop guard (point 4) still applies correctly in the
// meantime: the coordinator can legitimately be invoked more than once via
// `Escalate` (subordinate → coordinator), which is enough to observe and
// test the "≥ 2 turns without FinalAnswer" cutoff.
impl Decider for HierarchicalDecider {
    fn decide(&self, state: &ExecutionState) -> Vec<Action> {
        // 1. Nothing has happened yet: kick off the coordinator with the
        // run's original input.
        if state.conversations.is_empty() {
            return self.invoke_actions(&self.coordinator, &self.input, state, None);
        }

        // 2. Guards: depth / iteration / budget caps, checked before
        // considering any further delegation.
        if let Some(code) = self.breached_limit(state) {
            return vec![
                Action::Emit(ExecutionEvent::Warned {
                    code: code.to_string(),
                }),
                Action::Complete {
                    content: build_partial_content(state),
                },
            ];
        }

        // 3. Process the next unprocessed assistant response, if any.
        let Some((agent, content)) = pending_response(state) else {
            return Vec::new();
        };
        let agent = agent.to_string();
        let content = content.to_string();

        // 4. Anti-loop: a coordinator that keeps delegating without ever
        // producing a `FinalAnswer` would never terminate on its own —
        // force completion using its latest narrative once it has had 2+
        // turns.
        if agent == self.coordinator && response_count(state, &agent) >= 2 {
            return vec![Action::Complete { content }];
        }

        let depth = depth_of(state, &agent);
        plan_from_response(&content, &agent, &self.config, depth)
            .into_iter()
            .flat_map(|step| match step {
                PlannedStep::Invoke { agent, task, event } => {
                    self.invoke_actions(&agent, &task, state, Some(event))
                }
                PlannedStep::Complete { content } => vec![Action::Complete { content }],
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::es::event::ExecutionEvent;
    use crate::core::orchestration::{OrchestrationPattern, TeamConfig};

    /// Tests for `HierarchicalDecider` (Task 3): pure decision function
    /// built on top of `plan_from_response` (Tasks 1-2). Named `decide` so
    /// `cargo test es::hierarchical::tests::decide` targets this module.
    mod decide {
        use super::*;
        use crate::core::agent::{Agent, AgentMetadata};
        use crate::core::orchestration::es::engine::{Action, Decider};
        use crate::core::orchestration::es::state::fold;
        use crate::core::orchestration::{NestedPattern, OrchestrationPattern, TeamConfig};
        use crate::core::routing::RoutingRules;
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        /// Minimal `Agent` for routing lookups. `model` controls whether
        /// this agent routes through `latest:auto` (pass `"latest:auto"`)
        /// or uses a concrete model string as-is.
        fn test_agent(name: &str, model: &str) -> Agent {
            Agent {
                name: name.to_string(),
                source: PathBuf::from(format!("{name}.md")),
                metadata: AgentMetadata {
                    provider: "anthropic".to_string(),
                    model: Some(model.to_string()),
                    command: None,
                    args: None,
                    temperature: 0.7,
                    max_tokens: None,
                    timeout: None,
                    tags: vec![],
                    stacks: vec![],
                    scope: vec![],
                    model_fallback: vec![],
                    cost_limit: None,
                    rate_limit: None,
                    context_window: None,
                    mode: None,
                    orchestration: None,
                    triggers: None,
                    ring_config: None,
                },
                system_prompt: "prompt".to_string(),
                instructions: None,
                output_format: None,
                pipeline: None,
                context: None,
            }
        }

        /// A coordinator ("dev-lead") plus one flat team (no team lead) of
        /// two peers — same shape as `plan_from_response`'s `sample_config`,
        /// so Superior/Subordinate/Peer classification is all reachable.
        fn base_config() -> OrchestrationConfig {
            OrchestrationConfig {
                enabled: true,
                pattern: OrchestrationPattern::Hierarchical,
                coordinator: Some("dev-lead".to_string()),
                teams: vec![TeamConfig {
                    lead: None,
                    agents: vec!["core-specialist".to_string(), "qa-specialist".to_string()],
                    ..Default::default()
                }],
                ..Default::default()
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn test_decider(
            coordinator: &str,
            agent_names: &[(&str, &str)],
            config: OrchestrationConfig,
            max_depth: u32,
            max_iterations: u32,
            token_budget: Option<u32>,
            cost_limit: Option<f64>,
        ) -> HierarchicalDecider {
            let mut agents = BTreeMap::new();
            for (name, model) in agent_names {
                agents.insert((*name).to_string(), test_agent(name, model));
            }
            HierarchicalDecider::new(
                coordinator.to_string(),
                "build X".to_string(),
                config,
                agents,
                RoutingRules::default(),
                max_depth,
                max_iterations,
                token_budget,
                cost_limit,
            )
        }

        fn run_started(agents: &[&str]) -> ExecutionEvent {
            ExecutionEvent::RunStarted {
                run_id: "r".into(),
                pattern: "hierarchical".into(),
                agents: agents.iter().map(|a| a.to_string()).collect(),
                input: "build X".into(),
                project: None,
            }
        }

        // (a) empty state → first Invoke(coordinator)
        #[test]
        fn empty_state_invokes_coordinator_first() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                    ("qa-specialist", "concrete-model"),
                ],
                base_config(),
                5,
                50,
                None,
                None,
            );
            let state = fold(&[run_started(&["dev-lead"])]);
            let actions = dec.decide(&state);
            assert!(
                actions
                    .iter()
                    .any(|a| matches!(a, Action::Invoke { agent, .. } if agent == "dev-lead"))
            );
        }

        // (b) coordinator's response is a FinalAnswer → Complete
        #[test]
        fn final_answer_completes() {
            let dec = test_decider(
                "dev-lead",
                &[("dev-lead", "concrete-model")],
                base_config(),
                5,
                50,
                None,
                None,
            );
            let events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "Voici la réponse finale.".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 1);
            assert!(
                matches!(&actions[0], Action::Complete { content } if content.contains("finale"))
            );
        }

        // (c) coordinator's response carries 2 delegations → 2 Invoke (+ Delegated), in order
        #[test]
        fn two_delegations_become_two_invokes_in_order() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                    ("qa-specialist", "concrete-model"),
                ],
                base_config(),
                5,
                50,
                None,
                None,
            );
            let events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-specialist: implémente X\n@qa-specialist: teste X".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);

            let invoke_targets: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::Invoke { agent, .. } => Some(agent.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(invoke_targets, vec!["core-specialist", "qa-specialist"]);

            let delegated_targets: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::Emit(ExecutionEvent::Delegated { to, .. }) => Some(to.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(delegated_targets, vec!["core-specialist", "qa-specialist"]);

            // Delegated must precede its matching Invoke, in order.
            let invoke_core_pos = actions
                .iter()
                .position(|a| matches!(a, Action::Invoke{agent,..} if agent == "core-specialist"))
                .unwrap();
            let delegated_core_pos = actions
                .iter()
                .position(|a| matches!(a, Action::Emit(ExecutionEvent::Delegated{to,..}) if to == "core-specialist"))
                .unwrap();
            assert!(delegated_core_pos < invoke_core_pos);
        }

        // (d) depth ≥ max_depth → Warned + Complete
        #[test]
        fn depth_guard_warns_and_completes() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                ],
                base_config(),
                1,
                50,
                None,
                None,
            );
            let events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-specialist: task".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-specialist".into(),
                    task: "task".into(),
                    depth: 1,
                },
                ExecutionEvent::AgentInvoked {
                    agent: "core-specialist".into(),
                    input: "task".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "core-specialist".into(),
                    content: "partial result".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 2);
            assert!(
                matches!(&actions[0], Action::Emit(ExecutionEvent::Warned{code}) if code == "max_depth")
            );
            assert!(
                matches!(&actions[1], Action::Complete{content} if content.contains("partial result"))
            );
        }

        // (e) agent to invoke leads a nested-pattern team → NestedStarted emitted
        #[test]
        fn nested_team_lead_emits_nested_started() {
            let config = OrchestrationConfig {
                enabled: true,
                pattern: OrchestrationPattern::Hierarchical,
                coordinator: Some("dev-lead".to_string()),
                teams: vec![TeamConfig {
                    lead: Some("core-lead".to_string()),
                    agents: vec!["core-a".to_string(), "core-b".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-lead", "concrete-model"),
                ],
                config,
                5,
                50,
                None,
                None,
            );
            let events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-lead: go".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert!(actions.iter().any(|a| matches!(
                a,
                Action::Emit(ExecutionEvent::NestedStarted { team_lead, pattern })
                if team_lead == "core-lead" && pattern == "blackboard"
            )));
            assert!(
                actions
                    .iter()
                    .any(|a| matches!(a, Action::Invoke{agent,..} if agent == "core-lead"))
            );
        }

        // Anti-loop: the coordinator can legitimately be invoked more than
        // once — not via subordinate-result re-injection (out of scope for
        // this pure Decider; see `NOTE` on `Decider::decide` at the top of
        // this file) but via an `Escalate` (subordinate → coordinator),
        // which produces a fresh `AgentInvoked` for the coordinator. If the
        // coordinator's *second* turn still isn't a `FinalAnswer`, the
        // anti-loop must force `Complete` with that turn's raw content
        // rather than keep delegating forever.
        #[test]
        fn coordinator_anti_loop_forces_complete_after_two_turns() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                ],
                base_config(),
                5,
                50,
                None,
                None,
            );
            let events = vec![
                run_started(&["dev-lead"]),
                // Turn 1: coordinator delegates to core-specialist.
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-specialist: task 1".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-specialist".into(),
                    task: "task 1".into(),
                    depth: 1,
                },
                // core-specialist escalates back to the coordinator instead
                // of answering — a legitimate second invocation trigger.
                ExecutionEvent::AgentInvoked {
                    agent: "core-specialist".into(),
                    input: "task 1".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "core-specialist".into(),
                    content: "@dev-lead: je bloque, besoin d'arbitrage".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
                ExecutionEvent::Escalated {
                    from: "core-specialist".into(),
                    to: "dev-lead".into(),
                    message: "je bloque, besoin d'arbitrage".into(),
                },
                // Turn 2: coordinator responds again, still not a FinalAnswer.
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "je bloque, besoin d'arbitrage".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-specialist: task 2".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 1);
            assert!(matches!(&actions[0], Action::Complete{content} if content.contains("task 2")));
        }
    }

    /// A realistic hierarchical config: a coordinator plus one team with
    /// several members, so both Superior/Subordinate (coordinator↔team) and
    /// Peer (intra-team) relationships are actually reachable.
    fn sample_config() -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["core-specialist".to_string(), "qa-specialist".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn final_answer_becomes_complete() {
        let config = sample_config();
        let steps = plan_from_response("Voici la réponse finale.", "dev-lead", &config, 0);
        assert_eq!(steps.len(), 1);
        matches!(&steps[0], PlannedStep::Complete { content } if content.contains("finale"))
            .then_some(())
            .expect("expected Complete");
    }

    #[test]
    fn delegation_lines_become_invokes_with_events() {
        let config = sample_config();
        let resp = "@core-specialist: implémente X\n@qa-specialist: teste X";
        let steps = plan_from_response(resp, "dev-lead", &config, 0);
        assert_eq!(steps.len(), 2);
        // ordre = ordre des lignes (déterminisme)
        match &steps[0] {
            PlannedStep::Invoke { agent, event, .. } => {
                assert_eq!(agent, "core-specialist");
                assert!(
                    matches!(event, ExecutionEvent::Delegated { to, depth, .. } if to == "core-specialist" && *depth == 1)
                );
            }
            _ => panic!("expected Invoke"),
        }
        match &steps[1] {
            PlannedStep::Invoke { agent, .. } => assert_eq!(agent, "qa-specialist"),
            _ => panic!("expected Invoke"),
        }
    }

    #[test]
    fn peer_line_within_same_team_becomes_asked_peer() {
        // core-specialist and qa-specialist are teammates (no team lead),
        // neither is the coordinator — classify_relationship must resolve
        // this as Peer, not fall back to Unknown/Delegate.
        let config = sample_config();
        let resp = "@qa-specialist: peux-tu vérifier ce point ?";
        let steps = plan_from_response(resp, "core-specialist", &config, 0);
        assert_eq!(steps.len(), 1);
        match &steps[0] {
            PlannedStep::Invoke { agent, event, .. } => {
                assert_eq!(agent, "qa-specialist");
                assert!(
                    matches!(
                        event,
                        ExecutionEvent::AskedPeer { from, to, .. }
                        if from == "core-specialist" && to == "qa-specialist"
                    ),
                    "expected AskedPeer for intra-team peer contact, got {event:?}"
                );
            }
            _ => panic!("expected Invoke"),
        }
    }

    #[test]
    fn escalation_line_becomes_invoke_with_escalated_event() {
        // core-specialist is a subordinate of the coordinator "dev-lead"
        // (no team lead in between) — classify_relationship must resolve
        // this as Subordinate, so parse_delegations yields Escalate.
        let config = sample_config();
        let resp = "@dev-lead: je bloque sur X, besoin d'arbitrage";
        let steps = plan_from_response(resp, "core-specialist", &config, 0);
        assert_eq!(steps.len(), 1);
        match &steps[0] {
            PlannedStep::Invoke { agent, task, event } => {
                assert_eq!(agent, "dev-lead");
                assert_eq!(task, "je bloque sur X, besoin d'arbitrage");
                assert!(
                    matches!(
                        event,
                        ExecutionEvent::Escalated { from, to, message }
                        if from == "core-specialist"
                            && to == "dev-lead"
                            && message == "je bloque sur X, besoin d'arbitrage"
                    ),
                    "expected Escalated for subordinate→coordinator contact, got {event:?}"
                );
            }
            _ => panic!("expected Invoke"),
        }
    }
}
