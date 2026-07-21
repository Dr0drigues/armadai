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
use crate::core::orchestration::protocol::{
    DelegationAction, extract_narrative, parse_delegations,
};
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

/// The most recent `assistant` response produced by `agent`, if any.
///
/// At `decide` time each conversation alternates strictly `user`,
/// `assistant`, … (the engine loop records `AgentInvoked` then, within the
/// same batch, the resulting `AgentObserved`), so this is `agent`'s latest
/// turn — a delegation round, a synthesis, or a final answer.
fn latest_response<'a>(state: &'a ExecutionState, agent: &str) -> Option<&'a str> {
    state
        .conversations
        .get(agent)?
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.as_str())
}

/// Whether `response` (from `sender`) carries no delegation directive at
/// all — i.e. `parse_delegations` collapses it to a single `FinalAnswer`.
/// A settled leaf answer and a converged coordinator synthesis both satisfy
/// this.
fn is_final_answer(response: &str, sender: &str, config: &OrchestrationConfig) -> bool {
    matches!(
        parse_delegations(response, sender, config).as_slice(),
        [DelegationAction::FinalAnswer { .. }]
    )
}

/// The delegation *targets* named in `response` (from `sender`), in line
/// order — every `Delegate`/`AskPeer`/`Escalate` directive contributes its
/// target; a `FinalAnswer` contributes nothing. Order is deterministic
/// (line order, inherited from `parse_delegations`), so the synthesis
/// re-injection format is stable across replays.
fn delegation_targets(response: &str, sender: &str, config: &OrchestrationConfig) -> Vec<String> {
    parse_delegations(response, sender, config)
        .into_iter()
        .filter_map(|action| match action {
            DelegationAction::Delegate { target, .. }
            | DelegationAction::AskPeer { target, .. }
            | DelegationAction::Escalate { target, .. } => Some(target),
            DelegationAction::FinalAnswer { .. } => None,
        })
        .collect()
}

/// Total number of delegation directives `agent` has emitted across *all*
/// of its responses so far. Each dispatched directive records exactly one
/// `hier.trace` entry (`from == agent`), so comparing this against
/// [`outgoing_count`] tells us whether the latest response has been
/// dispatched yet (see [`pending_delegation_lines`]).
fn total_delegation_lines(
    state: &ExecutionState,
    config: &OrchestrationConfig,
    agent: &str,
) -> usize {
    state
        .conversations
        .get(agent)
        .map(|msgs| {
            msgs.iter()
                .filter(|m| m.role == "assistant")
                .map(|m| delegation_targets(&m.content, agent, config).len())
                .sum()
        })
        .unwrap_or(0)
}

/// Number of `agent`'s delegation directives not yet dispatched: the
/// directives it has emitted (`total_delegation_lines`) minus those already
/// recorded in the trace (`outgoing_count`). Dispatch is atomic per
/// response (a whole batch of `Emit(Delegated)` + `Invoke` at once), so this
/// is either `0` (all dispatched) or exactly the size of the latest
/// response's directive set (that response awaits dispatch).
fn pending_delegation_lines(
    state: &ExecutionState,
    config: &OrchestrationConfig,
    agent: &str,
) -> usize {
    total_delegation_lines(state, config, agent).saturating_sub(outgoing_count(state, agent))
}

/// Whether `agent` has delivered a final result to its delegator: it has
/// responded and its *latest* response is a `FinalAnswer`. An agent that
/// delegated (its latest response still carries directives) is *not*
/// settled — it is either awaiting its own children or awaiting its own
/// synthesis — which is exactly what makes settlement propagate bottom-up.
fn is_settled(state: &ExecutionState, config: &OrchestrationConfig, agent: &str) -> bool {
    latest_response(state, agent).is_some_and(|r| is_final_answer(r, agent, config))
}

/// How many times `agent` has been re-invoked with a synthesis re-injection
/// (a `user` message carrying formatted child results). Used purely for the
/// coordinator anti-loop guard.
fn synthesis_count(state: &ExecutionState, agent: &str) -> usize {
    state
        .conversations
        .get(agent)
        .map(|msgs| {
            msgs.iter()
                .filter(|m| m.role == "user" && m.content.contains("[Result from @"))
                .count()
        })
        .unwrap_or(0)
}

