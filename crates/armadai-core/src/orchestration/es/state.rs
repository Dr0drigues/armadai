//! Pure projection (`ExecutionState`) folded from a sequence of
//! `ExecutionEvent`s via `apply`/`fold` (OH1 Lot 1 socle).
//!
//! `apply` is a pure, total, deterministic reducer: no I/O, no clock, no
//! randomness. Given the same state and event it always produces the same
//! next state, which is what makes the event log replayable.

use std::collections::{BTreeMap, BTreeSet};

use super::event::ExecutionEvent;
use crate::provider::ChatMessage;

/// Terminal/in-flight status of an orchestration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunStatus {
    #[default]
    Running,
    Completed,
    Halted,
}

/// Hierarchical-pattern sub-state: a flat delegation trace.
///
/// Each entry is `(from, to, task, depth)`, populated by `Delegated` events.
#[derive(Debug, Clone, Default)]
pub struct HierState {
    pub trace: Vec<(String, String, String, u32)>,
}

/// A single blackboard entry, as recorded in the ES projection.
///
/// Distinct from the live `blackboard::Board` engine type — this is a
/// plain, reducible record with no runtime behavior.
#[derive(Debug, Clone, Default)]
pub struct BoardEntryRec {
    pub agent: String,
    pub round: u32,
    pub kind: String,
    pub content: String,
    pub refs: Vec<usize>,
    pub confidence: f32,
}

/// Blackboard-pattern sub-state.
#[derive(Debug, Clone, Default)]
pub struct BoardState {
    pub round: u32,
    pub entries: Vec<BoardEntryRec>,
}

/// A single ring contribution, as recorded in the ES projection.
#[derive(Debug, Clone, Default)]
pub struct ContribRec {
    pub agent: String,
    pub lap: u32,
    pub position: usize,
    pub action: String,
    pub content: String,
}

/// A single ring vote, as recorded in the ES projection.
#[derive(Debug, Clone, Default)]
pub struct VoteRec {
    pub agent: String,
    pub position: String,
    pub confidence: f32,
    pub supports: Vec<usize>,
    pub concerns: Vec<String>,
}

/// Ring-pattern sub-state.
///
/// Distinct from the live `ring::RingToken` engine type — this is a plain,
/// reducible record with no runtime behavior.
#[derive(Debug, Clone, Default)]
pub struct RingState {
    pub lap: u32,
    pub contributions: Vec<ContribRec>,
    pub votes: BTreeMap<String, VoteRec>,
}

/// Pure projection of an orchestration run, folded from its event log.
#[derive(Debug, Clone, Default)]
pub struct ExecutionState {
    pub run_id: String,
    pub pattern: String,
    pub agents: Vec<String>,
    pub conversations: BTreeMap<String, Vec<ChatMessage>>,
    pub budget_tokens_in: u64,
    pub budget_tokens_out: u64,
    pub budget_cost: f64,
    pub status: RunStatus,
    pub hier: HierState,
    pub board: BoardState,
    pub ring: RingState,
    /// The orchestration config (JSON serialized) captured from `ConfigSnapshot`.
    /// Emitted by blackboard/ring/hierarchical engines right after `RunStarted`;
    /// direct runs have no config and leave this `None`.
    pub config_json: Option<String>,
    /// Tier resolved for each `latest:auto` agent (agent name -> tier, e.g.
    /// `"Fast"`/`"Pro"`/`"Max"`), populated by `ModelRouted` events. Read by
    /// the pattern effect runners (e.g. `HierarchicalEffectRunner`) to
    /// resolve a concrete model string before invoking the provider — see
    /// `es::hierarchical::run_invoke`. Run-level (not pattern-specific):
    /// blackboard/ring patterns route models the same way. `BTreeMap` for
    /// deterministic iteration/ordering.
    pub routed_tiers: BTreeMap<String, String>,
    /// Team leads whose nested C9 sub-run boundary is currently *open*: a
    /// `NestedStarted { team_lead }` has been recorded with no matching
    /// `NestedEnded { team_lead }` yet. Populated purely by `apply`
    /// (`NestedStarted` inserts, `NestedEnded` removes), so the pure
    /// `HierarchicalDecider` can detect — without scanning the whole log —
    /// which nested boundaries still need a deferred `NestedEnded` emitted to
    /// close them (see `HierarchicalDecider::pending_nested_ended`). `BTreeSet`
    /// for deterministic iteration order. In any terminal (completed) run this
    /// set is empty — every boundary opened during the run is closed before it
    /// ends — so it never perturbs replay-vs-run `Debug` equality.
    pub open_nested: BTreeSet<String>,
}

