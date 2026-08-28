//! Pure transducer: LLM response → planned intentions (hierarchical pattern).
//!
//! `plan_from_response` reuses the existing (pure) delegation parser in
//! `crate::orchestration::protocol` and maps each parsed
//! `DelegationAction` onto a `PlannedStep` carrying the `ExecutionEvent` that
//! should be appended for it. No I/O, no async — this is the decision half
//! of the event-sourced hierarchical engine.
//!
//! `HierarchicalEffectRunner` (Task 4) is the other half: the sole
//! async/impure component, executing the actual LLM call behind
//! `Action::Invoke` and turning the provider's response into the
//! `AgentObserved` event the pure `Decider`/loop expect.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;

use super::blackboard::{build_board_result, run_blackboard_es};
use super::engine::{Action, Decider, EffectRunner, InvokeSpec, run_event_sourced};
use super::event::ExecutionEvent;
use super::log::{EventLog, InMemoryLog};
use super::ring::{resolve_votes, run_ring_es, vote_weights_from_agents};
use super::state::ExecutionState;
use crate::agent::Agent;
#[cfg(test)]
use crate::model_resolution::fallback_model_for_tier;
use crate::model_resolution::{ModelTier, resolve_routed_tier, resolve_tier_placeholder};
use crate::orchestration::blackboard::BlackboardConfig;
use crate::orchestration::context_injection::{AgentInfo, build_orchestration_prompt};
use crate::orchestration::protocol::{DelegationAction, extract_narrative, parse_delegations};
use crate::orchestration::ring::RingConfig;
use crate::orchestration::{NestedPattern, OrchestrationConfig, TeamConfig};
use crate::provider::{ChatMessage, CompletionRequest, Provider};
use crate::routing::{BudgetState, RoutingRules, route};

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

/// Turn cap: the maximum number of `assistant` responses any single agent may
/// produce before `decide` force-completes the run.
///
/// This is a *second*, orthogonal anti-loop guard sitting alongside the
/// coordinator's `synthesis_count` guard. `synthesis_count` only counts
/// synthesis re-injections (`user` turns carrying `[Result from @…]`), so it is
/// blind to an **escalation ping-pong**: a subordinate `S` escalates to the
/// coordinator, the coordinator re-delegates `@S`, `S` re-escalates, … Each
/// re-invocation here is a *raw* escalation/delegation message, never a
/// `format_results` re-injection, so `synthesis_count` stays `0` and the loop
/// would only ever be broken by the socle's `MAX_ITERATIONS` (~500 LLM calls).
/// Counting an agent's own turns catches that ping-pong far earlier.
///
/// `4` is chosen with a comfortable margin above the normal hierarchical flow:
/// an agent delegates then synthesizes (2 turns), and even a coordinator that
/// needs one synthesis retry tops out at 3 turns before the
/// `synthesis_count >= 2` guard fires. A 4th turn therefore reliably signals a
/// loop the other guards did not catch, without risking a false positive on a
/// healthy run.
const MAX_AGENT_TURNS: usize = 4;

/// Number of `assistant` turns `agent` has produced so far — one per
/// `AgentObserved` folded into its conversation. Used by the turn-cap
/// anti-loop guard (see [`MAX_AGENT_TURNS`]).
fn assistant_turn_count(state: &ExecutionState, agent: &str) -> usize {
    state
        .conversations
        .get(agent)
        .map(|msgs| msgs.iter().filter(|m| m.role == "assistant").count())
        .unwrap_or(0)
}