/// Format collected child results for re-injection into a delegator's
/// conversation. Pure, local replica of the legacy
/// `core::orchestration::hierarchical::format_results` — kept separate on
/// purpose (strict coexistence: the legacy engine is not modified nor
/// imported from here). Each result is wrapped in
/// `[Result from @NAME] … [End result from @NAME]`, in the order given.
fn format_results(results: &[(String, String)]) -> String {
    let mut out = String::new();
    for (agent, result) in results {
        out.push_str(&format!(
            "[Result from @{agent}]\n{result}\n[End result from @{agent}]\n\n"
        ));
    }
    out
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

// ── Synthesis loop (Task 3 fix) ──────────────────────────────────
//
// The central behaviour of the hierarchical pattern is *coordinator
// synthesis*: when an agent X delegates to children [A, B, …], the children's
// results are re-injected into X's own conversation as a single `user` turn
// (`[Result from @A] … [End result from @A]`, one block per child), and X is
// re-invoked to synthesize them. Only the *root coordinator*'s `FinalAnswer`
// (produced after its synthesis, or as a direct answer with no delegation)
// terminates the whole run — a subordinate's `FinalAnswer` merely settles it
// so its delegator can synthesize.
//
// This is reconstructed purely from `ExecutionState`, with no extra
// bookkeeping field:
//   * `hier.trace` (`from == X`) records what X dispatched; comparing its
//     size (`outgoing_count`) against the directives parsed from X's
//     responses (`total_delegation_lines`) yields `pending_delegation_lines`
//     — non-zero means X's latest response still awaits dispatch.
//   * an agent is "settled" when its latest response is a `FinalAnswer`
//     (`is_settled`); settlement therefore propagates bottom-up.
//   * X is "awaiting synthesis" when its latest response carries directives,
//     all of them are dispatched, and every child target is settled — at
//     which point X has not yet been re-invoked (its latest turn is still the
//     delegation), so the re-injection has not happened yet.
impl HierarchicalDecider {
    /// First agent (name-sorted, via `BTreeMap` iteration) whose latest
    /// response carries delegation directives that have not been dispatched
    /// yet. Resolving one per call keeps sibling delegations deterministic:
    /// the generic loop re-invokes `decide` after every batch.
    fn agent_needing_dispatch(&self, state: &ExecutionState) -> Option<String> {
        state
            .conversations
            .keys()
            .find(|agent| pending_delegation_lines(state, &self.config, agent) > 0)
            .cloned()
    }

    /// Whether `agent`'s latest (already-dispatched) delegation round is
    /// ready to be synthesized: it carried directives, all are dispatched,
    /// and every child target has settled.
    fn is_awaiting_synthesis(&self, state: &ExecutionState, agent: &str) -> bool {
        let Some(latest) = latest_response(state, agent) else {
            return false;
        };
        if is_final_answer(latest, agent, &self.config) {
            return false;
        }
        if pending_delegation_lines(state, &self.config, agent) > 0 {
            return false;
        }
        let children = delegation_targets(latest, agent, &self.config);
        !children.is_empty()
            && children
                .iter()
                .all(|child| is_settled(state, &self.config, child))
    }

    /// First agent (name-sorted) awaiting synthesis. Because a delegator is
    /// only "awaiting" once *all* its children have settled (and a child that
    /// itself delegated is not settled until it synthesizes down to a
    /// `FinalAnswer`), this resolves the delegation tree bottom-up.
    fn awaiting_synthesis_agent(&self, state: &ExecutionState) -> Option<String> {
        state
            .conversations
            .keys()
            .find(|agent| self.is_awaiting_synthesis(state, agent))
            .cloned()
    }

    /// Dispatch `agent`'s latest (undispatched) delegation response: map each
    /// directive to its `Emit(Delegated/AskedPeer/Escalated)` + `Invoke`
    /// batch via `plan_from_response` (Tasks 1-2). A `FinalAnswer` step
    /// cannot occur here (only agents with pending directives reach this),
    /// and is dropped defensively if it somehow does.
    fn dispatch_actions(&self, agent: &str, state: &ExecutionState) -> Vec<Action> {
        let Some(latest) = latest_response(state, agent) else {
            return Vec::new();
        };
        let latest = latest.to_string();
        let depth = depth_of(state, agent);
        plan_from_response(&latest, agent, &self.config, depth)
            .into_iter()
            .flat_map(|step| match step {
                PlannedStep::Invoke { agent, task, event } => {
                    self.invoke_actions(&agent, &task, state, Some(event))
                }
                PlannedStep::Complete { .. } => Vec::new(),
            })
            .collect()
    }

    /// Build the synthesis re-injection for `agent`: collect each child's
    /// latest response (in the order the children were addressed), format
    /// them as `[Result from @child] …` blocks, and re-invoke `agent` with
    /// that as its next `user` turn. No `Delegated` event is emitted (this is
    /// a synthesis turn, not a new delegation); an optional `ModelRouted`
    /// precedes the `Invoke` for `latest:auto` agents.
    fn synthesis_actions(&self, state: &ExecutionState, agent: &str) -> Vec<Action> {
        let Some(latest) = latest_response(state, agent) else {
            return Vec::new();
        };
        let results: Vec<(String, String)> = delegation_targets(latest, agent, &self.config)
            .into_iter()
            .map(|child| {
                let result = latest_response(state, &child)
                    .unwrap_or_default()
                    .to_string();
                (child, result)
            })
            .collect();
        let message = format_results(&results);
        let mut actions = Vec::new();
        if let Some(event) = self.model_routed_event(agent, &message, state) {
            actions.push(Action::Emit(event));
        }
        actions.push(Action::Invoke {
            agent: agent.to_string(),
            input: message,
        });
        actions
    }
}

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

        // 3. Root-coordinator termination / anti-loop. Only the coordinator's
        // own answer can end the run.
        if let Some(coord_latest) = latest_response(state, &self.coordinator) {
            // A `FinalAnswer` from the coordinator — produced after its
            // synthesis, or as a direct answer with no delegation at all —
            // is the run's final result.
            if is_final_answer(coord_latest, &self.coordinator, &self.config) {
                return vec![Action::Complete {
                    content: extract_narrative(coord_latest),
                }];
            }
            // Anti-loop: the coordinator has synthesized twice and still not
            // converged to its own `FinalAnswer`. Force completion with its
            // latest narrative rather than spin forever (the socle's
            // MAX_ITERATIONS remains the global net).
            if synthesis_count(state, &self.coordinator) >= 2 {
                return vec![Action::Complete {
                    content: extract_narrative(coord_latest),
                }];
            }
        }

        // 4. Dispatch any undispatched delegation round (the coordinator's
        // or a subordinate's), spawning its children.
        if let Some(agent) = self.agent_needing_dispatch(state) {
            return self.dispatch_actions(&agent, state);
        }

        // 5. Synthesis: an agent whose children have all settled is
        // re-invoked with their re-injected results (resolved bottom-up).
        if let Some(agent) = self.awaiting_synthesis_agent(state) {
            return self.synthesis_actions(state, &agent);
        }

        // 6. Nothing actionable (e.g. a subordinate settled but its delegator
        // is still waiting on a sibling): let the loop idle.
        Vec::new()
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

        /// Helper: a settled subordinate turn — invoked with `task`, then
        /// observes `answer` (a `FinalAnswer`, no delegation directives).
        fn subordinate_turn(agent: &str, task: &str, answer: &str) -> Vec<ExecutionEvent> {
            vec![
                ExecutionEvent::AgentInvoked {
                    agent: agent.into(),
                    input: task.into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: agent.into(),
                    content: answer.into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ]
        }

        // Test 1: coordinator delegated to A and B, both answered (each a
        // FinalAnswer). `decide` must re-inject *both* results into the
        // coordinator and re-invoke it for synthesis — NOT `Complete` on a
        // subordinate's answer.
        #[test]
        fn synthesis_after_all_children_respond() {
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
            let mut events = vec![
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
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-specialist".into(),
                    task: "implémente X".into(),
                    depth: 1,
                },
            ];
            events.extend(subordinate_turn(
                "core-specialist",
                "implémente X",
                "X est implémenté.",
            ));
            events.push(ExecutionEvent::Delegated {
                from: "dev-lead".into(),
                to: "qa-specialist".into(),
                task: "teste X".into(),
                depth: 1,
            });
            events.extend(subordinate_turn(
                "qa-specialist",
                "teste X",
                "X est testé, RAS.",
            ));

            let state = fold(&events);
            let actions = dec.decide(&state);

            // No Complete: a subordinate FinalAnswer must not end the run.
            assert!(!actions.iter().any(|a| matches!(a, Action::Complete { .. })));
            // Exactly the re-injection: Invoke the coordinator with both
            // children's results formatted in.
            let reinjection = actions.iter().find_map(|a| match a {
                Action::Invoke { agent, input } if agent == "dev-lead" => Some(input.as_str()),
                _ => None,
            });
            let input = reinjection.expect("expected a synthesis Invoke to the coordinator");
            assert!(
                input.contains("[Result from @core-specialist]")
                    && input.contains("X est implémenté."),
                "core-specialist result must be re-injected, got: {input}"
            );
            assert!(
                input.contains("[Result from @qa-specialist]")
                    && input.contains("X est testé, RAS."),
                "qa-specialist result must be re-injected, got: {input}"
            );
        }

        // Test 2a: a subordinate answered (FinalAnswer) but a sibling has not
        // yet responded → the run must NOT complete, and synthesis must NOT
        // fire yet (the coordinator waits for the missing sibling).
        #[test]
        fn no_completion_while_a_sibling_is_still_pending() {
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
            let mut events = vec![
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
                // Both delegations dispatched…
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-specialist".into(),
                    task: "implémente X".into(),
                    depth: 1,
                },
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "qa-specialist".into(),
                    task: "teste X".into(),
                    depth: 1,
                },
            ];
            // …but only core-specialist has answered; qa-specialist is still
            // in flight (no observation).
            events.extend(subordinate_turn(
                "core-specialist",
                "implémente X",
                "X est implémenté.",
            ));

            let state = fold(&events);
            let actions = dec.decide(&state);
            assert!(
                actions.is_empty(),
                "must idle until the pending sibling answers, got: {actions:?}"
            );
        }

        // Test 2b: the coordinator produced its own FinalAnswer *after*
        // synthesis → the run completes with that answer.
        #[test]
        fn completes_on_coordinator_final_answer_after_synthesis() {
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
            let mut events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-specialist: implémente X".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-specialist".into(),
                    task: "implémente X".into(),
                    depth: 1,
                },
            ];
            events.extend(subordinate_turn(
                "core-specialist",
                "implémente X",
                "X est implémenté.",
            ));
            // Coordinator re-invoked with the re-injected result, then
            // synthesizes a clean FinalAnswer.
            events.push(ExecutionEvent::AgentInvoked {
                agent: "dev-lead".into(),
                input: format_results(&[(
                    "core-specialist".to_string(),
                    "X est implémenté.".to_string(),
                )]),
            });
            events.push(ExecutionEvent::AgentObserved {
                agent: "dev-lead".into(),
                content: "Synthèse finale : la fonctionnalité est prête.".into(),
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.0,
                model: "m".into(),
            });

            let state = fold(&events);
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 1);
            assert!(
                matches!(&actions[0], Action::Complete { content } if content.contains("prête"))
            );
        }

        // Anti-loop: the coordinator has synthesized twice and still keeps
        // delegating instead of producing its own FinalAnswer → force
        // completion with its latest narrative.
        #[test]
        fn coordinator_anti_loop_forces_complete_after_two_syntheses() {
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
            let mut events = vec![
                run_started(&["dev-lead"]),
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
            ];
            events.extend(subordinate_turn("core-specialist", "task 1", "result 1"));
            // Synthesis #1: coordinator still delegates instead of answering.
            events.push(ExecutionEvent::AgentInvoked {
                agent: "dev-lead".into(),
                input: format_results(&[("core-specialist".to_string(), "result 1".to_string())]),
            });
            events.push(ExecutionEvent::AgentObserved {
                agent: "dev-lead".into(),
                content: "Encore du travail.\n@core-specialist: task 2".into(),
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.0,
                model: "m".into(),
            });
            events.push(ExecutionEvent::Delegated {
                from: "dev-lead".into(),
                to: "core-specialist".into(),
                task: "task 2".into(),
                depth: 1,
            });
            events.extend(subordinate_turn("core-specialist", "task 2", "result 2"));
            // Synthesis #2: still delegating — the anti-loop must now fire.
            events.push(ExecutionEvent::AgentInvoked {
                agent: "dev-lead".into(),
                input: format_results(&[("core-specialist".to_string(), "result 2".to_string())]),
            });
            events.push(ExecutionEvent::AgentObserved {
                agent: "dev-lead".into(),
                content: "Toujours en cours.\n@core-specialist: task 3".into(),
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.0,
                model: "m".into(),
            });

            let state = fold(&events);
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 1);
            assert!(
                matches!(&actions[0], Action::Complete { content } if content.contains("Toujours en cours"))
            );
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