/// Apply a single event to `state` in place.
///
/// Pure, total, deterministic: no I/O, no clock, no randomness. Every
/// variant of `ExecutionEvent` is handled explicitly; variants that don't
/// yet have a corresponding projection field (e.g. `ConsensusReached`,
/// `OutcomeResolved`) are recognized but currently no-ops — they exist in
/// the log/enum for downstream consumers and future lots, without forcing
/// premature shape decisions on `ExecutionState`.
pub fn apply(state: &mut ExecutionState, event: &ExecutionEvent) {
    match event {
        ExecutionEvent::RunStarted {
            run_id,
            pattern,
            agents,
            input: _,
            project: _,
            // Read only by the bridge (`map_execution_to_run_events`'s
            // `agent_meta`) and by replay (`run_replay.rs`), not by this
            // pure state projection — `ExecutionState` has no roster-meta
            // field of its own to fold it into.
            roster: _,
        } => {
            state.run_id = run_id.clone();
            state.pattern = pattern.clone();
            state.agents = agents.clone();
        }
        ExecutionEvent::ConfigSnapshot { config_json } => {
            state.config_json = Some(config_json.clone());
        }
        ExecutionEvent::AgentInvoked { agent, input } => {
            state
                .conversations
                .entry(agent.clone())
                .or_default()
                .push(ChatMessage {
                    role: "user".to_string(),
                    content: input.clone(),
                });
        }
        ExecutionEvent::AgentObserved {
            agent,
            content,
            tokens_in,
            tokens_out,
            cost,
            model: _,
        } => {
            state
                .conversations
                .entry(agent.clone())
                .or_default()
                .push(ChatMessage {
                    role: "assistant".to_string(),
                    content: content.clone(),
                });
            state.budget_tokens_in += u64::from(*tokens_in);
            state.budget_tokens_out += u64::from(*tokens_out);
            state.budget_cost += *cost;
        }
        ExecutionEvent::AgentFailed { agent, error } => {
            // Push an `assistant` marker so `latest_response(agent)` is
            // `Some` and the child reads as settled — otherwise a hierarchical
            // coordinator would await a child that never responds
            // (`awaiting_in_flight`). Budget counters are deliberately left
            // untouched (no successful call happened).
            state
                .conversations
                .entry(agent.clone())
                .or_default()
                .push(ChatMessage {
                    role: "assistant".to_string(),
                    content: crate::orchestration::es::event::delegation_failed_content(error),
                });
        }
        ExecutionEvent::ModelRouted { agent, tier, .. } => {
            state.routed_tiers.insert(agent.clone(), tier.clone());
        }
        ExecutionEvent::Warned { .. } => {}
        ExecutionEvent::Halted { .. } => {
            state.status = RunStatus::Halted;
        }
        ExecutionEvent::Completed { .. } => {
            state.status = RunStatus::Completed;
        }
        ExecutionEvent::Delegated {
            from,
            to,
            task,
            depth,
        } => {
            state
                .hier
                .trace
                .push((from.clone(), to.clone(), task.clone(), *depth));
        }
        ExecutionEvent::AskedPeer { from, to, question } => {
            // `AskedPeer` carries no `depth` field (unlike `Delegated`), and
            // `apply` must stay a cheap, pure projection — no traversal of
            // prior events to derive a "current depth". We record `0` as a
            // documented placeholder depth for peer-level interactions
            // (distinct from hierarchical delegation depth); consumers of
            // `hier.trace` should not read depth as meaningful for
            // `AskedPeer`/`Escalated` entries.
            state
                .hier
                .trace
                .push((from.clone(), to.clone(), question.clone(), 0));
        }
        ExecutionEvent::Escalated { from, to, message } => {
            // See `AskedPeer` above: `Escalated` has no `depth` field either,
            // so we record the same documented placeholder `0`.
            state
                .hier
                .trace
                .push((from.clone(), to.clone(), message.clone(), 0));
        }
        ExecutionEvent::Synthesized { .. } => {}
        ExecutionEvent::NestedStarted { team_lead, .. } => {
            // Open a nested C9 boundary for `team_lead`. The matching
            // `NestedEnded` (emitted in deferred fashion by
            // `HierarchicalDecider::decide`, once the lead's sub-run outcome
            // has been observed) removes it below.
            state.open_nested.insert(team_lead.clone());
        }
        ExecutionEvent::NestedEnded { team_lead } => {
            state.open_nested.remove(team_lead);
        }
        ExecutionEvent::RoundStarted { round } => {
            state.board.round = *round;
        }
        ExecutionEvent::BoardEntryAdded {
            agent,
            round,
            kind,
            content,
            refs,
            confidence,
            tokens_in,
            tokens_out,
            cost,
        } => {
            state.board.entries.push(BoardEntryRec {
                agent: agent.clone(),
                round: *round,
                kind: kind.clone(),
                content: content.clone(),
                refs: refs.clone(),
                confidence: *confidence,
            });
            state.budget_tokens_in += u64::from(*tokens_in);
            state.budget_tokens_out += u64::from(*tokens_out);
            state.budget_cost += *cost;
        }
        ExecutionEvent::ConsensusReached { .. } => {}
        ExecutionEvent::LapStarted { lap } => {
            state.ring.lap = *lap;
        }
        ExecutionEvent::ContributionAdded {
            agent,
            lap,
            position,
            action,
            content,
            tokens_in,
            tokens_out,
            cost,
        } => {
            state.ring.contributions.push(ContribRec {
                agent: agent.clone(),
                lap: *lap,
                position: *position,
                action: action.clone(),
                content: content.clone(),
            });
            state.budget_tokens_in += u64::from(*tokens_in);
            state.budget_tokens_out += u64::from(*tokens_out);
            state.budget_cost += *cost;
        }
        ExecutionEvent::VoteCast {
            agent,
            position,
            confidence,
            supports,
            concerns,
        } => {
            state.ring.votes.insert(
                agent.clone(),
                VoteRec {
                    agent: agent.clone(),
                    position: position.clone(),
                    confidence: *confidence,
                    supports: supports.clone(),
                    concerns: concerns.clone(),
                },
            );
        }
        ExecutionEvent::OutcomeResolved { .. } => {}
    }
}