/// Whether the run is legitimately waiting on an in-flight invocation: some
/// agent that has been delegated/asked/escalated to (`to` side of a
/// `hier.trace` entry) has not produced any response yet. Distinguishes a
/// transient "waiting for a reply" state (idle, no actions) from a genuinely
/// stuck run (see `decide`'s no-progress safety net).
fn awaiting_in_flight(state: &ExecutionState) -> bool {
    state
        .hier
        .trace
        .iter()
        .any(|(_, to, _, _)| latest_response(state, to).is_none())
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
    ///
    /// The `max_depth` branch here is the *reactive* net: it catches a depth
    /// already recorded in `hier.trace` on some earlier round (e.g. the
    /// turn-cap/anti-loop paths below, or a hand-built state in a test).
    /// The *proactive* guard-at-source — refusing to dispatch a delegation
    /// that would create a too-deep child in the first place — lives in
    /// `dispatch_actions` (OH1 Lot 4 Task 3, reconciliation A) and is what
    /// fires on the normal delegation path, faithfully mirroring legacy's
    /// `invoke_agent` entry check.
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
    /// `routing_input` (using `crate::routing::route`, pure) and
    /// return the `ModelRouted` event to emit before invoking it. Concrete
    /// models, other `latest:*` placeholders, and unknown agents all return
    /// `None` — nothing to route.
    ///
    /// Note: this only decides *which tier* to record for the `ModelRouted`
    /// bookkeeping event. Resolving a tier to a concrete model string
    /// (`resolve_model_for_tier`, in `crate::model_resolution`) is
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
    ///
    /// NOTE (OH1 Lot 3): this only *marks the boundary* — `decide` stays
    /// pure/sync and never launches the nested team's sub-run itself. The
    /// `Action::Invoke` that follows this `Emit` in `invoke_actions` still
    /// invokes `agent_name` (the team lead) as an ordinary agent in the
    /// *parent* run, exactly like any other hierarchical agent; today's
    /// `HierarchicalEffectRunner::run_invoke` has no special case for it.
    ///
    /// The Lot 3 hook: `EffectRunner::run_invoke` (or a wrapper around it)
    /// will detect a just-emitted `NestedStarted { team_lead, pattern }` and,
    /// instead of (or in addition to) the plain LLM call, drive a *separate*
    /// event-sourced sub-run for that team — option A from the plan:
    ///   - mint a child `run_id` carrying `parent_run_id = state.run_id`,
    ///   - reuse the existing blackboard/ring `Decider`/`EffectRunner` pairs
    ///     (`BlackboardDecider`/`RingDecider` and their effect runners) scoped
    ///     to `team.agents` with `team.lead` as arbiter, seeded with the task
    ///     handed to `team_lead`,
    ///   - fold that sub-run to completion via `run_event_sourced` on its own
    ///     log, then surface its outcome (+ aggregated tokens/cost) back into
    ///     *this* run as an `AgentObserved`-like event for `team_lead` (so
    ///     the parent's synthesis logic in this file needs no changes), and
    ///   - finally emit `ExecutionEvent::NestedEnded { team_lead }` to close
    ///     the boundary this `NestedStarted` opened.
    ///
    /// Nothing here — `decide`, `nested_started_event`, `invoke_actions` — is
    /// expected to change for that; the hook lives entirely on the impure
    /// `EffectRunner` side.
    fn nested_started_event(&self, agent_name: &str) -> Option<ExecutionEvent> {
        nested_team_for(&self.config, agent_name).map(|(_, pattern)| {
            ExecutionEvent::NestedStarted {
                team_lead: agent_name.to_string(),
                pattern: pattern.to_string(),
            }
        })
    }

    /// First team lead (in `state.open_nested`'s `BTreeSet` order) whose
    /// nested C9 boundary is still open *and* whose sub-run outcome has
    /// already been surfaced as an `AgentObserved` (`assistant_turn_count >=
    /// 1`). Returning it means `decide` should emit a deferred
    /// `NestedEnded { team_lead }` to close that boundary.
    ///
    /// Pure and deterministic: reads only the projected `state.open_nested`
    /// set (maintained by `es::state::apply` from `NestedStarted`/`NestedEnded`
    /// events) plus the lead's observed-turn count — no log scan, no I/O. The
    /// "observed" guard mirrors the legacy `run_nested_team`, which emits
    /// `NestedEnd` only *after* the sub-run has produced its outcome; on the
    /// production path the lead's `Invoke` (and the `AgentObserved` the nested
    /// `run_invoke` returns for it) is applied in the very same action batch as
    /// the opening `NestedStarted`, so by the next `decide` round the lead is
    /// always already observed — the guard only matters for hand-built partial
    /// states in tests.
    fn pending_nested_ended(&self, state: &ExecutionState) -> Option<String> {
        state
            .open_nested
            .iter()
            .find(|lead| assistant_turn_count(state, lead) >= 1)
            .cloned()
    }

    /// Build the ordered action batch for invoking `agent_name` with
    /// `input`: an optional `ModelRouted` (if it routes `latest:auto`), an
    /// optional bookkeeping event (`Delegated`/`AskedPeer`/`Escalated`,
    /// supplied by `plan_from_response` — absent for the initial coordinator
    /// kick-off), an optional `NestedStarted` (if it's a nested-team lead),
    /// then the `Invoke` itself. The bookkeeping event precedes
    /// `NestedStarted` so that a delegation into a nested team is observed
    /// (`delegate`) before the team boundary opens (`nested_start`), matching
    /// the legacy engine's emission order.
    /// The ordered bookkeeping `Emit(...)` actions that precede an invocation:
    /// an optional `ModelRouted` (if it routes `latest:auto`), an optional
    /// delegation event (`Delegated`/`AskedPeer`/`Escalated`, from
    /// `plan_from_response`), then an optional `NestedStarted` (nested-team
    /// lead). Split out of `invoke_actions` so the parallel dispatch path can
    /// record every child's emits sequentially (in Vec order) before a single
    /// `InvokeParallel`, while the sequential callers keep `Emit + Invoke`.
    fn invoke_emit_actions(
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
        if let Some(event) = delegation_event {
            actions.push(Action::Emit(event));
        }
        if let Some(event) = self.nested_started_event(agent_name) {
            actions.push(Action::Emit(event));
        }
        actions
    }

    /// The full sequential batch for one invocation: the bookkeeping emits
    /// (`invoke_emit_actions`) followed by the `Invoke` itself. Used by the
    /// coordinator kick-off and synthesis re-invokes (single-agent paths).
    fn invoke_actions(
        &self,
        agent_name: &str,
        input: &str,
        state: &ExecutionState,
        delegation_event: Option<ExecutionEvent>,
    ) -> Vec<Action> {
        let mut actions = self.invoke_emit_actions(agent_name, input, state, delegation_event);
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
    ///
    /// **max_depth guard-at-source** (OH1 Lot 4 Task 3, reconciliation A):
    /// every target this batch would invoke — `Delegate`, `AskPeer`, and
    /// `Escalate` alike — runs at `depth + 1` in the legacy engine (see
    /// `invoke_agent(ctx, state, target, task, depth + 1, sender)` in
    /// `crate::orchestration::hierarchical`), and legacy's
    /// `invoke_agent` checks `depth >= max_depth` as the *very first* thing
    /// it does — before recording anything in `trace`/`conversations` and
    /// before ever calling the provider. So a target at exactly `max_depth`
    /// is never invoked there.
    ///
    /// The ES engine used to only check this one `decide` round later
    /// (`breached_limit`'s `current_depth(state) >= self.max_depth`, above),
    /// by which point the too-deep child's `Delegated` + `Invoke` had
    /// already been emitted and the child had already run — one level
    /// deeper than legacy. Checking `depth + 1` here, before dispatch, closes
    /// that gap: the run halts (`Warned{max_depth}` + `Complete`, exactly
    /// the same graceful-halt shape `breached_limit` already produces)
    /// without invoking the over-depth target at all. All directives parsed
    /// from a single response share the same `depth + 1` (they're all
    /// dispatched from the same `agent` in the same round), so this either
    /// halts the whole batch or none of it — never a partial dispatch.
    fn dispatch_actions(&self, agent: &str, state: &ExecutionState) -> Vec<Action> {
        let Some(latest) = latest_response(state, agent) else {
            return Vec::new();
        };
        let latest = latest.to_string();
        let depth = depth_of(state, agent);
        if depth + 1 >= self.max_depth {
            return vec![
                Action::Emit(ExecutionEvent::Warned {
                    code: "max_depth".to_string(),
                }),
                Action::Complete {
                    content: build_partial_content(state),
                },
            ];
        }
        // Collect the invoke-steps (a `Complete` cannot occur here — only
        // agents with pending directives reach dispatch — and is dropped
        // defensively).
        let invoke_steps: Vec<(String, String, ExecutionEvent)> =
            plan_from_response(&latest, agent, &self.config, depth)
                .into_iter()
                .filter_map(|step| match step {
                    PlannedStep::Invoke { agent, task, event } => Some((agent, task, event)),
                    PlannedStep::Complete { .. } => None,
                })
                .collect();

        // 0 or 1 child: keep the sequential `Emit(s) + Invoke` shape (no
        // concurrency needed, byte-identical to before this lot).
        if invoke_steps.len() <= 1 {
            return invoke_steps
                .into_iter()
                .flat_map(|(child, task, event)| {
                    self.invoke_actions(&child, &task, state, Some(event))
                })
                .collect();
        }

        // ≥2 children: record every child's bookkeeping emits in line order,
        // then a single `InvokeParallel` whose batch is in line order. The
        // socle records `AgentInvoked ×N` then outcomes in batch order, so
        // replay stays deterministic.
        let mut actions = Vec::new();
        let mut batch = Vec::new();
        for (child, task, event) in invoke_steps {
            actions.extend(self.invoke_emit_actions(&child, &task, state, Some(event)));
            batch.push(InvokeSpec {
                agent: child,
                input: task,
            });
        }
        actions.push(Action::InvokeParallel {
            batch,
            max_concurrency: self.config.max_concurrency(),
        });
        actions
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

        // 1b. C9 nested boundary close-off: a team lead's sub-run has been
        // launched (`NestedStarted`) and its outcome observed
        // (`AgentObserved`, produced by the nested `run_invoke` short-circuit),
        // but the boundary is still open. Emit the deferred `NestedEnded` to
        // balance it — as its own single-action batch, so the next `decide`
        // round (with `open_nested` now empty) proceeds to the parent's normal
        // synthesis/arbitration of the lead's outcome. Checked ahead of the
        // guards so the boundary always closes, matching the legacy
        // `run_nested_team`, which emits `NestedEnd` regardless of outcome.
        if let Some(team_lead) = self.pending_nested_ended(state) {
            return vec![Action::Emit(ExecutionEvent::NestedEnded { team_lead })];
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

        // 4. Turn-cap anti-loop: some agent has produced `MAX_AGENT_TURNS`
        // responses without the run converging (the coordinator's own
        // termination is checked first, above, so a healthy run completes
        // normally before ever reaching this). This catches loops that
        // `synthesis_count` is blind to — notably an escalation ping-pong,
        // whose raw re-invocations never register as synthesis re-injections.
        // Force completion with the coordinator's latest narrative (falling
        // back to a partial digest if that narrative is empty), so `run.rs`
        // always receives non-empty content.
        if state
            .conversations
            .keys()
            .any(|agent| assistant_turn_count(state, agent) >= MAX_AGENT_TURNS)
        {
            let content = latest_response(state, &self.coordinator)
                .map(extract_narrative)
                .filter(|narrative| !narrative.trim().is_empty())
                .unwrap_or_else(|| build_partial_content(state));
            return vec![
                Action::Emit(ExecutionEvent::Warned {
                    code: "agent_turn_cap".to_string(),
                }),
                Action::Complete { content },
            ];
        }

        // 5. Dispatch any undispatched delegation round (the coordinator's
        // or a subordinate's), spawning its children.
        if let Some(agent) = self.agent_needing_dispatch(state) {
            return self.dispatch_actions(&agent, state);
        }

        // 6. Synthesis: an agent whose children have all settled is
        // re-invoked with their re-injected results (resolved bottom-up).
        if let Some(agent) = self.awaiting_synthesis_agent(state) {
            return self.synthesis_actions(state, &agent);
        }

        // 7a. Waiting on an in-flight reply: a subordinate settled but its
        // delegator is still awaiting a sibling that has been dispatched yet
        // not observed. Nothing to decide this round — stay idle. (In the real
        // socle loop an invocation resolves within the same batch, so this
        // arises only for synthetic/partial states; it is a genuine wait, not
        // a dead end.)
        if awaiting_in_flight(state) {
            return Vec::new();
        }

        // 7b. No-progress safety net: nothing to dispatch, nothing awaiting
        // synthesis, and nothing in flight — the run cannot advance. Returning
        // an empty batch here would let the socle break out of its loop with
        // `status = Running` (neither Completed nor Halted), a silent stall.
        // Emit an explicit terminal instead, with a best-effort partial digest
        // as content (non-empty for `run.rs`).
        vec![
            Action::Emit(ExecutionEvent::Warned {
                code: "no_progress".to_string(),
            }),
            Action::Complete {
                content: build_partial_content(state),
            },
        ]
    }
}

/// Parse a tier string as stored in `ExecutionState::routed_tiers` — the
/// `Debug` format of `ModelTier` (`"Fast"`/`"Pro"`/`"Max"`), matched
/// case-insensitively since `ModelRouted.tier` is produced via
/// `format!("{tier:?}")` in `HierarchicalDecider::model_routed_event` — back
/// into a `ModelTier`. Unrecognized strings fall back to `Pro`, the same
/// default `parse_latest_placeholder` uses for the bare `"latest"`
/// placeholder; this should not occur in practice since the only producer
/// of `routed_tiers` entries is that same `format!("{tier:?}")` call.
fn parse_routed_tier(tier: &str) -> ModelTier {
    match tier.to_lowercase().as_str() {
        "fast" => ModelTier::Fast,
        "max" => ModelTier::Max,
        _ => ModelTier::Pro,
    }
}

/// Shared C9 detection: if `agent` is the declared lead of a team that runs a
/// nested sub-pattern (blackboard/ring), return that `(team, pattern)`.
///
/// Single source of truth for "is this agent a nested-team lead?", used both
/// by the pure `HierarchicalDecider::nested_started_event` (to *mark* the
/// boundary) and by the impure `HierarchicalEffectRunner::run_invoke` (to
/// actually *launch* the sub-run). Config validation (`validate_config`,
/// `DuplicateLead`) guarantees a lead appears in at most one team, so the
/// first match is the only match.
fn nested_team_for<'a>(
    config: &'a OrchestrationConfig,
    agent: &str,
) -> Option<(&'a TeamConfig, NestedPattern)> {
    config.teams.iter().find_map(|team| {
        if team.lead.as_deref() == Some(agent) {
            team.pattern.map(|pattern| (team, pattern))
        } else {
            None
        }
    })
}

/// Resolve the effective `BlackboardConfig` for a nested team: start from
/// `base` (which already carries the remaining shared token budget) and apply
/// team-level overrides, falling back to the parent orchestration config.
/// Mirrors the legacy `apply_team_blackboard_overrides`
/// (`core::orchestration::hierarchical.rs`) — reimplemented over `&TeamConfig`
/// / `&OrchestrationConfig` rather than `&EngineContext`, keeping strict
/// coexistence with the legacy engine.
fn team_blackboard_config(
    mut base: BlackboardConfig,
    team: &TeamConfig,
    config: &OrchestrationConfig,
) -> BlackboardConfig {
    if let Some(v) = team.max_rounds.or(config.max_rounds) {
        base.max_rounds = v;
    }
    if let Some(v) = team.consensus_threshold.or(config.consensus_threshold) {
        base.consensus_threshold = v;
    }
    base
}

/// Fold a finished nested sub-run into the single `AgentObserved` event the
/// parent run records for `team_lead`: the sub-run's `outcome` text as the
/// lead's "response", and the child run's aggregated budget as the lead's
/// token/cost accounting (so it flows into the parent budget exactly like a
/// real LLM turn). The child's `budget_tokens_in/out` are `u64`; they are
/// narrowed to the `AgentObserved` `u32` fields with a saturating cast
/// (`u32::MAX` on the — practically impossible — overflow) rather than a
/// silent wrap. `model` is the sentinel `"nested"`, marking this observation
/// as a folded sub-run rather than a direct provider call.
fn nested_observed(team_lead: &str, outcome: String, child: &ExecutionState) -> ExecutionEvent {
    ExecutionEvent::AgentObserved {
        agent: team_lead.to_string(),
        content: outcome,
        tokens_in: u32::try_from(child.budget_tokens_in).unwrap_or(u32::MAX),
        tokens_out: u32::try_from(child.budget_tokens_out).unwrap_or(u32::MAX),
        cost: child.budget_cost,
        model: "nested".to_string(),
    }
}

/// Resolve the effective `RingConfig` for a nested team. Mirrors the legacy
/// `apply_team_ring_overrides`: `max_laps` takes the team override only (no
/// global fallback, matching legacy), `consensus_threshold` falls back to the
/// parent config.
fn team_ring_config(
    mut base: RingConfig,
    team: &TeamConfig,
    config: &OrchestrationConfig,
) -> RingConfig {
    if let Some(v) = team.max_laps {
        base.max_laps = v;
    }
    if let Some(v) = team.consensus_threshold.or(config.consensus_threshold) {
        base.consensus_threshold = v;
    }
    base
}

// ── HierarchicalEffectRunner (Task 4): the sole async/impure effect ──

/// Executes the actual LLM call behind `Action::Invoke` for the hierarchical
/// pattern and turns the raw provider response into the `AgentObserved`
/// event the pure loop/decider expect.
///
/// This is the *only* impure/async piece of the event-sourced hierarchical
/// engine — `plan_from_response` and `HierarchicalDecider` above never touch
/// I/O. Coexists with the legacy `core::orchestration::hierarchical::HierarchicalEngine`
/// (this struct is not wired into it, and does not import from it — strict
/// coexistence, mirroring `format_results` above).
pub struct HierarchicalEffectRunner {
    /// All known agents by name (system prompt, model, temperature, …).
    pub agents: BTreeMap<String, Agent>,
    /// Provider instance per agent name.
    pub providers: BTreeMap<String, Arc<dyn Provider>>,
    /// Orchestration config, used to build each agent's enriched
    /// (orchestration-aware) system prompt via
    /// `context_injection::build_orchestration_prompt`.
    pub config: OrchestrationConfig,
    /// Routing rules forwarded to a nested C9 sub-run
    /// (`run_blackboard_es`/`run_ring_es`) so its `latest:auto` members route
    /// consistently with the parent run. Defaults to `RoutingRules::default()`
    /// via [`HierarchicalEffectRunner::new`]; production callers thread the
    /// real rules in with [`HierarchicalEffectRunner::with_routing_rules`].
    /// Unused on the flat (non-nested) invoke path.
    pub routing_rules: RoutingRules,
}

impl HierarchicalEffectRunner {
    /// Construct a new `HierarchicalEffectRunner` from its immutable inputs.
    /// `routing_rules` defaults to `RoutingRules::default()` — the correct
    /// "no custom routing" value for the many unit tests that don't exercise
    /// nested sub-runs; production assembly (`run_hierarchical_es`) overrides
    /// it via [`Self::with_routing_rules`].
    pub fn new(
        agents: BTreeMap<String, Agent>,
        providers: BTreeMap<String, Arc<dyn Provider>>,
        config: OrchestrationConfig,
    ) -> Self {
        Self {
            agents,
            providers,
            config,
            routing_rules: RoutingRules::default(),
        }
    }

    /// Thread the run's `routing_rules` into this effect runner, so a nested
    /// C9 sub-run launched from `run_invoke` routes its members' `latest:auto`
    /// models the same way the parent run does. Builder style to keep `new`'s
    /// signature stable for the existing unit tests.
    pub fn with_routing_rules(mut self, routing_rules: RoutingRules) -> Self {
        self.routing_rules = routing_rules;
        self
    }

    /// Launch a nested C9 sub-run for a team `team_lead` leads: run the team's
    /// members (`team.agents`) through the event-sourced blackboard/ring
    /// engine on a **dedicated, ephemeral child `InMemoryLog`**, then surface
    /// the sub-run's outcome + aggregated metrics back into the parent run as
    /// a single `AgentObserved` for `team_lead`.
    ///
    /// ## Child log choice
    /// The sub-run writes its fine-grained events (`RoundStarted`,
    /// `BoardEntryAdded`, `VoteCast`, …) to a *local* `InMemoryLog` that is
    /// discarded when this function returns — they never enter the parent log.
    /// Only three things cross the boundary into the parent log: the
    /// `NestedStarted`/`NestedEnded` markers (emitted by the decider) and this
    /// one folded `AgentObserved` carrying the outcome text + aggregated
    /// tokens/cost. This is deliberate: it keeps the parent log's *replay*
    /// deterministic and effect-free — replay reconstructs `team_lead`'s
    /// observed outcome directly from the baked-in `AgentObserved`, with no
    /// need to (and no risk of) re-executing the sub-run. The trade-off,
    /// matching the intent of the legacy `run_nested_team` (which surfaced only
    /// outcome + metrics upward, stashing the full `NestedRun` separately), is
    /// that the sub-run's internal event trace is not persisted in the parent
    /// log.
    ///
    /// ## Budget
    /// The sub-run receives the parent's *remaining* shared token budget
    /// (`config.token_budget − tokens already consumed`, saturating at 0), or
    /// the sub-pattern's own default when the parent sets no budget — exactly
    /// as legacy `run_nested_team` computes it. The tokens the sub-run consumes
    /// flow back into the parent budget through the aggregated `AgentObserved`
    /// (folded by `es::state::apply` like any other observation).
    ///
    /// `config.cost_limit` (issue #345) gets the identical treatment, in
    /// `f64`: `remaining = (total − state.budget_cost).max(0.0)`, the float
    /// analogue of `saturating_sub`. Before #345 this was the one ceiling
    /// still handed down verbatim — never decremented — so a purely
    /// *sequential* chain of nested delegations (`batch_len == 1` at every
    /// hop, no fan-out involved at all) could each spend up to the full
    /// parental `cost_limit` again, a strictly wider defect than #291's
    /// (which needed `InvokeParallel` to manifest). `None` (no global
    /// `cost_limit`) still means "hand the sub-pattern no cost ceiling",
    /// unaffected by `batch_len` — matching the `token_budget` `None` arm.
    ///
    /// ## Parallel batches (issue #291)
    /// `run_invoke` — and therefore this function — is called once per entry
    /// of an `Action::InvokeParallel` batch, all sharing the exact same
    /// `state` snapshot (see `es::engine::run_loop`): nothing mutates it
    /// between sibling calls, so every sibling would otherwise compute the
    /// *identical* "remaining" budget and could each independently spend up
    /// to all of it — the total overrun growing with the batch size. `batch_len`
    /// (the number of siblings dispatched together, `1` for a solitary
    /// `Action::Invoke`) is what lets this function partition the remaining
    /// budget instead: each child gets `remaining / batch_len`, floor-divided
    /// for `token_budget` (`u64`) — `cost_limit` (`f64`) divides the same way
    /// but without the floor, since a fractional dollar ceiling is
    /// meaningful where a fractional token isn't; a remaining cost smaller
    /// than `batch_len` therefore still yields a small nonzero share per
    /// child instead of truncating to zero. An unequal, demand-driven split
    /// (a shared pot children draw from as
    /// they run) would use the budget more efficiently, but requires shared
    /// *mutable* state across concurrent effects — which this event-sourced
    /// engine cannot allow without breaking `--replay` determinism (the
    /// gaveldrop `direct-replay` case pins exactly that guarantee). Equal,
    /// floor-divided partition is the deterministic alternative: it depends
    /// only on inputs every sibling already has (the shared snapshot +
    /// `batch_len`), so replay reconstructs the identical split with no
    /// coordination at all.
    ///
    /// The `None` (no global `token_budget`) arm is unaffected: it always
    /// hands back the sub-pattern's own default, regardless of `batch_len`.
    /// `cost_limit`'s `None` arm is analogous: it always hands back `None`
    /// (no cost ceiling for the sub-pattern), regardless of `batch_len`.
    ///
    /// Consequence, stated plainly: a child in a parallel batch may now halt
    /// earlier than it would have running alone — even if every sibling in
    /// its batch ends up consuming nothing at all. That is the price of a
    /// ceiling that holds without shared mutable state. A smarter split —
    /// the coordinator receiving the remaining budget in its own context and
    /// allocating it across delegations itself, rather than an equal a
    /// priori division — needs a structured delegation channel the agent can
    /// read/act on, which is deferred to #251 (native tool calling).
    async fn run_nested(
        &self,
        team_lead: &str,
        task: &str,
        team: &TeamConfig,
        pattern: NestedPattern,
        state: &ExecutionState,
        batch_len: usize,
    ) -> anyhow::Result<ExecutionEvent> {
        // Scope agents/providers to the team's members (the lead is the
        // arbiter, not a sub-run participant — matching legacy).
        let mut member_agents: BTreeMap<String, Agent> = BTreeMap::new();
        let mut member_providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        for name in &team.agents {
            let agent = self
                .agents
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("nested team agent '{name}' not found"))?
                .clone();
            let provider = self
                .providers
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("no provider for nested team agent '{name}'"))?;
            member_agents.insert(name.clone(), agent);
            member_providers.insert(name.clone(), Arc::clone(provider));
        }

        // Remaining shared token budget, partitioned equally across this
        // parallel batch (never below zero; see the doc comment above for
        // why an equal, floor-divided split — not a shared mutable pot — is
        // what keeps this deterministic), or the sub-pattern default when
        // the parent run sets no global budget (unaffected by `batch_len`).
        let remaining_budget = match self.config.token_budget {
            Some(total) => {
                let remaining =
                    total.saturating_sub(state.budget_tokens_in + state.budget_tokens_out);
                remaining / (batch_len.max(1) as u64)
            }
            None => match pattern {
                NestedPattern::Blackboard => BlackboardConfig::default().token_budget,
                NestedPattern::Ring => RingConfig::default().token_budget,
            },
        };

        // Remaining cost budget (USD), the `cost_limit` counterpart of
        // `remaining_budget` above (issue #345 — the same defect class as
        // #291, just wider: `self.config.cost_limit` was previously handed
        // down *verbatim*, both across an `InvokeParallel` batch and across
        // a purely sequential chain of nested delegations, so it was never
        // decremented at all — a chain could spend the full ceiling at every
        // hop). Subtract what `state.budget_cost` already records as spent
        // (every direct turn plus every prior nested sub-run, folded back by
        // `nested_observed`), floor at zero (the `f64` analogue of
        // `saturating_sub`), then divide equally across this batch for the
        // exact replay-determinism reason `remaining_budget` is: no shared
        // mutable pot, so every sibling can compute the identical split from
        // inputs it already has. `batch_len` is floored to `1` (same
        // belt-and-braces as `remaining_budget`'s `.max(1)`) — it should
        // never be `0` in practice, but this keeps the division total
        // instead of a `NaN`/`inf` trap either way. Unlike the integer
        // `token_budget` split, this is float division: a remaining cost
        // smaller than the batch length does not floor to zero the way
        // integer division would — every sibling still gets a (tiny,
        // nonzero) proportional share rather than being starved outright.
        // `None` (no global `cost_limit`) is unaffected: `run_blackboard_es`
        // / `run_ring_es` are handed `None` regardless of `batch_len`, so
        // the sub-pattern's own cost guard stays off exactly as before.
        let remaining_cost_limit = self.config.cost_limit.map(|total| {
            let remaining = (total - state.budget_cost).max(0.0);
            remaining / (batch_len.max(1) as f64)
        });

        // Deterministic child run_id (carries the parent id + lead) on a
        // dedicated, ephemeral child log (see the doc comment above).
        let child_run_id = format!("{}::nested::{}", state.run_id, team_lead);
        let mut child_log = InMemoryLog::default();

        let child_state = match pattern {
            NestedPattern::Blackboard => {
                let cfg = team_blackboard_config(
                    BlackboardConfig {
                        token_budget: remaining_budget,
                        ..BlackboardConfig::default()
                    },
                    team,
                    &self.config,
                );
                run_blackboard_es(
                    &child_run_id,
                    task,
                    member_agents,
                    member_providers,
                    cfg,
                    self.routing_rules.clone(),
                    remaining_cost_limit,
                    &mut child_log,
                )
                .await?
            }
            NestedPattern::Ring => {
                let cfg = team_ring_config(
                    RingConfig {
                        token_budget: remaining_budget,
                        ..RingConfig::default()
                    },
                    team,
                    &self.config,
                );
                // `resolve_votes` needs the same vote weights the sub-run's
                // `RingDecider` used — rebuild them from the scoped members
                // before ownership moves into `run_ring_es`.
                let vote_weights = vote_weights_from_agents(&member_agents);
                // `team.agents` is the chain order the C9 team was declared
                // with (`armadai.yaml`'s `teams: [...] agents:` list) — pass
                // it straight through as `agent_order` so the nested ring
                // circulates in that order rather than `member_agents`'
                // BTreeMap-alphabetical iteration (OH1 Lot 4 Task 3, Bug A).
                let child = run_ring_es(
                    &child_run_id,
                    task,
                    member_agents,
                    team.agents.clone(),
                    member_providers,
                    cfg.clone(),
                    self.routing_rules.clone(),
                    remaining_cost_limit,
                    &mut child_log,
                )
                .await?;
                // Re-derive the outcome from the child state (the ring outcome
                // is `resolve_votes` over the recorded votes — see
                // `RingDecider`). Stash it in a closure-free local by short-
                // circuiting: build the AgentObserved right here for ring, so
                // the borrow of `cfg`/`vote_weights` stays local.
                let outcome = resolve_votes(&child, &vote_weights, &cfg);
                return Ok(nested_observed(team_lead, outcome, &child));
            }
        };

        // Blackboard outcome: the deterministic board digest (`[agent]
        // content` per entry), identical to the legacy engine's blackboard
        // final answer.
        let outcome = build_board_result(&child_state);
        Ok(nested_observed(team_lead, outcome, &child_state))
    }

    /// Reconstruct `agents_info` (name → description, the first non-empty
    /// line of the agent's `system_prompt`) from `self.agents`, for
    /// `context_injection::build_orchestration_prompt`.
    ///
    /// Mirrors `HierarchicalEngine::with_routing_rules`'s construction in the
    /// legacy engine (`core::orchestration::hierarchical.rs`) — reimplemented
    /// here rather than imported, keeping strict coexistence (neither engine
    /// depends on the other).
    fn agents_info(&self) -> HashMap<String, AgentInfo> {
        self.agents
            .iter()
            .map(|(name, agent)| {
                let description = agent
                    .system_prompt
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.trim().to_string());
                (
                    name.clone(),
                    AgentInfo {
                        name: name.clone(),
                        description,
                    },
                )
            })
            .collect()
    }

    /// Build the enriched system prompt for `agent_name`: its own
    /// `system_prompt`, plus the orchestration-protocol block from
    /// `context_injection::build_orchestration_prompt` when one applies
    /// (hierarchical pattern enabled, per `self.config`). Falls back to a
    /// generic default if `agent_name` isn't in `self.agents` — should not
    /// happen once the caller (`run_invoke`) has already resolved the agent,
    /// but keeps this helper total.
    fn enriched_system_prompt(&self, agent_name: &str) -> String {
        let base = self
            .agents
            .get(agent_name)
            .map(|a| a.system_prompt.as_str())
            .unwrap_or("You are a helpful assistant.");
        match build_orchestration_prompt(agent_name, &self.config, &self.agents_info()) {
            Some(block) => format!("{base}{block}"),
            None => base.to_string(),
        }
    }
}

