//! Pure projection (`ExecutionState`) folded from a sequence of
//! `ExecutionEvent`s via `apply`/`fold` (OH1 Lot 1 socle).
//!
//! `apply` is a pure, total, deterministic reducer: no I/O, no clock, no
//! randomness. Given the same state and event it always produces the same
//! next state, which is what makes the event log replayable.

use std::collections::BTreeMap;

use super::event::ExecutionEvent;
use crate::providers::traits::ChatMessage;

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
    /// Tier resolved for each `latest:auto` agent (agent name -> tier, e.g.
    /// `"Fast"`/`"Pro"`/`"Max"`), populated by `ModelRouted` events. Read by
    /// the pattern effect runners (e.g. `HierarchicalEffectRunner`) to
    /// resolve a concrete model string before invoking the provider — see
    /// `es::hierarchical::run_invoke`. Run-level (not pattern-specific):
    /// blackboard/ring patterns route models the same way. `BTreeMap` for
    /// deterministic iteration/ordering.
    pub routed_tiers: BTreeMap<String, String>,
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
        } => {
            state.run_id = run_id.clone();
            state.pattern = pattern.clone();
            state.agents = agents.clone();
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
        ExecutionEvent::NestedStarted { .. } => {}
        ExecutionEvent::NestedEnded { .. } => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::es::event::ExecutionEvent as E;

    #[test]
    fn fold_common_events_builds_state() {
        let events = vec![
            E::RunStarted {
                run_id: "r1".into(),
                pattern: "hierarchical".into(),
                agents: vec!["dev-lead".into(), "core".into()],
                input: "task".into(),
                project: None,
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
    fn fold_is_deterministic_and_equals_incremental_apply() {
        let events = vec![
            E::RunStarted {
                run_id: "r".into(),
                pattern: "blackboard".into(),
                agents: vec!["a".into()],
                input: "x".into(),
                project: None,
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
}