/// Fold a full event log into an `ExecutionState`, starting from `Default`.
pub fn fold(events: &[ExecutionEvent]) -> ExecutionState {
    let mut state = ExecutionState::default();
    for event in events {
        apply(&mut state, event);
    }
    state
}

/// Warning code emitted when a configured budget was fed no usage at all.
pub const UNREPORTED_USAGE_CODE: &str = "budget_usage_unreported";

/// `Some(message)` when a configured `token_budget`/`cost_limit` cannot
/// possibly do its job, because the provider reported no usage whatsoever
/// for the whole run.
///
/// The budget counters here are fed exclusively by `AgentObserved`'s
/// `tokens_in`/`tokens_out`/`cost`, which come straight from
/// `CompletionResponse`. That type has no way to say "unknown": a provider
/// that does not report usage — many OpenAI-compatible gateways and local
/// runtimes simply omit the `usage` block, and `CliProvider` reports nothing
/// for a plain-text CLI — is indistinguishable from one reporting a free,
/// zero-token call. Reporting `0.0` is the right choice (inventing a median
/// price would put dollars into `armadai costs` that were never spent), but
/// downstream every zero reads as "free":
///
/// - `token_budget`/`cost_limit` never breach, so a run that should have
///   halted keeps going up to `max_iterations`/`max_depth`;
/// - the per-hop partition (`remaining = total - consumed`) hands **every**
///   nested delegation the full original ceiling — the very defect #345
///   closed, reopened through the data;
/// - `remaining_ratio` stays at `1.0`, so `budget_downgrade_ratio` never
///   engages.
///
/// None of that is unbounded — `max_iterations` (50) and `max_depth` (5) are
/// checked before the budget branches, and the socle's `MAX_ITERATIONS`
/// (500) is the outer net — but a silently inoperative limit is worth one
/// line of warning. The real fix is an `Option` for unknown usage carried
/// end to end (`CliResponse` in `json_runner.rs` already distinguishes
/// them); that reaches `CompletionResponse`, the event log and the SQLite
/// schema, and is deliberately not attempted here.
///
/// Returns `None` when no budget is configured, when any usage at all was
/// reported, or when no agent ever answered (nothing to measure). A run in
/// which every call *failed* also reports no usage; the notice is still
/// literally true there, only less interesting.
pub fn unreported_usage_warning(
    token_budget: Option<u64>,
    cost_limit: Option<f64>,
    state: &ExecutionState,
) -> Option<String> {
    let mut configured: Vec<&str> = Vec::new();
    if token_budget.is_some_and(|b| b > 0) {
        configured.push("token_budget");
    }
    if cost_limit.is_some_and(|c| c > 0.0) {
        configured.push("cost_limit");
    }
    if configured.is_empty() {
        return None;
    }

    if state.budget_tokens_in > 0 || state.budget_tokens_out > 0 || state.budget_cost > 0.0 {
        return None;
    }

    let anyone_answered = state
        .conversations
        .values()
        .flatten()
        .any(|m| m.role == "assistant");
    if !anyone_answered {
        return None;
    }

    Some(format!(
        "{} is configured, but the provider reported no usage for this run \
         (0 tokens, $0.00) — the limit can never trigger, and each nested \
         delegation receives the full ceiling instead of its share. \
         Endpoints that omit `usage` (many OpenAI-compatible gateways and \
         local runtimes) make budget limits inoperative.",
        configured.join(" and ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::es::event::ExecutionEvent as E;

    #[test]
    fn fold_common_events_builds_state() {
        let events = vec![
            E::RunStarted {
                run_id: "r1".into(),
                pattern: "hierarchical".into(),
                agents: vec!["dev-lead".into(), "core".into()],
                input: "task".into(),
                project: None,
                roster: Default::default(),
            },
            E::AgentInvoked {
                agent: "dev-lead".into(),
                input: "task".into(),
            },
            E::AgentObserved {
                agent: "dev-lead".into(),
                content: "@core: do it".into(),
                tokens_in: 10,
                tokens_out: 20,
                cost: 0.01,
                model: "m".into(),
            },
            E::Completed {
                content: "done".into(),
            },
        ];
        let st = fold(&events);
        assert_eq!(st.run_id, "r1");
        assert_eq!(st.pattern, "hierarchical");
        assert_eq!(st.agents.len(), 2);
        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(st.budget_tokens_in, 10);
        assert_eq!(st.budget_tokens_out, 20);
        assert!((st.budget_cost - 0.01).abs() < 1e-9);
        // conversation recorded for dev-lead (invoked + observed)
        assert!(!st.conversations.get("dev-lead").unwrap().is_empty());
    }

    #[test]
    fn agent_failed_pushes_assistant_marker_and_leaves_budget_untouched() {
        let mut st = ExecutionState::default();
        apply(
            &mut st,
            &E::AgentInvoked {
                agent: "b".into(),
                input: "do it".into(),
            },
        );
        apply(
            &mut st,
            &E::AgentFailed {
                agent: "b".into(),
                error: "boom".into(),
            },
        );

        let convo = st.conversations.get("b").expect("conversation exists");
        assert_eq!(convo.len(), 2);
        assert_eq!(convo[0].role, "user");
        assert_eq!(convo[1].role, "assistant");
        assert_eq!(convo[1].content, "[Delegation failed: boom]");
        // AgentFailed must not move budget counters (unlike AgentObserved).
        assert_eq!(st.budget_tokens_in, 0);
        assert_eq!(st.budget_tokens_out, 0);
        assert_eq!(st.budget_cost, 0.0);
    }

    #[test]
    fn fold_is_deterministic_and_equals_incremental_apply() {
        let events = vec![
            E::RunStarted {
                run_id: "r".into(),
                pattern: "blackboard".into(),
                agents: vec!["a".into()],
                input: "x".into(),
                project: None,
                roster: Default::default(),
            },
            E::AgentObserved {
                agent: "a".into(),
                content: "c".into(),
                tokens_in: 1,
                tokens_out: 2,
                cost: 0.0,
                model: "m".into(),
            },
            E::Halted {
                reason: "max_rounds".into(),
            },
        ];
        let a = fold(&events);
        let mut b = ExecutionState::default();
        for e in &events {
            apply(&mut b, e);
        }
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(a.status, RunStatus::Halted);
    }

    #[test]
    fn board_and_delegate_events_update_substates() {
        let events = vec![
            E::RunStarted {
                run_id: "r".into(),
                pattern: "blackboard".into(),
                agents: vec!["a".into(), "b".into()],
                input: "x".into(),
                project: None,
                roster: Default::default(),
            },
            E::RoundStarted { round: 1 },
            E::BoardEntryAdded {
                agent: "a".into(),
                round: 1,
                kind: "finding".into(),
                content: "f".into(),
                refs: vec![],
                confidence: 0.9,
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.0,
            },
        ];
        let st = fold(&events);
        assert_eq!(st.board.entries.len(), 1);
        assert_eq!(st.board.round, 1);
        assert_eq!(st.budget_tokens_in, 5);
    }

    #[test]
    fn model_routed_projects_tier_into_run_state() {
        let events = vec![
            E::RunStarted {
                run_id: "r".into(),
                pattern: "hierarchical".into(),
                agents: vec!["a".into()],
                input: "x".into(),
                project: None,
                roster: Default::default(),
            },
            E::ModelRouted {
                agent: "a".into(),
                tier: "fast".into(),
                reason: "Length".into(),
            },
        ];
        let st = fold(&events);
        assert_eq!(st.routed_tiers.get("a").map(String::as_str), Some("fast"));
    }

    #[test]
    fn asked_peer_and_escalated_are_traced() {
        let events = vec![
            E::RunStarted {
                run_id: "r".into(),
                pattern: "hierarchical".into(),
                agents: vec!["a".into(), "b".into()],
                input: "x".into(),
                project: None,
                roster: Default::default(),
            },
            E::AskedPeer {
                from: "a".into(),
                to: "b".into(),
                question: "q?".into(),
            },
            E::Escalated {
                from: "b".into(),
                to: "a".into(),
                message: "up".into(),
            },
        ];
        let st = fold(&events);
        assert_eq!(st.hier.trace.len(), 2);
        assert_eq!(st.hier.trace[0].1, "b"); // to
        assert_eq!(st.hier.trace[1].2, "up"); // message
    }

    #[test]
    fn config_snapshot_is_captured_in_state() {
        let events = vec![
            E::RunStarted {
                run_id: "r".into(),
                pattern: "blackboard".into(),
                agents: vec!["a".into()],
                input: "x".into(),
                project: None,
                roster: Default::default(),
            },
            E::ConfigSnapshot {
                config_json: "{\"max_rounds\":5}".into(),
            },
        ];
        let state = fold(&events);
        assert_eq!(state.config_json.as_deref(), Some("{\"max_rounds\":5}"));
    }
    // --- unreported usage (#374 review, I3) ---

    fn state_with(tokens_in: u64, tokens_out: u64, cost: f64) -> ExecutionState {
        let mut state = ExecutionState::default();
        state.conversations.insert(
            "lead".to_string(),
            vec![ChatMessage {
                role: "assistant".to_string(),
                content: "answered".to_string(),
            }],
        );
        state.budget_tokens_in = tokens_in;
        state.budget_tokens_out = tokens_out;
        state.budget_cost = cost;
        state
    }

    #[test]
    fn a_configured_budget_fed_no_usage_at_all_is_reported() {
        let state = state_with(0, 0, 0.0);
        let warning = unreported_usage_warning(Some(10_000), None, &state)
            .expect("a token_budget with zero reported usage must be reported");
        assert!(warning.contains("token_budget"), "{warning}");

        let warning = unreported_usage_warning(None, Some(5.0), &state)
            .expect("a cost_limit with zero reported usage must be reported");
        assert!(warning.contains("cost_limit"), "{warning}");

        let warning = unreported_usage_warning(Some(10_000), Some(5.0), &state).expect("both");
        assert!(warning.contains("token_budget"), "{warning}");
        assert!(warning.contains("cost_limit"), "{warning}");
    }

    /// The three ways this must stay quiet. Without them the notice would
    /// fire on every run that configures nothing, on every healthy run, and
    /// on a run where nothing was ever invoked.
    #[test]
    fn nothing_is_reported_when_there_is_nothing_to_report() {
        // No budget configured at all.
        assert!(unreported_usage_warning(None, None, &state_with(0, 0, 0.0)).is_none());
        // A budget of zero is not a budget.
        assert!(unreported_usage_warning(Some(0), Some(0.0), &state_with(0, 0, 0.0)).is_none());
        // Usage really was reported — any one of the three counters is enough.
        assert!(unreported_usage_warning(Some(10), None, &state_with(1, 0, 0.0)).is_none());
        assert!(unreported_usage_warning(Some(10), None, &state_with(0, 1, 0.0)).is_none());
        assert!(unreported_usage_warning(None, Some(1.0), &state_with(0, 0, 0.01)).is_none());
        // Nothing ran: no usage to miss.
        assert!(
            unreported_usage_warning(Some(10), Some(1.0), &ExecutionState::default()).is_none()
        );
    }

    /// An invoked-but-unanswered run (only the `user` turn recorded) must
    /// not warn either — otherwise the notice fires before the first
    /// provider call has had a chance to report anything.
    #[test]
    fn a_run_with_no_assistant_turn_yet_is_not_reported() {
        let mut state = ExecutionState::default();
        state.conversations.insert(
            "lead".to_string(),
            vec![ChatMessage {
                role: "user".to_string(),
                content: "go".to_string(),
            }],
        );
        assert!(unreported_usage_warning(Some(10_000), None, &state).is_none());
    }
}