#[async_trait]
impl EffectRunner for HierarchicalEffectRunner {
    async fn run_invoke(
        &self,
        agent: &str,
        input: &str,
        state: &ExecutionState,
        batch_len: usize,
    ) -> anyhow::Result<ExecutionEvent> {
        // C9 short-circuit: if `agent` leads a team declaring a nested
        // sub-pattern, drive a full event-sourced blackboard/ring sub-run for
        // that team instead of a flat LLM call, and surface its outcome +
        // aggregated metrics as the lead's `AgentObserved` (see `run_nested`,
        // whose doc comment covers `batch_len`'s role in partitioning the
        // budget across a parallel batch — issue #291).
        if let Some((team, pattern)) = nested_team_for(&self.config, agent) {
            return self
                .run_nested(agent, input, team, pattern, state, batch_len)
                .await;
        }

        let agent_def = self
            .agents
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("Unknown agent '{agent}' — no Agent definition"))?;
        let provider = self
            .providers
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("No provider configured for agent '{agent}'"))?;

        let system_prompt = self.enriched_system_prompt(agent);

        // The generic event-sourced loop (`run_event_sourced` in
        // `es::engine`) always applies `AgentInvoked{agent, input}` — which
        // pushes exactly this `user` turn — into `state` *before* calling
        // `run_invoke`, so in production `state.conversations[agent]`
        // already ends with it. Tests that exercise `run_invoke` directly
        // against a hand-built state (skipping that step) won't have it yet.
        // Append it only if it isn't already the trailing turn, so behavior
        // is correct under both calling conventions without duplicating the
        // turn on the (production) common path.
        let mut messages = state.conversations.get(agent).cloned().unwrap_or_default();
        let already_applied = messages
            .last()
            .is_some_and(|m| m.role == "user" && m.content == input);
        if !already_applied {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: input.to_string(),
            });
        }

        // `agent.metadata.model` is passed through verbatim, *except* for
        // the exact `"latest:auto"` placeholder: `HierarchicalDecider`
        // (Task 3) always emits a `ModelRouted{agent, tier, ..}` event ahead
        // of the matching `Invoke` for such an agent (see
        // `model_routed_event`/`invoke_actions`), which `es::state::apply`
        // projects into `state.routed_tiers`. We read that tier back
        // here and resolve it to a concrete model via
        // `crate::model_resolution::resolve_model_for_tier` — this
        // is the only place in the event-sourced hierarchical engine that
        // does so, keeping the pure `Decider` free of that effectful lookup.
        // Every other `latest:*` placeholder resolves in the `else`
        // branch below (#376); a concrete model id is sent as-is.
        let raw_model = agent_def
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let model = if raw_model == "latest:auto" {
            let tier = match state.routed_tiers.get(agent) {
                Some(tier_str) => parse_routed_tier(tier_str),
                None => {
                    // Defensive fallback: `HierarchicalDecider` always emits
                    // `ModelRouted` before dispatching `Invoke` for a
                    // `latest:auto` agent, so this branch should be
                    // unreachable on the production path (`run_event_sourced`
                    // applies events into `state` before calling
                    // `run_invoke`). It can only be reached by a hand-built
                    // state in a test, or a future decider regression. Rather
                    // than leak the literal `"latest:auto"` string to the
                    // provider, fall back to the `Pro` tier — the same
                    // default `parse_latest_placeholder` uses for the bare
                    // `"latest"` placeholder — and log it loudly so the gap
                    // is visible.
                    tracing::warn!(
                        agent,
                        "no ModelRouted tier recorded for latest:auto agent; falling back to Pro tier"
                    );
                    ModelTier::Pro
                }
            };
            resolve_routed_tier(&agent_def.metadata.provider, tier)
        } else {
            // Every OTHER `latest:*` placeholder (`latest`, `latest:fast`,
            // `latest:pro`, `latest:max`, …) has a tier that is known
            // statically, so it is resolved right here — the last gate
            // before the string becomes a provider's model name. Until #376
            // this branch passed them through verbatim, and an API provider
            // was asked for a model literally called `latest:pro`.
            resolve_tier_placeholder(&raw_model, &agent_def.metadata.provider).unwrap_or(raw_model)
        };

        let request = CompletionRequest {
            model,
            system_prompt,
            messages,
            temperature: agent_def.metadata.temperature,
            max_tokens: agent_def.metadata.max_tokens,
        };

        let response = provider.complete(request).await?;

        Ok(ExecutionEvent::AgentObserved {
            agent: agent.to_string(),
            content: response.content,
            tokens_in: response.tokens_in,
            tokens_out: response.tokens_out,
            cost: response.cost,
            model: response.model,
        })
    }
}

// ── run_hierarchical_es (Task 5): end-to-end assembly ────────────────

/// Run a complete hierarchical orchestration end-to-end through the
/// event-sourced engine: builds the initial `RunStarted` event, constructs a
/// [`HierarchicalDecider`] + [`HierarchicalEffectRunner`] from `config` /
/// `agents` / `providers` / `routing_rules`, and drives them through
/// [`run_event_sourced`], returning the final [`ExecutionState`].
///
/// `run_id` is accepted explicitly rather than generated internally, so
/// callers — notably tests proving replay determinism — can pass a fixed id
/// and later reconstruct the same state purely from the log via
/// [`super::engine::replay`].
///
/// Depth/iteration/token/cost limits are derived from `config`:
/// `OrchestrationConfig::max_depth()`/`max_iterations()` apply their
/// documented defaults (5 / 50) when unset; `token_budget` is narrowed from
/// `Option<u64>` to the `Option<u32>` `HierarchicalDecider` expects
/// (saturating at `u32::MAX` — well above any realistic token budget) and
/// `cost_limit` passes through unchanged, `None` meaning "no limit" for
/// both.
///
/// Coexists with the legacy
/// `core::orchestration::hierarchical::HierarchicalEngine` — this function
/// is not called from `run.rs`; wiring it in as the active engine is a
/// later lot (the bascule).
#[allow(clippy::too_many_arguments)]
pub async fn run_hierarchical_es(
    run_id: &str,
    coordinator: &str,
    input: &str,
    config: OrchestrationConfig,
    agents: BTreeMap<String, Agent>,
    providers: BTreeMap<String, Arc<dyn Provider>>,
    routing_rules: RoutingRules,
    log: &mut impl EventLog,
) -> anyhow::Result<ExecutionState> {
    let agent_names: Vec<String> = agents.keys().cloned().collect();
    let roster = super::bridge::roster_from_agents(&agents);
    let initial = vec![
        ExecutionEvent::RunStarted {
            run_id: run_id.to_string(),
            pattern: "hierarchical".to_string(),
            agents: agent_names,
            input: input.to_string(),
            project: None,
            roster,
        },
        ExecutionEvent::ConfigSnapshot {
            config_json: serde_json::to_string(&config).unwrap_or_default(),
        },
    ];

    let max_depth = config.max_depth();
    let max_iterations = config.max_iterations();
    let token_budget = config
        .token_budget
        .map(|b| u32::try_from(b).unwrap_or(u32::MAX));
    let cost_limit = config.cost_limit;

    let decider = HierarchicalDecider::new(
        coordinator.to_string(),
        input.to_string(),
        config.clone(),
        agents.clone(),
        routing_rules.clone(),
        max_depth,
        max_iterations,
        token_budget,
        cost_limit,
    );
    let effects =
        HierarchicalEffectRunner::new(agents, providers, config).with_routing_rules(routing_rules);

    run_event_sourced(run_id, initial, &decider, &effects, log).await
}

/// Resume a previously interrupted `hierarchical` run (OH1 Lot 6, Task 3):
/// recovers the run's original `input` and the roster (for the coordinator
/// fallback below) from `RunStarted`, and the run's `OrchestrationConfig`
/// from `ConfigSnapshot` (see
/// [`super::engine::run_started_roster_and_input`]/[`super::engine::config_snapshot`]).
/// The coordinator name is `config.coordinator` when set, else the first
/// roster entry — the SAME fallback [`run_hierarchical_es`]'s caller
/// (`run_orchestrated_inner` in `cli::run`) applies, just read from the
/// log's roster instead of the CLI's freshly-resolved `agent_names`. Rebuilds
/// the SAME [`HierarchicalDecider`]/[`HierarchicalEffectRunner`] pair
/// [`run_hierarchical_es`] would, and drives
/// [`super::engine::resume_event_sourced`] instead of appending a fresh
/// `RunStarted`/`ConfigSnapshot`.
///
/// `agents`/`providers` must be the roster reloaded from the project on disk
/// (keyed by the same roster keys the original run used —
/// `ExecutionState::agents`, folded from `RunStarted`). Unlike
/// blackboard/ring, `hierarchical`'s `cost_limit` and `token_budget` both
/// live directly on `OrchestrationConfig`, so — unlike those two patterns —
/// nothing here needs re-deriving from the project's config on disk beyond
/// the roster/providers themselves.
///
/// Bails if `run_id` has no recorded `RunStarted` (unknown run) or isn't
/// currently `Running` (see `resume_event_sourced`).
pub async fn resume_hierarchical_es(
    run_id: &str,
    agents: BTreeMap<String, Agent>,
    providers: BTreeMap<String, Arc<dyn Provider>>,
    routing_rules: RoutingRules,
    log: &mut impl EventLog,
) -> anyhow::Result<ExecutionState> {
    use super::engine::{config_snapshot, resume_event_sourced, run_started_roster_and_input};

    let events = log.events(run_id)?;
    let (roster, input) = run_started_roster_and_input(&events)
        .ok_or_else(|| anyhow::anyhow!("no run found for id {run_id}"))?;
    let config: OrchestrationConfig = config_snapshot(&events);

    let coordinator = config
        .coordinator
        .clone()
        .unwrap_or_else(|| roster.first().cloned().unwrap_or_default());
    let max_depth = config.max_depth();
    let max_iterations = config.max_iterations();
    let token_budget = config
        .token_budget
        .map(|b| u32::try_from(b).unwrap_or(u32::MAX));
    let cost_limit = config.cost_limit;

    let decider = HierarchicalDecider::new(
        coordinator,
        input,
        config.clone(),
        agents.clone(),
        routing_rules.clone(),
        max_depth,
        max_iterations,
        token_budget,
        cost_limit,
    );
    let effects =
        HierarchicalEffectRunner::new(agents, providers, config).with_routing_rules(routing_rules);

    resume_event_sourced(run_id, &decider, &effects, log).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::es::event::ExecutionEvent;
    use crate::orchestration::{OrchestrationPattern, TeamConfig};

    /// Tests for `HierarchicalDecider` (Task 3): pure decision function
    /// built on top of `plan_from_response` (Tasks 1-2). Named `decide` so
    /// `cargo test es::hierarchical::tests::decide` targets this module.
    mod decide {
        use super::*;
        use crate::agent::{Agent, AgentMetadata};
        use crate::orchestration::es::engine::{Action, Decider};
        use crate::orchestration::es::state::fold;
        use crate::orchestration::{NestedPattern, OrchestrationPattern, TeamConfig};
        use crate::routing::RoutingRules;
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
                roster: Default::default(),
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

        // (c) coordinator delegates to two siblings → one InvokeParallel
        // (batch in line order, cap = default 4), preceded by both Delegated
        // emits in order.
        #[test]
        fn two_delegations_become_one_invoke_parallel_in_order() {
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

            // Exactly one InvokeParallel, batch in line order, default cap 4.
            let batch_agents: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::InvokeParallel {
                        batch,
                        max_concurrency,
                    } => {
                        assert_eq!(*max_concurrency, 4);
                        Some(batch.iter().map(|s| s.agent.as_str()).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(batch_agents, vec!["core-specialist", "qa-specialist"]);
            assert_eq!(
                actions
                    .iter()
                    .filter(|a| matches!(a, Action::InvokeParallel { .. }))
                    .count(),
                1
            );
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Invoke { .. })),
                "fan-out of 2 must not emit any sequential Invoke"
            );

            // Both Delegated emitted in line order, before the InvokeParallel.
            let delegated_targets: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::Emit(ExecutionEvent::Delegated { to, .. }) => Some(to.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(delegated_targets, vec!["core-specialist", "qa-specialist"]);
            let parallel_pos = actions
                .iter()
                .position(|a| matches!(a, Action::InvokeParallel { .. }))
                .unwrap();
            let last_delegated_pos = actions
                .iter()
                .rposition(|a| matches!(a, Action::Emit(ExecutionEvent::Delegated { .. })))
                .unwrap();
            assert!(last_delegated_pos < parallel_pos);
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

        // (d-bis) guard-at-source (OH1 Lot 4 Task 3, reconciliation A): a
        // response that delegates to a target one level too deep must halt
        // (`Warned{max_depth}` + `Complete`) *without* emitting that target's
        // `Delegated`/`Invoke` at all — unlike (d) above, `core-specialist`
        // here has *not yet been invoked*: this is the normal dispatch path
        // (`agent_needing_dispatch` → `dispatch_actions`), not the reactive
        // `breached_limit` fallback. With `max_depth: 1`, `dev-lead` sits at
        // depth 0, so delegating to `core-specialist` would invoke it at
        // depth `0 + 1 == max_depth` — exactly the legacy `invoke_agent`
        // bail condition (`depth >= max_depth`).
        #[test]
        fn dispatch_guard_at_source_halts_before_invoking_too_deep_child() {
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
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 2);
            assert!(
                matches!(&actions[0], Action::Emit(ExecutionEvent::Warned{code}) if code == "max_depth")
            );
            assert!(matches!(&actions[1], Action::Complete { .. }));
            assert!(
                !actions.iter().any(
                    |a| matches!(a, Action::Invoke { agent, .. } if agent == "core-specialist")
                ),
                "core-specialist is one level too deep and must never be invoked"
            );
            assert!(
                !actions
                    .iter()
                    .any(|a| matches!(a, Action::Emit(ExecutionEvent::Delegated { .. }))),
                "no Delegated event should be emitted for a target that is never dispatched"
            );
        }

        // (e) agent to invoke leads a nested-pattern team → NestedStarted
        // emitted, ordered before the lead's own `Invoke`, and with no
        // sub-run of the nested team's members (`core-a`/`core-b`) started —
        // the actual sub-run is Lot 3's `EffectRunner` hook (see the NOTE on
        // `nested_started_event` above).
        //
        // This is the "test à garantir" from the OH1 Lot 2 Task 6 brief:
        // `decider_emits_nested_started_for_team_lead_with_pattern`.
        #[test]
        fn decider_emits_nested_started_for_team_lead_with_pattern() {
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

            let nested_started_pos = actions.iter().position(|a| {
                matches!(
                    a,
                    Action::Emit(ExecutionEvent::NestedStarted { team_lead, pattern })
                    if team_lead == "core-lead" && pattern == "blackboard"
                )
            });
            assert!(
                nested_started_pos.is_some(),
                "expected NestedStarted{{team_lead: \"core-lead\", pattern: \"blackboard\"}} \
                 in {actions:?}"
            );

            let lead_invoke_pos = actions
                .iter()
                .position(|a| matches!(a, Action::Invoke { agent, .. } if agent == "core-lead"));
            assert!(
                lead_invoke_pos.is_some(),
                "expected Invoke{{agent: \"core-lead\"}} in {actions:?}"
            );
            assert!(
                nested_started_pos.unwrap() < lead_invoke_pos.unwrap(),
                "NestedStarted must be emitted before the lead's own Invoke, got {actions:?}"
            );

            // No sub-run: `decide` never invokes the nested team's own
            // members directly — only `core-lead` (as a normal agent in the
            // *parent* run) is invoked. The real blackboard/ring sub-run for
            // `core-a`/`core-b` is Lot 3's concern (`EffectRunner`), not this
            // pure decision function.
            assert!(
                !actions
                    .iter()
                    .any(|a| matches!(a, Action::Invoke { agent, .. } if agent == "core-a" || agent == "core-b")),
                "decide must not start the nested sub-run itself, got {actions:?}"
            );
            assert_eq!(
                actions.len(),
                3,
                "expected exactly [Emit(NestedStarted), Emit(Delegated), Invoke(core-lead)], \
                 got {actions:?}"
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

        // I1: escalation ping-pong (subordinate escalates → coordinator
        // re-delegates → subordinate re-escalates → …) never re-injects
        // `format_results`, so `synthesis_count` stays 0 and cannot stop it.
        // The turn-cap guard must catch it: once the coordinator has produced
        // `MAX_AGENT_TURNS` (4) responses without converging, `decide` must
        // `Warned{agent_turn_cap}` + `Complete` rather than dispatch yet
        // another invocation.
        #[test]
        fn escalation_ping_pong_is_capped() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                ],
                base_config(),
                // Generous depth/iteration caps: the point is that neither of
                // them fires — the *turn cap* is what stops the ping-pong.
                50,
                500,
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
                    content: "Je délègue.\n@core-specialist: fais X".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
            ];
            // Four escalate/re-delegate round-trips. Each pushes: Delegated,
            // the subordinate's escalation turn, Escalated, then the
            // coordinator's re-delegation turn. dev-lead accrues one assistant
            // turn per round-trip (its kick-off above is turn #1, so the 3rd
            // round-trip yields its 4th turn → the cap fires).
            let rounds = [
                ("fais X", "retry", "besoin d'aide", "Réessaie."),
                ("retry", "retry2", "encore besoin", "Réessaie encore."),
                ("retry2", "retry3", "toujours besoin", "Je relance."),
            ];
            for (task, next_task, escalation, narrative) in rounds {
                events.push(ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-specialist".into(),
                    task: task.into(),
                    depth: 1,
                });
                events.push(ExecutionEvent::AgentInvoked {
                    agent: "core-specialist".into(),
                    input: task.into(),
                });
                events.push(ExecutionEvent::AgentObserved {
                    agent: "core-specialist".into(),
                    content: format!("Je bloque.\n@dev-lead: {escalation}"),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                });
                events.push(ExecutionEvent::Escalated {
                    from: "core-specialist".into(),
                    to: "dev-lead".into(),
                    message: escalation.into(),
                });
                events.push(ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: escalation.into(),
                });
                events.push(ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: format!("{narrative}\n@core-specialist: {next_task}"),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                });
            }

            let state = fold(&events);
            // Sanity: neither the depth nor the iteration socle cap has been
            // hit, and no synthesis re-injection ever happened.
            assert!(current_depth(&state) < 50);
            assert!(invocation_count(&state) < 500);
            assert_eq!(synthesis_count(&state, "dev-lead"), 0);
            assert!(assistant_turn_count(&state, "dev-lead") >= MAX_AGENT_TURNS);

            let actions = dec.decide(&state);
            // The turn cap must terminate the run — NOT dispatch another
            // invocation.
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Invoke { .. })),
                "turn cap must stop the ping-pong, not invoke again: {actions:?}"
            );
            assert!(actions.iter().any(|a| matches!(
                a,
                Action::Emit(ExecutionEvent::Warned { code }) if code == "agent_turn_cap"
            )));
            let completed = actions.iter().find_map(|a| match a {
                Action::Complete { content } => Some(content.as_str()),
                _ => None,
            });
            let content = completed.expect("turn cap must Complete the run");
            assert!(
                !content.trim().is_empty(),
                "completion content must be non-empty"
            );
        }

        // I2: three-level hierarchy C → L → {A, B}. Settlement must propagate
        // bottom-up: once A and B settle, `decide` synthesizes L (NOT the run);
        // once L settles, `decide` synthesizes C — the run does not end on L's
        // FinalAnswer.
        #[test]
        fn multi_level_synthesis_propagates_bottom_up() {
            let config = OrchestrationConfig {
                enabled: true,
                pattern: OrchestrationPattern::Hierarchical,
                coordinator: Some("dev-lead".to_string()),
                teams: vec![TeamConfig {
                    lead: Some("core-lead".to_string()),
                    agents: vec!["core-a".to_string(), "core-b".to_string()],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-lead", "concrete-model"),
                    ("core-a", "concrete-model"),
                    ("core-b", "concrete-model"),
                ],
                config,
                5,
                50,
                None,
                None,
            );
            // C delegates to L; L delegates to A and B; A and B answer.
            let mut events = vec![
                run_started(&["dev-lead"]),
                ExecutionEvent::AgentInvoked {
                    agent: "dev-lead".into(),
                    input: "build X".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "dev-lead".into(),
                    content: "@core-lead: gère la feature".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
                ExecutionEvent::Delegated {
                    from: "dev-lead".into(),
                    to: "core-lead".into(),
                    task: "gère la feature".into(),
                    depth: 1,
                },
                ExecutionEvent::AgentInvoked {
                    agent: "core-lead".into(),
                    input: "gère la feature".into(),
                },
                ExecutionEvent::AgentObserved {
                    agent: "core-lead".into(),
                    content: "@core-a: fais A\n@core-b: fais B".into(),
                    tokens_in: 5,
                    tokens_out: 5,
                    cost: 0.0,
                    model: "m".into(),
                },
                ExecutionEvent::Delegated {
                    from: "core-lead".into(),
                    to: "core-a".into(),
                    task: "fais A".into(),
                    depth: 2,
                },
                ExecutionEvent::Delegated {
                    from: "core-lead".into(),
                    to: "core-b".into(),
                    task: "fais B".into(),
                    depth: 2,
                },
            ];
            events.extend(subordinate_turn("core-a", "fais A", "A est fait."));
            events.extend(subordinate_turn("core-b", "fais B", "B est fait."));

            // Step 1: L's children have settled → synthesize L, not the run.
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Complete { .. })),
                "L's children settling must not end the run: {actions:?}"
            );
            let l_reinjection = actions.iter().find_map(|a| match a {
                Action::Invoke { agent, input } if agent == "core-lead" => Some(input.as_str()),
                _ => None,
            });
            let l_input = l_reinjection.expect("expected a synthesis Invoke to the lead L");
            assert!(
                l_input.contains("[Result from @core-a]") && l_input.contains("A est fait."),
                "A's result must be re-injected into L, got: {l_input}"
            );
            assert!(
                l_input.contains("[Result from @core-b]") && l_input.contains("B est fait."),
                "B's result must be re-injected into L, got: {l_input}"
            );

            // Step 2: L now produces its own FinalAnswer (after synthesis).
            events.push(ExecutionEvent::AgentInvoked {
                agent: "core-lead".into(),
                input: l_input.to_string(),
            });
            events.push(ExecutionEvent::AgentObserved {
                agent: "core-lead".into(),
                content: "Feature complète.".into(),
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.0,
                model: "m".into(),
            });

            // `decide` must now synthesize C with L's result re-injected —
            // NOT terminate the run on L's FinalAnswer.
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Complete { .. })),
                "L's FinalAnswer must not end the run — C must synthesize: {actions:?}"
            );
            let c_reinjection = actions.iter().find_map(|a| match a {
                Action::Invoke { agent, input } if agent == "dev-lead" => Some(input.as_str()),
                _ => None,
            });
            let c_input =
                c_reinjection.expect("expected a bottom-up synthesis Invoke to coordinator C");
            assert!(
                c_input.contains("[Result from @core-lead]")
                    && c_input.contains("Feature complète."),
                "L's result must propagate up and be re-injected into C, got: {c_input}"
            );
        }

        // I3: an agent configured with the exact `latest:auto` model must emit
        // a `ModelRouted` event *before* its `Invoke`; agents on a concrete
        // model must not.
        #[test]
        fn latest_auto_agent_emits_model_routed_before_invoke() {
            let dec = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "latest:auto"),
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

            // Exactly one ModelRouted, for the latest:auto agent only.
            let routed: Vec<&str> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::Emit(ExecutionEvent::ModelRouted { agent, .. }) => Some(agent.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(routed, vec!["core-specialist"]);

            // …and it precedes the InvokeParallel batch that carries
            // core-specialist (fan-out of 2 → InvokeParallel, not Invoke).
            let routed_pos = actions
                .iter()
                .position(|a| matches!(
                    a,
                    Action::Emit(ExecutionEvent::ModelRouted { agent, .. }) if agent == "core-specialist"
                ))
                .unwrap();
            let invoke_parallel_pos = actions
                .iter()
                .position(|a| {
                    matches!(
                        a,
                        Action::InvokeParallel { batch, .. }
                            if batch.iter().any(|s| s.agent == "core-specialist")
                    )
                })
                .unwrap();
            assert!(
                routed_pos < invoke_parallel_pos,
                "ModelRouted must precede the InvokeParallel it annotates"
            );
        }

        /// Build a minimal 1-delegation-then-partial-result state, used by the
        /// budget/cost/iteration guard tests: the coordinator delegates once
        /// and the subordinate returns a partial result. Each guard test wires
        /// a decider whose caps make exactly one branch of `breached_limit`
        /// trip on this state.
        fn one_partial_result_state() -> ExecutionState {
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
                    cost: 1.0,
                    model: "m".into(),
                },
            ];
            fold(&events)
        }

        // I4: max_iterations breach → Warned{max_iterations} + Complete.
        #[test]
        fn guard_max_iterations_warns_and_completes() {
            // depth (0) < max_depth (5) so the depth branch does not pre-empt;
            // 1 invocation ≥ max_iterations (1) → the iteration branch trips.
            let dec = test_decider(
                "dev-lead",
                &[("dev-lead", "concrete-model")],
                base_config(),
                5,
                1,
                None,
                None,
            );
            let state = one_partial_result_state();
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 2);
            assert!(matches!(
                &actions[0],
                Action::Emit(ExecutionEvent::Warned { code }) if code == "max_iterations"
            ));
            assert!(
                matches!(&actions[1], Action::Complete { content } if !content.trim().is_empty())
            );
        }

        // I4: token_budget breach → Warned{token_budget} + Complete. Also
        // guards the u32→u64 widening: the comparison must not truncate.
        #[test]
        fn guard_token_budget_warns_and_completes() {
            // 5 in + 5 out = 10 tokens ≥ budget (5). max_iterations high and
            // cost_limit None so only the token branch can trip.
            let dec = test_decider(
                "dev-lead",
                &[("dev-lead", "concrete-model")],
                base_config(),
                5,
                50,
                Some(5),
                None,
            );
            let state = one_partial_result_state();
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 2);
            assert!(matches!(
                &actions[0],
                Action::Emit(ExecutionEvent::Warned { code }) if code == "token_budget"
            ));
            assert!(
                matches!(&actions[1], Action::Complete { content } if !content.trim().is_empty())
            );

            // Boundary / no-truncation: a budget just above the 10 tokens
            // consumed must NOT trip the token branch (proves both sides are
            // compared as u64, not a truncated u32 cast of the state).
            let dec_ok = test_decider(
                "dev-lead",
                &[
                    ("dev-lead", "concrete-model"),
                    ("core-specialist", "concrete-model"),
                ],
                base_config(),
                5,
                50,
                Some(11),
                None,
            );
            let actions_ok = dec_ok.decide(&state);
            assert!(
                !actions_ok.iter().any(|a| matches!(
                    a,
                    Action::Emit(ExecutionEvent::Warned { code }) if code == "token_budget"
                )),
                "budget above consumption must not trip: {actions_ok:?}"
            );
        }

        // I4: cost_limit breach → Warned{cost_limit} + Complete.
        #[test]
        fn guard_cost_limit_warns_and_completes() {
            // Observed cost 1.0 ≥ cost_limit (0.5). max_iterations high and
            // token_budget None so only the cost branch can trip.
            let dec = test_decider(
                "dev-lead",
                &[("dev-lead", "concrete-model")],
                base_config(),
                5,
                50,
                None,
                Some(0.5),
            );
            let state = one_partial_result_state();
            let actions = dec.decide(&state);
            assert_eq!(actions.len(), 2);
            assert!(matches!(
                &actions[0],
                Action::Emit(ExecutionEvent::Warned { code }) if code == "cost_limit"
            ));
            assert!(
                matches!(&actions[1], Action::Complete { content } if !content.trim().is_empty())
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

    /// Tests for `HierarchicalEffectRunner` (Task 4): the sole async/impure
    /// component of the event-sourced hierarchical engine. Named
    /// `effect_runner` so `cargo test es::hierarchical::tests::effect`
    /// targets this module.
    mod effect_runner {
        use super::*;
        use crate::agent::AgentMetadata;
        use crate::orchestration::es::state::fold;
        use crate::provider::{CompletionResponse, ProviderMetadata, TokenStream};
        use std::path::PathBuf;
        use std::sync::Mutex;

        /// Minimal `Agent` for effect-runner tests: a concrete (non
        /// `latest:auto`) model by default, with a two-line system prompt so
        /// `agents_info`'s "first non-empty line" description extraction has
        /// something to bite on.
        fn test_agent(name: &str, model: &str) -> Agent {
            Agent {
                name: name.to_string(),
                source: PathBuf::from(format!("{name}.md")),
                metadata: AgentMetadata {
                    provider: "anthropic".to_string(),
                    model: Some(model.to_string()),
                    command: None,
                    args: None,
                    temperature: 0.5,
                    max_tokens: Some(256),
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
                system_prompt: format!("You are {name}, a specialist agent.\nBe concise."),
                instructions: None,
                output_format: None,
                pipeline: None,
                context: None,
            }
        }

        /// Returns a fixed response with fixed token/cost/model metrics,
        /// regardless of the request — a local test double (no reusable mock
        /// is reachable from this module without depending on the legacy
        /// engine's private test items).
        struct FixedProvider {
            content: String,
            tokens_in: u32,
            tokens_out: u32,
            cost: f64,
            model: String,
        }

        #[async_trait]
        impl Provider for FixedProvider {
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> anyhow::Result<CompletionResponse> {
                Ok(CompletionResponse {
                    content: self.content.clone(),
                    model: self.model.clone(),
                    tokens_in: self.tokens_in,
                    tokens_out: self.tokens_out,
                    cost: self.cost,
                })
            }
            async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
                anyhow::bail!("streaming not exercised by HierarchicalEffectRunner tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "fixed".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        /// Like `FixedProvider`, but records every `CompletionRequest` it
        /// receives, so tests can assert what `run_invoke` actually sent
        /// (system prompt, messages, model) — mirrors `CapturingProvider` in
        /// the legacy `core::orchestration::hierarchical` test module.
        struct CapturingProvider {
            requests: Mutex<Vec<CompletionRequest>>,
            response: String,
        }

        impl CapturingProvider {
            fn new(response: &str) -> Self {
                Self {
                    requests: Mutex::new(Vec::new()),
                    response: response.to_string(),
                }
            }

            fn requests(&self) -> Vec<CompletionRequest> {
                self.requests.lock().unwrap().clone()
            }
        }

        #[async_trait]
        impl Provider for CapturingProvider {
            async fn complete(
                &self,
                request: CompletionRequest,
            ) -> anyhow::Result<CompletionResponse> {
                let model = request.model.clone();
                self.requests.lock().unwrap().push(request);
                Ok(CompletionResponse {
                    content: self.response.clone(),
                    model,
                    tokens_in: 1,
                    tokens_out: 1,
                    cost: 0.0,
                })
            }
            async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
                anyhow::bail!("streaming not exercised by HierarchicalEffectRunner tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "capturing".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        fn run_started(agents: &[&str], input: &str) -> ExecutionEvent {
            ExecutionEvent::RunStarted {
                run_id: "r".into(),
                pattern: "hierarchical".into(),
                agents: agents.iter().map(|a| a.to_string()).collect(),
                input: input.into(),
                project: None,
                roster: Default::default(),
            }
        }

        // Step 1 (brief): fixed mock provider → AgentObserved with the
        // expected agent/content/tokens/cost/model.
        #[tokio::test]
        async fn effect_runner_invokes_provider_and_returns_observed() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert(
                "a".to_string(),
                Arc::new(FixedProvider {
                    content: "resp".to_string(),
                    tokens_in: 3,
                    tokens_out: 4,
                    cost: 0.02,
                    model: "concrete-model".to_string(),
                }),
            );
            let runner =
                HierarchicalEffectRunner::new(agents, providers, OrchestrationConfig::default());

            let state = fold(&[run_started(&["a"], "go")]);
            let ev = runner.run_invoke("a", "go", &state, 1).await.unwrap();

            match ev {
                ExecutionEvent::AgentObserved {
                    agent,
                    content,
                    tokens_in,
                    tokens_out,
                    cost,
                    model,
                } => {
                    assert_eq!(agent, "a");
                    assert_eq!(content, "resp");
                    assert_eq!(tokens_in, 3);
                    assert_eq!(tokens_out, 4);
                    assert!((cost - 0.02).abs() < 1e-9);
                    assert_eq!(model, "concrete-model");
                }
                other => panic!("expected AgentObserved, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn run_invoke_errors_for_unknown_agent() {
            let runner = HierarchicalEffectRunner::new(
                BTreeMap::new(),
                BTreeMap::new(),
                OrchestrationConfig::default(),
            );
            let state = ExecutionState::default();
            let err = runner
                .run_invoke("missing", "go", &state, 1)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("missing"));
        }

        #[tokio::test]
        async fn run_invoke_errors_when_provider_missing_for_known_agent() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            // No provider registered for "a".
            let runner = HierarchicalEffectRunner::new(
                agents,
                BTreeMap::new(),
                OrchestrationConfig::default(),
            );
            let state = ExecutionState::default();
            let err = runner.run_invoke("a", "go", &state, 1).await.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("provider") && msg.contains("'a'"),
                "expected a distinctive missing-provider message, got: {msg}"
            );
        }

        // The generic loop applies `AgentInvoked` (pushing the `user` turn)
        // into `state` before calling `run_invoke` — so in production the
        // conversation already ends with `input`. `run_invoke` must not
        // duplicate that turn when it's already the trailing message.
        #[tokio::test]
        async fn run_invoke_does_not_duplicate_an_already_applied_user_turn() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            let capturing = Arc::new(CapturingProvider::new("resp"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                HierarchicalEffectRunner::new(agents, providers, OrchestrationConfig::default());

            // Simulate the real engine loop: RunStarted, then AgentInvoked
            // (applied), before run_invoke is called.
            let state = fold(&[
                run_started(&["a"], "go"),
                ExecutionEvent::AgentInvoked {
                    agent: "a".into(),
                    input: "go".into(),
                },
            ]);
            runner.run_invoke("a", "go", &state, 1).await.unwrap();

            let sent = capturing.requests();
            assert_eq!(sent.len(), 1);
            let user_turns: Vec<&ChatMessage> = sent[0]
                .messages
                .iter()
                .filter(|m| m.role == "user")
                .collect();
            assert_eq!(
                user_turns.len(),
                1,
                "must not duplicate the already-applied user turn, got: {:?}",
                sent[0].messages
            );
        }

        // `run_invoke` called directly against a hand-built state that has
        // *not* gone through `AgentInvoked` (e.g. a unit test constructing
        // state from `RunStarted` alone) must still deliver `input` as a
        // `user` turn to the provider.
        #[tokio::test]
        async fn run_invoke_appends_input_when_not_already_in_conversation() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            let capturing = Arc::new(CapturingProvider::new("resp"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                HierarchicalEffectRunner::new(agents, providers, OrchestrationConfig::default());

            let state = fold(&[run_started(&["a"], "go")]);
            runner.run_invoke("a", "go", &state, 1).await.unwrap();

            let sent = capturing.requests();
            assert_eq!(sent.len(), 1);
            assert!(
                sent[0]
                    .messages
                    .iter()
                    .any(|m| m.role == "user" && m.content == "go"),
                "expected the input to reach the provider as a user turn, got: {:?}",
                sent[0].messages
            );
        }

        // `enriched_system_prompt` must fold in the orchestration protocol
        // block (context_injection) when hierarchical orchestration is
        // enabled, and use `agents_info` built from `self.agents` for peer
        // descriptions.
        #[tokio::test]
        async fn run_invoke_sends_enriched_system_prompt_when_hierarchical_enabled() {
            let mut agents = BTreeMap::new();
            agents.insert(
                "dev-lead".to_string(),
                test_agent("dev-lead", "concrete-model"),
            );
            agents.insert(
                "core-specialist".to_string(),
                test_agent("core-specialist", "concrete-model"),
            );
            let capturing = Arc::new(CapturingProvider::new("resp"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert(
                "dev-lead".to_string(),
                capturing.clone() as Arc<dyn Provider>,
            );

            let config = OrchestrationConfig {
                enabled: true,
                pattern: crate::orchestration::OrchestrationPattern::Hierarchical,
                coordinator: Some("dev-lead".to_string()),
                teams: vec![crate::orchestration::TeamConfig {
                    lead: None,
                    agents: vec!["core-specialist".to_string()],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let runner = HierarchicalEffectRunner::new(agents, providers, config);

            let state = fold(&[run_started(&["dev-lead"], "build X")]);
            runner
                .run_invoke("dev-lead", "build X", &state, 1)
                .await
                .unwrap();

            let sent = capturing.requests();
            assert_eq!(sent.len(), 1);
            assert!(sent[0].system_prompt.contains("You are dev-lead"));
            assert!(sent[0].system_prompt.contains("## Orchestration Protocol"));
            assert!(sent[0].system_prompt.contains("core-specialist"));
        }

        // A CONCRETE model id is passed through as-is. The `latest:*`
        // placeholders are not: `latest:auto` routes per turn
        // (`run_invoke_resolves_latest_auto_to_concrete_model`) and the
        // static tiers resolve from the string alone
        // (`run_invoke_resolves_static_latest_tier_to_concrete_model`).
        // Until #376 this test pinned `latest:pro` here and asserted the
        // provider received that literal string — it was pinning the defect.
        //
        // Also asserts `temperature`/`max_tokens` from the agent's metadata
        // reach the `CompletionRequest` unchanged (`test_agent` sets
        // `temperature: 0.5`, `max_tokens: Some(256)`).
        #[tokio::test]
        async fn run_invoke_passes_agent_model_through_verbatim() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "some-concrete-model-id"));
            let capturing = Arc::new(CapturingProvider::new("resp"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                HierarchicalEffectRunner::new(agents, providers, OrchestrationConfig::default());

            let state = fold(&[run_started(&["a"], "go")]);
            runner.run_invoke("a", "go", &state, 1).await.unwrap();

            let sent = capturing.requests();
            assert_eq!(sent[0].model, "some-concrete-model-id");
            assert_eq!(sent[0].temperature, 0.5);
            assert_eq!(sent[0].max_tokens, Some(256));
        }

        // A STATIC tier placeholder resolves to a concrete model too (#376),
        // with no `ModelRouted` event needed — its tier is known from the
        // string alone, so the state carries no routing at all here, exactly
        // as on the real path. Uncached provider name for the same
        // hermeticity reason as the `latest:auto` test below.
        #[tokio::test]
        async fn run_invoke_resolves_static_latest_tier_to_concrete_model() {
            let mut agent = test_agent("a", "latest:max");
            agent.metadata.provider = "test-only-uncached-provider".to_string();
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), agent);
            let capturing = Arc::new(CapturingProvider::new("resp"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                HierarchicalEffectRunner::new(agents, providers, OrchestrationConfig::default());

            let state = fold(&[run_started(&["a"], "go")]);
            let event = runner.run_invoke("a", "go", &state, 1).await.unwrap();

            let expected =
                fallback_model_for_tier("test-only-uncached-provider", ModelTier::Max).to_string();
            let sent = capturing.requests();
            assert_eq!(sent[0].model, expected);
            assert_ne!(sent[0].model, "latest:max");
            match event {
                ExecutionEvent::AgentObserved { model, .. } => assert_eq!(model, expected),
                other => panic!("expected AgentObserved, got {other:?}"),
            }
        }

        // `"latest:auto"` is the one placeholder the effect runner resolves
        // itself: `HierarchicalDecider` always emits `ModelRouted{agent,
        // tier, ..}` ahead of the matching `Invoke` for such an agent (see
        // `model_routed_event`), which `es::state::apply` projects into
        // `state.routed_tiers`. `run_invoke` must read that tier back
        // and turn it into a concrete model via
        // `resolve_model_for_tier` — both in the `CompletionRequest` sent to
        // the provider, and in the `model` field of the returned
        // `AgentObserved` — never leaking the literal `"latest:auto"`
        // string to either.
        //
        // The agent's provider is deliberately a name no real `models.dev`
        // cache on the machine running this test could ever contain
        // (`load_models_cached` keys its cache by provider name), so
        // `resolve_model_for_tier` is forced onto its hardcoded, pure
        // `fallback_model_for_tier` path — keeping this test hermetic and
        // independent of whatever the on-disk model registry cache happens
        // to hold (or is concurrently being refreshed to by unrelated tests)
        // on this machine.
        #[tokio::test]
        async fn run_invoke_resolves_latest_auto_to_concrete_model() {
            let mut agent = test_agent("a", "latest:auto");
            agent.metadata.provider = "test-only-uncached-provider".to_string();
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), agent);
            let capturing = Arc::new(CapturingProvider::new("resp"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                HierarchicalEffectRunner::new(agents, providers, OrchestrationConfig::default());

            let state = fold(&[
                run_started(&["a"], "go"),
                ExecutionEvent::ModelRouted {
                    agent: "a".into(),
                    tier: "Fast".into(),
                    reason: "Length".into(),
                },
            ]);
            let event = runner.run_invoke("a", "go", &state, 1).await.unwrap();

            // Deterministic, hardcoded fallback for an uncached provider —
            // see `fallback_model_for_tier`'s wildcard arm.
            let expected =
                fallback_model_for_tier("test-only-uncached-provider", ModelTier::Fast).to_string();

            let sent = capturing.requests();
            assert_eq!(sent[0].model, expected);
            assert_ne!(sent[0].model, "latest:auto");

            match event {
                ExecutionEvent::AgentObserved { model, .. } => assert_eq!(model, expected),
                other => panic!("expected AgentObserved, got {other:?}"),
            }
        }
    }

    // ── run_hierarchical_es (Task 5): end-to-end + replay determinism ──
    //
    // Exercises `run_hierarchical_es` as a whole (unlike `decide` and
    // `effect_runner` above, which drive `HierarchicalDecider` /
    // `HierarchicalEffectRunner` directly) — the same scenarios the legacy
    // `HierarchicalEngine` covers, plus a proof that `replay` reconstructs
    // an identical `ExecutionState` purely from the log, with no effect
    // re-executed.
    //
    // IMPORTANT: every agent below uses a CONCRETE model string (never
    // `latest:auto`). Resolving `latest:auto` calls
    // `resolve_model_for_tier`, which — unless the agent's `provider` is
    // deliberately absent from the cache (see the `effect_runner` tests
    // above) — reads an on-disk `models.dev` cache. That I/O is
    // non-hermetic (depends on whatever happens to be cached on the
    // machine running the test) and would make these tests flaky. Sticking
    // to concrete models keeps `HierarchicalEffectRunner::run_invoke`
    // entirely in-memory, driven only by the scripted providers below.
    use crate::agent::AgentMetadata;
    use crate::orchestration::es::engine::replay;
    use crate::orchestration::es::log::InMemoryLog;
    use crate::orchestration::es::state::RunStatus;
    use crate::provider::{CompletionResponse, ProviderMetadata, TokenStream};
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal `Agent` for `run_hierarchical_es` E2E tests: always a
    /// concrete model (see module note above).
    fn es_test_agent(name: &str, model: &str) -> Agent {
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
            system_prompt: format!("You are {name}."),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    /// Provider scripted with a fixed sequence of responses, one per call
    /// (repeating the last one for any call beyond the scripted list, so a
    /// test can under-provision without panicking). Also counts calls, so
    /// `es_replay_reconstructs_state` can prove `replay` triggers none.
    struct ScriptedProvider {
        responses: std::sync::Mutex<VecDeque<String>>,
        last: std::sync::Mutex<String>,
        calls: AtomicUsize,
    }

    impl ScriptedProvider {
        fn new(responses: &[&str]) -> Self {
            Self {
                responses: std::sync::Mutex::new(
                    responses.iter().map(|s| (*s).to_string()).collect(),
                ),
                last: std::sync::Mutex::new(String::new()),
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut queue = self.responses.lock().unwrap();
            let content = queue
                .pop_front()
                .unwrap_or_else(|| self.last.lock().unwrap().clone());
            *self.last.lock().unwrap() = content.clone();
            Ok(CompletionResponse {
                content,
                model: request.model,
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
            })
        }
        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not exercised by run_hierarchical_es tests")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "scripted".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// A provider whose every call fails — to exercise the collect-and-record
    /// path (`run_invoke` `Err` → `AgentFailed`, run continues).
    struct FailingProvider;

    #[async_trait]
    impl Provider for FailingProvider {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> anyhow::Result<CompletionResponse> {
            anyhow::bail!("simulated provider failure")
        }
        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("simulated provider failure")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "scripted".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// Base flat-team config: coordinator with a single team of `peers` (no
    /// nested lead) — the same topology as `decide`'s `base_config`.
    fn es_flat_config(coordinator: &str, peers: &[&str]) -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some(coordinator.to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: peers.iter().map(|p| (*p).to_string()).collect(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Extract the `content` of the last `Completed` event recorded for
    /// `run_id`, if any. `ExecutionState` itself carries no `content`
    /// field (only `status`), so asserting the final answer's content
    /// requires reading it back from the log.
    fn final_content(log: &InMemoryLog, run_id: &str) -> String {
        log.events(run_id)
            .unwrap()
            .into_iter()
            .rev()
            .find_map(|e| match e {
                ExecutionEvent::Completed { content } => Some(content),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Whether `run_id`'s log contains a `Warned{code}` event matching
    /// `code` exactly. `apply` treats `Warned` as a no-op on `ExecutionState`
    /// (see `es::state::apply`), so — like `final_content` above — this can
    /// only be checked against the log, not the projected state.
    fn log_has_warned(log: &InMemoryLog, run_id: &str, code: &str) -> bool {
        log.events(run_id)
            .unwrap()
            .iter()
            .any(|e| matches!(e, ExecutionEvent::Warned { code: c } if c == code))
    }

    // Scenario 1: coordinator delegates to a single agent, which answers
    // with a `FinalAnswer`; the coordinator then synthesizes its own
    // `FinalAnswer` from that single result. The run must `Complete`, the
    // delegation must be traced, and the final content must be non-empty.
    #[tokio::test]
    async fn es_single_delegation_completes() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: fais X",
                "Synthèse : tout est prêt.",
            ])),
        );
        providers.insert(
            "core-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est fait."])),
        );

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-single",
            "dev-lead",
            "build X",
            es_flat_config("dev-lead", &["core-specialist"]),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert!(!st.hier.trace.is_empty(), "expected a recorded delegation");
        assert!(
            st.hier
                .trace
                .iter()
                .any(|(from, to, _, _)| from == "dev-lead" && to == "core-specialist"),
            "expected dev-lead -> core-specialist in the trace, got {:?}",
            st.hier.trace
        );
        assert!(
            !final_content(&log, "run-single").trim().is_empty(),
            "expected non-empty final content"
        );

        let replayed = replay("run-single", &log).unwrap();
        assert_eq!(format!("{st:?}"), format!("{replayed:?}"));
    }

    /// OH1 Lot 6, Task 3: `resume_hierarchical_es` reconstructs the same
    /// `HierarchicalDecider`/`HierarchicalEffectRunner` a fresh
    /// `run_hierarchical_es` would from a log that only has `RunStarted` +
    /// `ConfigSnapshot` recorded (simulating a crash immediately after
    /// config capture, before any delegation happened) — proving
    /// `OrchestrationConfig` (including its `coordinator` field, which
    /// `resume_hierarchical_es` falls back to the roster's first entry for
    /// when absent) round-trips through `ConfigSnapshot` correctly. Same
    /// scripted scenario as `es_single_delegation_completes` above, seeded
    /// by hand instead of via `run_hierarchical_es`.
    #[tokio::test]
    async fn resume_hierarchical_es_completes_from_a_config_snapshot_only_log() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: fais X",
                "Synthèse : tout est prêt.",
            ])),
        );
        providers.insert(
            "core-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est fait."])),
        );

        let config = es_flat_config("dev-lead", &["core-specialist"]);
        let agent_names: Vec<String> = agents.keys().cloned().collect();

        let mut log = InMemoryLog::default();
        log.append(
            "run-resume-hier",
            &ExecutionEvent::RunStarted {
                run_id: "run-resume-hier".to_string(),
                pattern: "hierarchical".to_string(),
                agents: agent_names,
                input: "build X".to_string(),
                project: None,
                roster: Default::default(),
            },
        )
        .unwrap();
        log.append(
            "run-resume-hier",
            &ExecutionEvent::ConfigSnapshot {
                config_json: serde_json::to_string(&config).unwrap(),
            },
        )
        .unwrap();

        let st = resume_hierarchical_es(
            "run-resume-hier",
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert!(!st.hier.trace.is_empty(), "expected a recorded delegation");
        assert!(
            st.hier
                .trace
                .iter()
                .any(|(from, to, _, _)| from == "dev-lead" && to == "core-specialist"),
            "expected dev-lead -> core-specialist in the trace, got {:?}",
            st.hier.trace
        );
        assert!(!final_content(&log, "run-resume-hier").trim().is_empty());
    }

    /// OH1 Lot 6, Task 4: crash→resume test for `hierarchical` where the
    /// "crash" happens AFTER an agent has already been invoked and observed
    /// — unlike `resume_hierarchical_es_completes_from_a_config_snapshot_only_log`
    /// above (which crashes before any delegation at all), this is the exact
    /// scenario the task brief calls out: an already-`AgentObserved` agent
    /// must NOT be re-invoked on resume.
    ///
    /// Strategy: run the same scripted scenario as `es_single_delegation_completes`
    /// once, straight through to completion, to obtain the canonical event
    /// sequence a crash-free run produces. Then simulate a crash by
    /// truncating that sequence right after core-specialist's
    /// `AgentObserved` (dev-lead has delegated and gotten its answer back,
    /// but hasn't yet been asked to synthesize) — the truncated log's folded
    /// state is still `Running` (no terminal event recorded, verified below).
    /// Resuming from there, with a FRESH `ScriptedProvider` per agent
    /// (core-specialist's has NOTHING scripted, so any call to it would be a
    /// bug this test must catch), must reach `Completed` without ever
    /// calling core-specialist's provider again, and the resulting log must
    /// be identical event-for-event (`Debug`-format comparison, the same
    /// technique `run_event_sourced`'s own generic test and
    /// `es_replay_reconstructs_state` above use) to the canonical
    /// straight-through run — proving resume genuinely *continues* the same
    /// deterministic sequence rather than merely reaching `Completed` by
    /// some other, divergent path.
    #[tokio::test]
    async fn resume_hierarchical_es_does_not_reinvoke_an_already_observed_agent() {
        let run_id = "run-crash";
        let config = es_flat_config("dev-lead", &["core-specialist"]);

        // 1. Canonical straight-through run (never interrupted) — the
        // source of both the "pre-crash" prefix and the expected final
        // event shape.
        let mut canonical_agents = BTreeMap::new();
        canonical_agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        canonical_agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        let mut canonical_providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        canonical_providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: fais X",
                "Synthèse : tout est prêt.",
            ])),
        );
        canonical_providers.insert(
            "core-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est fait."])),
        );
        let mut canonical_log = InMemoryLog::default();
        run_hierarchical_es(
            run_id,
            "dev-lead",
            "build X",
            config.clone(),
            canonical_agents,
            canonical_providers,
            RoutingRules::default(),
            &mut canonical_log,
        )
        .await
        .unwrap();
        let canonical_events = canonical_log.events(run_id).unwrap();

        // 2. Truncate right after core-specialist's `AgentObserved` — the
        // "crash point": core-specialist has already answered, dev-lead
        // hasn't been asked to synthesize yet.
        let cut = canonical_events
            .iter()
            .position(
                |e| matches!(e, ExecutionEvent::AgentObserved { agent, .. } if agent == "core-specialist"),
            )
            .expect("expected core-specialist to be observed in the canonical run")
            + 1;
        let prefix = &canonical_events[..cut];
        assert!(
            prefix.iter().any(
                |e| matches!(e, ExecutionEvent::AgentInvoked { agent, .. } if agent == "core-specialist")
            ),
            "sanity: the crash prefix must include core-specialist's AgentInvoked"
        );
        assert!(
            cut < canonical_events.len(),
            "sanity: the crash prefix must be a strict, non-trivial prefix of the canonical run \
             (there must be remaining work — dev-lead's synthesis — for resume to do)"
        );

        let mut crashed_log = InMemoryLog::default();
        for event in prefix {
            crashed_log.append(run_id, event).unwrap();
        }
        assert_eq!(
            replay(run_id, &crashed_log).unwrap().status,
            RunStatus::Running,
            "sanity: the truncated log must still be mid-run (no terminal event recorded)"
        );

        // 3. Resume with FRESH providers: dev-lead's has only the synthesis
        // response left (it already answered the delegation question before
        // the "crash"); core-specialist's has NOTHING scripted.
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        let dev_lead_provider = Arc::new(ScriptedProvider::new(&["Synthèse : tout est prêt."]));
        let core_specialist_provider = Arc::new(ScriptedProvider::new(&[]));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            dev_lead_provider.clone() as Arc<dyn Provider>,
        );
        providers.insert(
            "core-specialist".to_string(),
            core_specialist_provider.clone() as Arc<dyn Provider>,
        );

        let st = resume_hierarchical_es(
            run_id,
            agents,
            providers,
            RoutingRules::default(),
            &mut crashed_log,
        )
        .await
        .unwrap();

        // (a) reaches Completed.
        assert_eq!(st.status, RunStatus::Completed);
        // (c) core-specialist (already observed before the crash) is never
        // re-invoked; dev-lead's only NEW call is the synthesis one.
        assert_eq!(
            core_specialist_provider.call_count(),
            0,
            "core-specialist already answered before the crash and must not be re-invoked"
        );
        assert_eq!(
            dev_lead_provider.call_count(),
            1,
            "dev-lead's synthesis call is the only new work resume should perform"
        );

        // (b) the full expected event set is present: resuming from the
        // crash point reconstructs a log event-for-event identical to the
        // canonical straight-through run.
        let resumed_events = crashed_log.events(run_id).unwrap();
        assert_eq!(
            format!("{canonical_events:?}"),
            format!("{resumed_events:?}"),
            "resume must reconstruct the exact same event log a crash-free run would have produced"
        );
        assert!(!final_content(&crashed_log, run_id).trim().is_empty());
    }

    #[tokio::test]
    async fn resume_hierarchical_es_bails_on_completed_run() {
        let mut log = InMemoryLog::default();
        log.append(
            "run-resume-hier",
            &ExecutionEvent::RunStarted {
                run_id: "run-resume-hier".to_string(),
                pattern: "hierarchical".to_string(),
                agents: vec!["dev-lead".to_string()],
                input: "build X".to_string(),
                project: None,
                roster: Default::default(),
            },
        )
        .unwrap();
        log.append(
            "run-resume-hier",
            &ExecutionEvent::Completed {
                content: "done".to_string(),
            },
        )
        .unwrap();

        let err = resume_hierarchical_es(
            "run-resume-hier",
            BTreeMap::new(),
            BTreeMap::new(),
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not resumable"));
    }

    // Scenario 2: coordinator delegates to two sibling agents in the same
    // response; both answer with `FinalAnswer`s, and the coordinator
    // synthesizes both results into its own final answer.
    #[tokio::test]
    async fn es_multiple_delegations_synthesize() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        agents.insert(
            "qa-specialist".to_string(),
            es_test_agent("qa-specialist", "concrete-model"),
        );
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: implémente X\n@qa-specialist: teste X",
                "Synthèse : fonctionnalité livrée.",
            ])),
        );
        providers.insert(
            "core-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est implémenté."])),
        );
        providers.insert(
            "qa-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est testé, RAS."])),
        );

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-multi",
            "dev-lead",
            "build X",
            es_flat_config("dev-lead", &["core-specialist", "qa-specialist"]),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        let targets: Vec<&str> = st
            .hier
            .trace
            .iter()
            .filter(|(from, ..)| from == "dev-lead")
            .map(|(_, to, _, _)| to.as_str())
            .collect();
        assert_eq!(targets, vec!["core-specialist", "qa-specialist"]);
        assert!(
            !final_content(&log, "run-multi").trim().is_empty(),
            "expected non-empty final content"
        );
    }

    // Scenario: coordinator delegates to two siblings concurrently; one child's
    // provider fails. Collect-and-record: the run still completes on the
    // surviving child, and the failure is recorded (AgentFailed) rather than
    // aborting the run.
    #[tokio::test]
    async fn es_parallel_fanout_survives_one_failed_child() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        agents.insert(
            "qa-specialist".to_string(),
            es_test_agent("qa-specialist", "concrete-model"),
        );
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&[
                "@core-specialist: implémente X\n@qa-specialist: teste X",
                "Synthèse : livré malgré un échec.",
            ])),
        );
        // core-specialist fails; qa-specialist succeeds.
        providers.insert("core-specialist".to_string(), Arc::new(FailingProvider));
        providers.insert(
            "qa-specialist".to_string(),
            Arc::new(ScriptedProvider::new(&["X est testé, RAS."])),
        );

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-partial-fail",
            "dev-lead",
            "build X",
            es_flat_config("dev-lead", &["core-specialist", "qa-specialist"]),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        // Run completed despite the failure (collect-and-record, not abort).
        assert_eq!(st.status, RunStatus::Completed);

        let events = log.events("run-partial-fail").unwrap();

        // The failed child is recorded as AgentFailed, in Vec order (core
        // before qa), and qa was still invoked and observed.
        assert!(
            events.iter().any(|e| matches!(
                e,
                ExecutionEvent::AgentFailed { agent, .. } if agent == "core-specialist"
            )),
            "expected AgentFailed for core-specialist"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                ExecutionEvent::AgentObserved { agent, .. } if agent == "qa-specialist"
            )),
            "expected qa-specialist to still be observed"
        );

        // Deterministic recorded order: both AgentInvoked in batch order.
        let invoked: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                ExecutionEvent::AgentInvoked { agent, .. }
                    if agent == "core-specialist" || agent == "qa-specialist" =>
                {
                    Some(agent.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(invoked, vec!["core-specialist", "qa-specialist"]);

        // Final content is the coordinator's synthesis (non-empty).
        assert!(
            !final_content(&log, "run-partial-fail").trim().is_empty(),
            "expected non-empty final content after synthesizing partial results"
        );
    }

    // Scenario 3: a two-level delegation chain (dev-lead -> core-lead ->
    // core-a) would reach `max_depth` (2) on its second hop. `dispatch_actions`'s
    // guard-at-source (OH1 Lot 4 Task 3, reconciliation A) must refuse to
    // dispatch that second hop at all — mirroring legacy's `invoke_agent`,
    // which bails *before* invoking a target at `depth >= max_depth` — so
    // `core-a` is never invoked (`call_count() == 0`). The run still ends
    // `Completed` with a `Warned{max_depth}` + non-empty partial digest (the
    // guard *completes* the run, it does not error), one round earlier than
    // before this reconciliation (previously `core-a` *was* invoked and the
    // halt only kicked in on the following `decide` round — see git history
    // for the pre-Task-3 version of this test).
    #[tokio::test]
    async fn es_max_depth_halts_gracefully() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-lead".to_string(),
            es_test_agent("core-lead", "concrete-model"),
        );
        agents.insert(
            "core-a".to_string(),
            es_test_agent("core-a", "concrete-model"),
        );
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            Arc::new(ScriptedProvider::new(&["@core-lead: gère la feature"])),
        );
        providers.insert(
            "core-lead".to_string(),
            Arc::new(ScriptedProvider::new(&["@core-a: fais A"])),
        );
        let core_a_provider = Arc::new(ScriptedProvider::new(&["A en cours."]));
        providers.insert(
            "core-a".to_string(),
            core_a_provider.clone() as Arc<dyn Provider>,
        );

        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![TeamConfig {
                lead: Some("core-lead".to_string()),
                agents: vec!["core-a".to_string()],
                ..Default::default()
            }],
            max_depth: Some(2),
            ..Default::default()
        };

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-depth",
            "dev-lead",
            "build X",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert!(
            log_has_warned(&log, "run-depth", "max_depth"),
            "expected a max_depth Warned event in the log"
        );
        assert_eq!(
            core_a_provider.call_count(),
            0,
            "core-a is one level too deep (depth 2 >= max_depth 2) and must never be invoked \
             — the guard-at-source halts before dispatching it"
        );
        assert!(
            !final_content(&log, "run-depth").trim().is_empty(),
            "expected a non-empty partial digest"
        );
    }

    // Scenario 4: replay determinism. After a full run, `replay(run_id,
    // &log)` must reconstruct an `ExecutionState` identical (`Debug`
    // format) to the one `run_hierarchical_es` returned — and it must do
    // so without invoking any provider again (`replay` takes no
    // `EffectRunner` at all; the call-count assertions below are an extra,
    // belt-and-braces proof that no effect silently re-runs).
    #[tokio::test]
    async fn es_replay_reconstructs_state() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "dev-lead".to_string(),
            es_test_agent("dev-lead", "concrete-model"),
        );
        agents.insert(
            "core-specialist".to_string(),
            es_test_agent("core-specialist", "concrete-model"),
        );
        let dev_lead_provider = Arc::new(ScriptedProvider::new(&[
            "@core-specialist: fais X",
            "Synthèse : tout est prêt.",
        ]));
        let core_provider = Arc::new(ScriptedProvider::new(&["X est fait."]));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert(
            "dev-lead".to_string(),
            dev_lead_provider.clone() as Arc<dyn Provider>,
        );
        providers.insert(
            "core-specialist".to_string(),
            core_provider.clone() as Arc<dyn Provider>,
        );

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-replay",
            "dev-lead",
            "build X",
            es_flat_config("dev-lead", &["core-specialist"]),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();
        assert_eq!(st.status, RunStatus::Completed);

        let dev_lead_calls_before = dev_lead_provider.call_count();
        let core_calls_before = core_provider.call_count();
        assert!(
            dev_lead_calls_before > 0,
            "expected the dev-lead provider to have been invoked during the run"
        );
        assert!(
            core_calls_before > 0,
            "expected the core-specialist provider to have been invoked during the run"
        );

        let replayed = replay("run-replay", &log).unwrap();

        assert_eq!(
            format!("{st:?}"),
            format!("{replayed:?}"),
            "replay must reconstruct an identical ExecutionState"
        );
        assert_eq!(
            dev_lead_provider.call_count(),
            dev_lead_calls_before,
            "replay must not re-invoke the dev-lead provider"
        );
        assert_eq!(
            core_provider.call_count(),
            core_calls_before,
            "replay must not re-invoke the core-specialist provider"
        );
    }

    // ── Nested C9 sub-runs (Task 10) ─────────────────────────────────
    //
    // These mirror the legacy `hierarchical::tests::
    // test_nested_blackboard_runs_and_folds_metrics` /
    // `test_nested_ring_runs_and_folds_metrics`: the coordinator delegates to
    // a lead of a team declaring a nested sub-pattern; `run_invoke`
    // short-circuits into a full event-sourced blackboard/ring sub-run on a
    // dedicated child log, and folds its outcome + aggregated metrics back
    // into the parent run as a single `AgentObserved` for the lead.

    /// Config: coordinator `dev-lead`, one team led by `core-lead` running the
    /// nested `pattern` over members `core-a`/`core-b`.
    fn nested_team_config(pattern: NestedPattern) -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![TeamConfig {
                lead: Some("core-lead".to_string()),
                agents: vec!["core-a".to_string(), "core-b".to_string()],
                pattern: Some(pattern),
                // Keep the sub-run short/deterministic (one lap/round).
                max_rounds: Some(1),
                max_laps: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// The `AgentObserved` recorded for `agent` in `run_id`'s log, if any —
    /// returns `(content, tokens_in, tokens_out, model)`.
    fn observed_for(
        log: &InMemoryLog,
        run_id: &str,
        agent: &str,
    ) -> Option<(String, u32, u32, String)> {
        log.events(run_id)
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                ExecutionEvent::AgentObserved {
                    agent: a,
                    content,
                    tokens_in,
                    tokens_out,
                    model,
                    ..
                } if a == agent => Some((content, tokens_in, tokens_out, model)),
                _ => None,
            })
    }

    /// Like `observed_for`, but reads back the `cost` field of `agent`'s
    /// `AgentObserved` — for a nested team lead, this is the folded child
    /// run's total accumulated cost (`nested_observed` sets it to
    /// `child.budget_cost`), the most direct observable proxy for the
    /// `cost_limit` ceiling `run_nested` actually handed that child (issue
    /// #345 tests below).
    fn observed_cost_for(log: &InMemoryLog, run_id: &str, agent: &str) -> Option<f64> {
        log.events(run_id)
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                ExecutionEvent::AgentObserved { agent: a, cost, .. } if a == agent => Some(cost),
                _ => None,
            })
    }

    /// Position (index) of the first `NestedStarted`/`NestedEnded` for
    /// `team_lead` in `run_id`'s log, for ordering assertions.
    fn nested_marker_positions(
        log: &InMemoryLog,
        run_id: &str,
        team_lead: &str,
    ) -> (Option<usize>, Option<usize>) {
        let events = log.events(run_id).unwrap();
        let started = events.iter().position(
            |e| matches!(e, ExecutionEvent::NestedStarted { team_lead: tl, .. } if tl == team_lead),
        );
        let ended = events.iter().position(
            |e| matches!(e, ExecutionEvent::NestedEnded { team_lead: tl } if tl == team_lead),
        );
        (started, ended)
    }

    // Scenario: coordinator delegates to a `blackboard` team lead. The nested
    // sub-run executes (both members contribute), the lead's `AgentObserved`
    // carries the board digest + aggregated metrics with `model == "nested"`,
    // the parent log records `NestedStarted` then `NestedEnded`, the parent
    // budget includes the sub-run's tokens, and replay reconstructs the state.
    #[tokio::test]
    async fn es_nested_blackboard_runs_and_folds_metrics() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a", "core-b"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        let core_a = Arc::new(ScriptedProvider::new(&[
            "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:core-a confirme",
        ]));
        let core_b = Arc::new(ScriptedProvider::new(&[
            "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:core-b confirme",
        ]));
        // `core-lead` is the arbiter, not a sub-run participant — its provider
        // must never be called on the nested path (sentinel proves the
        // short-circuit fired instead of a flat LLM call).
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), {
            Arc::new(ScriptedProvider::new(&[
                "@core-lead: gère la feature",
                "Synthèse : livré.",
            ])) as Arc<dyn Provider>
        });
        providers.insert(
            "core-lead".to_string(),
            core_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);
        providers.insert("core-b".to_string(), core_b.clone() as Arc<dyn Provider>);

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-nested-bb",
            "dev-lead",
            "build X",
            nested_team_config(NestedPattern::Blackboard),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);

        // Sub-run actually executed: both members were invoked...
        let core_a_calls_before = core_a.call_count();
        let core_b_calls_before = core_b.call_count();
        assert!(
            core_a_calls_before > 0 && core_b_calls_before > 0,
            "le sous-run doit avoir invoqué les membres pendant le run"
        );
        // ...but the lead's own provider was never called (short-circuit).
        assert_eq!(
            core_lead.call_count(),
            0,
            "lead must not make a flat LLM call"
        );

        // The lead's observed turn is the folded sub-run: board digest,
        // aggregated tokens (2 members × 1 in / 1 out), model "nested".
        let (content, tin, tout, model) =
            observed_for(&log, "run-nested-bb", "core-lead").expect("lead observation");
        assert_eq!(model, "nested");
        assert_eq!((tin, tout), (2, 2), "aggregated sub-run metrics");
        assert_eq!(
            content,
            "[core-a] core-a confirme\n[core-b] core-b confirme"
        );

        // The parent log records NestedStarted before NestedEnded.
        let (started, ended) = nested_marker_positions(&log, "run-nested-bb", "core-lead");
        assert!(
            matches!((started, ended), (Some(s), Some(e)) if s < e),
            "expected NestedStarted before NestedEnded, got {started:?}/{ended:?}"
        );

        // Parent budget includes the sub-run: dev-lead delegate (1) +
        // nested (2) + dev-lead synthesis (1) = 4 tokens in.
        assert_eq!(st.budget_tokens_in, 4);
        assert_eq!(st.budget_tokens_out, 4);

        // Sub-run stays isolated: members never leak into the parent state.
        assert!(!st.conversations.contains_key("core-a"));
        assert!(!st.conversations.contains_key("core-b"));
        // Boundary is closed in the terminal state.
        assert!(st.open_nested.is_empty());

        // Replay reconstructs the identical state from the parent log alone
        // (the child sub-run is never re-executed).
        let replayed = replay("run-nested-bb", &log).unwrap();
        assert_eq!(format!("{st:?}"), format!("{replayed:?}"));
        assert_eq!(
            core_a.call_count(),
            core_a_calls_before,
            "le replay ne doit pas ré-exécuter le sous-run (core-a)"
        );
        assert_eq!(
            core_b.call_count(),
            core_b_calls_before,
            "le replay ne doit pas ré-exécuter le sous-run (core-b)"
        );
    }

    // Scenario: same topology, `ring` sub-pattern. The nested ring circulates
    // one lap, both members vote for the same position, the outcome resolves
    // to that position, and it is folded into the lead's `AgentObserved`.
    #[tokio::test]
    async fn es_nested_ring_runs_and_folds_metrics() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a", "core-b"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        let core_a = Arc::new(ScriptedProvider::new(&[
            "ACTION: PROPOSE\nCONTENT: use Rust",
            "CONFIDENCE: 0.9\nUse Rust",
        ]));
        let core_b = Arc::new(ScriptedProvider::new(&[
            "ACTION: PROPOSE\nCONTENT: agreed, Rust",
            "CONFIDENCE: 0.8\nUse Rust",
        ]));
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), {
            Arc::new(ScriptedProvider::new(&[
                "@core-lead: gère la feature",
                "Synthèse : livré.",
            ])) as Arc<dyn Provider>
        });
        providers.insert(
            "core-lead".to_string(),
            core_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);
        providers.insert("core-b".to_string(), core_b.clone() as Arc<dyn Provider>);

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-nested-ring",
            "dev-lead",
            "build X",
            nested_team_config(NestedPattern::Ring),
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        let core_a_calls_before = core_a.call_count();
        let core_b_calls_before = core_b.call_count();
        assert!(
            core_a_calls_before > 0 && core_b_calls_before > 0,
            "le sous-run doit avoir invoqué les membres pendant le run"
        );
        assert_eq!(
            core_lead.call_count(),
            0,
            "lead must not make a flat LLM call"
        );

        // Ring outcome = resolved representative position; metrics aggregate
        // the two contributions (votes carry no tokens in the ES projection).
        let (content, tin, tout, model) =
            observed_for(&log, "run-nested-ring", "core-lead").expect("lead observation");
        assert_eq!(model, "nested");
        assert_eq!(content, "Use Rust");
        assert_eq!((tin, tout), (2, 2), "aggregated sub-run metrics");

        let (started, ended) = nested_marker_positions(&log, "run-nested-ring", "core-lead");
        assert!(
            matches!((started, ended), (Some(s), Some(e)) if s < e),
            "expected NestedStarted before NestedEnded, got {started:?}/{ended:?}"
        );

        assert_eq!(st.budget_tokens_in, 4);
        assert_eq!(st.budget_tokens_out, 4);
        assert!(!st.conversations.contains_key("core-a"));
        assert!(!st.conversations.contains_key("core-b"));
        assert!(st.open_nested.is_empty());

        let replayed = replay("run-nested-ring", &log).unwrap();
        assert_eq!(format!("{st:?}"), format!("{replayed:?}"));
        assert_eq!(
            core_a.call_count(),
            core_a_calls_before,
            "le replay ne doit pas ré-exécuter le sous-run (core-a)"
        );
        assert_eq!(
            core_b.call_count(),
            core_b_calls_before,
            "le replay ne doit pas ré-exécuter le sous-run (core-b)"
        );
    }

    // ── Issue #291: InvokeParallel must partition the remaining budget ──
    //
    // `Action::InvokeParallel` (`es::engine`) hands every entry in its batch
    // the SAME shared, immutable state snapshot (taken once, before any
    // effect runs). `run_nested` derives each nested child's token
    // allotment from that snapshot's `budget_tokens_in/out` — before the
    // fix, both children see the *entire* remaining budget and can each
    // spend up to all of it, so the combined nested consumption can be up
    // to ~batch_len times what was actually left. The fix partitions
    // equally: `remaining / batch_len`, floor. `batch_len` is `1` for the
    // ordinary sequential `Action::Invoke`, so that path is unaffected.

    /// A provider that returns a fixed response but also records every
    /// `CompletionRequest` it receives — used to read back the exact
    /// `Budget remaining: N tokens` line `BlackboardEffectRunner::build_prompt`
    /// embeds in a nested member's round-0 prompt, which is the most direct
    /// observable proxy for the `token_budget` value `run_nested` actually
    /// handed to that child's `BlackboardConfig` (reading it back through
    /// consumed tokens would require running the sub-pattern's own default
    /// budget to exhaustion — hundreds of thousands of rounds).
    struct CapturingProvider {
        requests: std::sync::Mutex<Vec<CompletionRequest>>,
        response: String,
    }

    impl CapturingProvider {
        fn new(response: &str) -> Self {
            Self {
                requests: std::sync::Mutex::new(Vec::new()),
                response: response.to_string(),
            }
        }

        fn requests(&self) -> Vec<CompletionRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for CapturingProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            let model = request.model.clone();
            self.requests.lock().unwrap().push(request);
            Ok(CompletionResponse {
                content: self.response.clone(),
                model,
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
            })
        }
        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not exercised by these tests")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "capturing".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// A provider scripted with a fixed sequence of `(response, cost)`
    /// pairs, one per call (repeating the last pair for any call beyond the
    /// scripted list, mirroring `ScriptedProvider`'s under-provisioning
    /// behaviour) — the `cost_limit` counterpart of `ScriptedProvider` /
    /// `CapturingProvider` above, neither of which can produce a nonzero
    /// cost (both hardcode `cost: 0.0`). Used by the issue #345 tests
    /// below, where `state.budget_cost` — and therefore the remaining
    /// `cost_limit` ceiling `run_nested` computes — must actually move.
    /// Responses are plain (non-`ACTION:CONFIRMATION`) content unless a
    /// test scripts otherwise, for the same reason the token-budget tests
    /// above avoid `CONFIRMATION`: it never trips convergence, so the cost
    /// guard is the only thing that can halt a nested run.
    struct CostedProvider {
        responses: std::sync::Mutex<VecDeque<(String, f64)>>,
        last: std::sync::Mutex<(String, f64)>,
        calls: AtomicUsize,
    }

    impl CostedProvider {
        fn new(responses: &[(&str, f64)]) -> Self {
            Self {
                responses: std::sync::Mutex::new(
                    responses
                        .iter()
                        .map(|(s, c)| ((*s).to_string(), *c))
                        .collect(),
                ),
                last: std::sync::Mutex::new((String::new(), 0.0)),
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Provider for CostedProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut queue = self.responses.lock().unwrap();
            let (content, cost) = queue
                .pop_front()
                .unwrap_or_else(|| self.last.lock().unwrap().clone());
            *self.last.lock().unwrap() = (content.clone(), cost);
            Ok(CompletionResponse {
                content,
                model: request.model,
                tokens_in: 1,
                tokens_out: 1,
                cost,
            })
        }
        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not exercised by these tests")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "costed".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// Config: coordinator `dev-lead` delegating, in a SINGLE response (two
    /// `@lead:` directives → one `Action::InvokeParallel`, see
    /// `dispatch_actions`), to two distinct nested-team leads, each running
    /// the Blackboard sub-pattern over a single member.
    fn two_parallel_nested_teams_config(
        token_budget: Option<u64>,
        max_rounds: Option<u32>,
    ) -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("core-lead".to_string()),
                    agents: vec!["core-a".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    max_rounds,
                    ..Default::default()
                },
                TeamConfig {
                    lead: Some("other-lead".to_string()),
                    agents: vec!["other-a".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    max_rounds,
                    ..Default::default()
                },
            ],
            token_budget,
            ..Default::default()
        }
    }

    // Scenario: dev-lead delegates to `core-lead` AND `other-lead` in the
    // same message — one `InvokeParallel` batch of 2. `config.token_budget`
    // is set; dev-lead's own opening turn (1 in / 1 out, `ScriptedProvider`'s
    // fixed cost) consumes 2 of it before the batch is dispatched, leaving
    // exactly 100. Each nested sub-run has 1 member, so every round of its
    // own budget-bound Blackboard loop costs exactly 2 tokens (1 in + 1
    // out) — chosen so both the "whole remaining budget" (bug) and the
    // "half, floor-divided" (fix) amounts are themselves exact multiples of
    // that round cost, leaving no rounding-granularity ambiguity in the
    // expected counts.
    //
    // Mutation this catches: deleting the `/ batch_len` division (reverting
    // to `total.saturating_sub(consumed)` alone) makes both children see
    // the full 100 instead of 50 each — `(50, 50)` instead of `(25, 25)`,
    // and `200 > 100` instead of `100 <= 100`. Confirmed failing against
    // master (pre-fix) before this fix was applied.
    #[tokio::test]
    async fn es_parallel_nested_batch_partitions_remaining_budget_equally() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a", "other-lead", "other-a"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        // Plain (non-`ACTION:CONFIRMATION`) content parses to `Finding`
        // (`parse_board_action`'s fallback), which never counts toward the
        // confirmation ratio `check_convergence` compares against
        // `consensus_threshold` — with `CONFIRMATION` content instead, a
        // single-member team "converges" after round 0 regardless of
        // budget, which would make this scenario exercise convergence
        // rather than the budget guard it's meant to test.
        let core_a = Arc::new(ScriptedProvider::new(&["core-a note"]));
        let other_a = Arc::new(ScriptedProvider::new(&["other-a note"]));
        // Arbiters must never take the flat-LLM-call path (sentinel proves
        // the nested short-circuit fired for both).
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let other_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));

        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), {
            Arc::new(ScriptedProvider::new(&[
                "@core-lead: gère A\n@other-lead: gère B",
                "Synthèse : livré.",
            ])) as Arc<dyn Provider>
        });
        providers.insert(
            "core-lead".to_string(),
            core_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert(
            "other-lead".to_string(),
            other_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);
        providers.insert("other-a".to_string(), other_a.clone() as Arc<dyn Provider>);

        let config = two_parallel_nested_teams_config(Some(102), Some(1_000));

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-parallel-budget",
            "dev-lead",
            "build X and Y",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            core_lead.call_count(),
            0,
            "core-lead must not make a flat LLM call"
        );
        assert_eq!(
            other_lead.call_count(),
            0,
            "other-lead must not make a flat LLM call"
        );

        let (_, core_tin, core_tout, _) =
            observed_for(&log, "run-parallel-budget", "core-lead").expect("core-lead observation");
        let (_, other_tin, other_tout, _) = observed_for(&log, "run-parallel-budget", "other-lead")
            .expect("other-lead observation");

        // Each child individually: capped at exactly half of what was left
        // (50), not the whole of it (100) — tight enough to also catch a
        // division by the wrong denominator (e.g. `max_concurrency` instead
        // of the actual batch length, which here happen to differ: 4 vs 2).
        assert_eq!(
            (core_tin, core_tout),
            (25, 25),
            "core-lead's nested sub-run must stop at half the remaining budget"
        );
        assert_eq!(
            (other_tin, other_tout),
            (25, 25),
            "other-lead's nested sub-run must stop at half the remaining budget"
        );

        // The guarantee itself, in the issue's own terms: the two children
        // dispatched in the same InvokeParallel batch must not collectively
        // spend more than what was left of the shared budget at fork time.
        let remaining_at_fork: u64 = 100; // 102 total − dev-lead's opening 1-in/1-out turn.
        let combined = u64::from(core_tin)
            + u64::from(core_tout)
            + u64::from(other_tin)
            + u64::from(other_tout);
        assert!(
            combined <= remaining_at_fork,
            "combined nested consumption ({combined}) must not exceed what was \
             remaining at fork time ({remaining_at_fork})"
        );
    }

    // Scenario: same two-parallel-leads topology, but `config.token_budget`
    // is `None`. `run_nested`'s `None` arm must keep handing each child the
    // sub-pattern's own default budget verbatim, regardless of `batch_len`
    // — the division only applies to the `Some(total)` arm. `max_rounds: 1`
    // keeps each sub-run to a single round (so the test runs fast and
    // doesn't need to spend the (huge) default budget to observe it); the
    // `Budget remaining: N tokens` line `build_prompt` puts in each nested
    // member's round-0 prompt is read back to assert on the exact budget
    // value each child actually received.
    //
    // Mutation this catches: applying the `/ batch_len` division to the
    // `None` arm too (e.g. dividing the sub-pattern default by 2) changes
    // the observed "Budget remaining" line from the full default to half
    // of it, for both children.
    #[tokio::test]
    async fn es_parallel_nested_batch_with_no_token_budget_keeps_subpattern_default() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a", "other-lead", "other-a"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        let core_a = Arc::new(CapturingProvider::new(
            "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:core-a confirme",
        ));
        let other_a = Arc::new(CapturingProvider::new(
            "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:other-a confirme",
        ));
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let other_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));

        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), {
            Arc::new(ScriptedProvider::new(&[
                "@core-lead: gère A\n@other-lead: gère B",
                "Synthèse : livré.",
            ])) as Arc<dyn Provider>
        });
        providers.insert("core-lead".to_string(), core_lead as Arc<dyn Provider>);
        providers.insert("other-lead".to_string(), other_lead as Arc<dyn Provider>);
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);
        providers.insert("other-a".to_string(), other_a.clone() as Arc<dyn Provider>);

        let config = two_parallel_nested_teams_config(None, Some(1));

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-parallel-none-budget",
            "dev-lead",
            "build X and Y",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);

        let expected_default = BlackboardConfig::default().token_budget;
        for (name, provider) in [("core-a", &core_a), ("other-a", &other_a)] {
            let requests = provider.requests();
            assert_eq!(
                requests.len(),
                1,
                "{name} must be invoked exactly once (max_rounds: 1)"
            );
            let prompt = &requests[0].messages[0].content;
            assert!(
                prompt.contains(&format!("Budget remaining: {expected_default} tokens")),
                "{name}'s nested sub-run must receive the full sub-pattern default \
                 budget ({expected_default}) when the parent sets no token_budget, \
                 batch_len notwithstanding; got prompt: {prompt}"
            );
        }
    }

    // Scenario: a SINGLE nested delegation — the ordinary sequential
    // `Action::Invoke` path (`dispatch_actions`'s `invoke_steps.len() <= 1`
    // branch), never `InvokeParallel` — with `config.token_budget` set.
    // `batch_len == 1` must make `remaining / batch_len` a no-op: the lone
    // child still gets the WHOLE remaining budget, exactly as before this
    // fix. No existing test pinned this combination (the pre-existing
    // `es_nested_blackboard_runs_and_folds_metrics` leaves `token_budget`
    // unset, so it only exercises the `None` arm on the sequential path).
    //
    // Mutation this catches: hard-coding some other divisor for the
    // sequential path (e.g. always dividing by `max_concurrency`, or a
    // stray `+ 1`/`- 1` on `batch_len`) changes the observed count away
    // from the full, unhalved remaining budget.
    #[tokio::test]
    async fn es_single_nested_delegation_gets_the_full_remaining_budget() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }
        // Plain content (see the sibling parallel-batch test above for why:
        // `ACTION:CONFIRMATION` would converge a 1-member team after round
        // 0 regardless of budget).
        let core_a = Arc::new(ScriptedProvider::new(&["core-a note"]));
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), {
            Arc::new(ScriptedProvider::new(&[
                "@core-lead: gère la feature",
                "Synthèse : livré.",
            ])) as Arc<dyn Provider>
        });
        providers.insert(
            "core-lead".to_string(),
            core_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);

        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![TeamConfig {
                lead: Some("core-lead".to_string()),
                agents: vec!["core-a".to_string()],
                pattern: Some(NestedPattern::Blackboard),
                max_rounds: Some(1_000),
                ..Default::default()
            }],
            // dev-lead's opening turn costs 2 tokens (1 in + 1 out),
            // leaving exactly 20 for core-lead's sole, sequential nested
            // sub-run.
            token_budget: Some(22),
            ..Default::default()
        };

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-single-nested-budget",
            "dev-lead",
            "build X",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            core_lead.call_count(),
            0,
            "core-lead must not make a flat LLM call"
        );

        let (_, tin, tout, _) = observed_for(&log, "run-single-nested-budget", "core-lead")
            .expect("core-lead observation");
        // 1 member x 2 tokens/round; the budget guard halts once accumulated
        // >= 20, i.e. after the 10th round — the FULL remaining budget,
        // unhalved.
        assert_eq!(
            (tin, tout),
            (10, 10),
            "a solitary nested delegation (batch_len == 1) must still get the \
             entire remaining budget, not a fraction of it"
        );
    }

    // ── Issue #345: cost_limit must be decremented AND partitioned ──
    //
    // `self.config.cost_limit` was previously handed to every nested
    // sub-run VERBATIM: never reduced by `state.budget_cost` (the cost
    // already spent) and never divided across an `InvokeParallel` batch —
    // the same defect class #291 closed for `token_budget`, left open (and
    // wider — see the sequential test below) for `cost_limit`.
    // `ScriptedProvider`/`CapturingProvider` above cannot exercise this:
    // both hardcode `cost: 0.0`, so `state.budget_cost` never moves under
    // them — `CostedProvider` (defined above) fixes that.

    // Scenario: dev-lead delegates to `core-lead` AND `other-lead` in the
    // same message — one `InvokeParallel` batch of 2 — with
    // `config.cost_limit` set. dev-lead's own opening turn costs $1.0
    // before the batch is dispatched, leaving exactly $20.0; each nested
    // sub-run has 1 member costing $1.0/round, chosen so both the "whole
    // remaining" (one mutation) and the "half" (fix) amounts are exact
    // multiples of that per-round cost — no float-rounding ambiguity in the
    // expected round counts.
    //
    // Mutation sensitivity (the issue asks for each half's own sentinel):
    // - drop the `/ batch_len` division, keep the subtraction: each child
    //   gets the WHOLE $20.0 remaining instead of $10.0 — 20 rounds instead
    //   of 10 (cost $20.0 each), combined $40.0 > the $20.0 bound.
    // - drop the `(total − state.budget_cost).max(0.0)` subtraction, keep
    //   the division: each child gets $21.0 / 2 = $10.5 instead of $10.0 —
    //   halts one round later, at 11 rounds (cumulative >= 10.5), cost
    //   $11.0 each, combined $22.0 > the $20.0 bound.
    // Both mutations are caught by the tight `(10.0, 10.0)` equality below
    // (each yields a distinct wrong value, $20.0 vs $11.0, not just "some
    // overrun"), and independently by the `combined <= remaining_at_fork`
    // guarantee-shaped assertion.
    //
    // Confirmed failing against master (pre-fix): `cost_limit` handed down
    // verbatim ($21.0 to each child, ignoring both the batch and the
    // dev-lead spend) lets each nested run go 21 rounds — combined $42.0,
    // more than double the $20.0 that was actually left at fork time.
    #[tokio::test]
    async fn es_parallel_nested_batch_partitions_remaining_cost_equally() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a", "other-lead", "other-a"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        let dev_lead = Arc::new(CostedProvider::new(&[
            ("@core-lead: gère A\n@other-lead: gère B", 1.0),
            ("Synthèse : livré.", 0.0),
        ]));
        let core_a = Arc::new(CostedProvider::new(&[("core-a note", 1.0)]));
        let other_a = Arc::new(CostedProvider::new(&[("other-a note", 1.0)]));
        // Arbiters must never take the flat-LLM-call path (sentinel proves
        // the nested short-circuit fired for both).
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let other_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));

        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), dev_lead as Arc<dyn Provider>);
        providers.insert(
            "core-lead".to_string(),
            core_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert(
            "other-lead".to_string(),
            other_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);
        providers.insert("other-a".to_string(), other_a.clone() as Arc<dyn Provider>);

        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("core-lead".to_string()),
                    agents: vec!["core-a".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    max_rounds: Some(1_000),
                    ..Default::default()
                },
                TeamConfig {
                    lead: Some("other-lead".to_string()),
                    agents: vec!["other-a".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    max_rounds: Some(1_000),
                    ..Default::default()
                },
            ],
            cost_limit: Some(21.0),
            ..Default::default()
        };

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-parallel-cost",
            "dev-lead",
            "build X and Y",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            core_lead.call_count(),
            0,
            "core-lead must not make a flat LLM call"
        );
        assert_eq!(
            other_lead.call_count(),
            0,
            "other-lead must not make a flat LLM call"
        );

        let core_cost =
            observed_cost_for(&log, "run-parallel-cost", "core-lead").expect("core-lead cost");
        let other_cost =
            observed_cost_for(&log, "run-parallel-cost", "other-lead").expect("other-lead cost");

        assert_eq!(
            (core_cost, other_cost),
            (10.0, 10.0),
            "each nested sub-run must stop at half the remaining cost ($10.0), \
             not the whole of it ($20.0) nor the unhalved total ($21.0)"
        );

        // The guarantee itself, in the issue's own terms: the two children
        // dispatched in the same InvokeParallel batch must not collectively
        // spend more than what was left of the shared cost ceiling at fork
        // time.
        let remaining_at_fork: f64 = 20.0; // $21.0 total − dev-lead's opening $1.0 turn.
        let combined = core_cost + other_cost;
        assert!(
            combined <= remaining_at_fork,
            "combined nested cost ({combined}) must not exceed what was \
             remaining at fork time ({remaining_at_fork})"
        );
    }

    // Scenario: the SEQUENTIAL half of the same defect (this issue's own
    // half, per #345 — it needs no fan-out at all to manifest, unlike
    // #291's). dev-lead delegates to `lead-1` (a solitary `Action::Invoke`,
    // `batch_len == 1`); `lead-1`'s nested run completes and folds back into
    // the parent state; only THEN, in a later, separate turn, does dev-lead
    // delegate to `lead-2` (also `batch_len == 1`). No `InvokeParallel`
    // batch is ever formed — both delegations divide by 1, a no-op — so
    // this test is deliberately insensitive to the `/ batch_len` division
    // and exercises ONLY the subtraction half, which #291's tests could not
    // (every #291 scenario is a single fork with siblings dispatched
    // together; none of them chains a SECOND delegation after the first
    // has already spent something).
    //
    // `lead-1`'s team is capped at exactly 1 round (`max_rounds: 1`), so it
    // spends a FIXED $50.0 (one call to `core-a`) regardless of whatever
    // cost ceiling it is handed — its OWN ceiling never comes into play,
    // which keeps "how much was already spent" deterministic and, at
    // $50.0 of a $100.0 total, comfortably under the point where the
    // top-level `HierarchicalDecider`'s own `cost_limit` guard (which gates
    // the PARENT's loop — out of this issue's scope, like
    // `max_depth`/`max_iterations`) would force the whole run to complete
    // before dev-lead ever gets to delegate to `lead-2`.
    //
    // `lead-2`'s team, by contrast, has a huge `max_rounds` (1_000) and a
    // small $1.0/round cost, so it is `lead-2`'s OWN cost ceiling — not
    // `max_rounds` — that halts it, making its folded-back cost a direct,
    // exact readout of the ceiling `run_nested` computed. On the FIXED
    // code that ceiling is `remaining / batch_len` = `($100.0 − $50.0) / 1`
    // = `$50.0`.
    //
    // Confirmed failing against master (pre-fix): `cost_limit` handed down
    // verbatim gives `lead-2` the full, undiminished $100.0 ceiling all
    // over again, ignoring the $50.0 `lead-1` already spent — `core-b` runs
    // twice as many rounds (100 instead of 50) and the nested cost reads
    // back as $100.0, not $50.0. Combined with `lead-1`'s $50.0, that is
    // $150.0 spent against a $100.0 limit, entirely sequentially, with zero
    // parallelism anywhere in this run — the issue's own "a chain of nested
    // delegations can consume k times the limit" made concrete.
    //
    // Mutation sensitivity: dropping the subtraction (keeping the, here
    // no-op, division) reproduces exactly that master behaviour above —
    // `lead-2`'s folded cost reads $100.0 instead of $50.0. This test
    // cannot distinguish a dropped division from a correct one (both are
    // `x / 1 == x` at `batch_len == 1`), which is exactly why the parallel
    // test above is needed as the division's own sentinel — the two halves
    // are independent and neither test can stand in for the other.
    #[tokio::test]
    async fn es_sequential_nested_chain_sees_decreasing_cost_ceiling() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "lead-1", "core-a", "lead-2", "core-b"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        let dev_lead = Arc::new(ScriptedProvider::new(&[
            "@lead-1: gère A",
            "@lead-2: gère B",
            "Synthèse : livré.",
        ]));
        let core_a = Arc::new(CostedProvider::new(&[("core-a note", 50.0)]));
        let core_b = Arc::new(CostedProvider::new(&[("core-b note", 1.0)]));
        let lead_1 = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let lead_2 = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));

        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), dev_lead as Arc<dyn Provider>);
        providers.insert("lead-1".to_string(), lead_1.clone() as Arc<dyn Provider>);
        providers.insert("lead-2".to_string(), lead_2.clone() as Arc<dyn Provider>);
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);
        providers.insert("core-b".to_string(), core_b.clone() as Arc<dyn Provider>);

        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("lead-1".to_string()),
                    agents: vec!["core-a".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    max_rounds: Some(1),
                    ..Default::default()
                },
                TeamConfig {
                    lead: Some("lead-2".to_string()),
                    agents: vec!["core-b".to_string()],
                    pattern: Some(NestedPattern::Blackboard),
                    max_rounds: Some(1_000),
                    ..Default::default()
                },
            ],
            cost_limit: Some(100.0),
            ..Default::default()
        };

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-sequential-cost-chain",
            "dev-lead",
            "build X then Y",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            lead_1.call_count(),
            0,
            "lead-1 must not make a flat LLM call"
        );
        assert_eq!(
            lead_2.call_count(),
            0,
            "lead-2 must not make a flat LLM call"
        );

        let lead1_cost =
            observed_cost_for(&log, "run-sequential-cost-chain", "lead-1").expect("lead-1 cost");
        assert_eq!(
            lead1_cost, 50.0,
            "lead-1's single forced round (max_rounds: 1) costs a fixed \
             $50.0, independent of whatever ceiling it was handed — this is \
             the deterministic \"already spent\" this test relies on for \
             lead-2's ceiling"
        );

        let lead2_cost =
            observed_cost_for(&log, "run-sequential-cost-chain", "lead-2").expect("lead-2 cost");
        assert_eq!(
            lead2_cost, 50.0,
            "lead-2's nested sub-run must halt once IT alone has spent \
             $50.0 — the $100.0 total MINUS the $50.0 lead-1 already spent \
             — not $100.0, the full, undecremented total master hands down \
             verbatim (which would let lead-2 alone spend as much as the \
             entire run was ever allowed)"
        );
    }

    // Scenario: `config.cost_limit` is `None`. `remaining_cost_limit` must
    // stay `None` regardless of `batch_len` or `state.budget_cost` — the
    // sub-pattern's own cost guard must simply stay OFF, exactly as before
    // this fix (`cost_limit: None` is explicitly called out as unaffected
    // by the issue). A single sequential nested delegation whose member
    // costs an outlandish $1,000,000.0 per round proves the guard is truly
    // off: only `max_rounds` can halt it.
    //
    // Mutation this catches: any change that turns the `None` arm into
    // `Some(0.0)` instead of `None` (e.g. an errant `.unwrap_or_default()`
    // in place of `.map(...)`) would halt `core-a` at round 0 — 0 calls
    // instead of the full 3 scripted rounds.
    #[tokio::test]
    async fn es_nested_cost_limit_none_is_unaffected() {
        let mut agents = BTreeMap::new();
        for name in ["dev-lead", "core-lead", "core-a"] {
            agents.insert(name.to_string(), es_test_agent(name, "concrete-model"));
        }

        let core_a = Arc::new(CostedProvider::new(&[("core-a note", 1_000_000.0)]));
        let core_lead = Arc::new(ScriptedProvider::new(&["LEAD-FLAT-CALL-SHOULD-NOT-HAPPEN"]));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("dev-lead".to_string(), {
            Arc::new(ScriptedProvider::new(&[
                "@core-lead: gère la feature",
                "Synthèse : livré.",
            ])) as Arc<dyn Provider>
        });
        providers.insert(
            "core-lead".to_string(),
            core_lead.clone() as Arc<dyn Provider>,
        );
        providers.insert("core-a".to_string(), core_a.clone() as Arc<dyn Provider>);

        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("dev-lead".to_string()),
            teams: vec![TeamConfig {
                lead: Some("core-lead".to_string()),
                agents: vec!["core-a".to_string()],
                pattern: Some(NestedPattern::Blackboard),
                max_rounds: Some(3),
                ..Default::default()
            }],
            cost_limit: None,
            ..Default::default()
        };

        let mut log = InMemoryLog::default();
        let st = run_hierarchical_es(
            "run-cost-limit-none",
            "dev-lead",
            "build X",
            config,
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            core_a.call_count(),
            3,
            "with no cost_limit configured, only max_rounds should halt the \
             nested sub-run — an astronomically expensive member must still \
             be allowed to run all 3 scripted rounds"
        );
    }
}
