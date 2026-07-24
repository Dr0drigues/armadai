//! Bridge from the OH1 event-sourcing socle (`ExecutionEvent`/`ExecutionState`)
//! to two consumers that must keep working once `run.rs` switches onto the
//! ES engines (OH1 Lot 5):
//!
//! 1. **Observability** (`SinkProjectingLog` + [`map_execution_to_run_events`]):
//!    projects each appended `ExecutionEvent` onto zero or more `RunEvent`s
//!    and pushes them through an [`EventSink`], so the existing headless
//!    JSONL stream and live TUI feed (both driven by `RunEvent`) keep working
//!    unchanged when the ES engines start feeding the log.
//! 2. **Hierarchical display/storage** ([`to_orchestration_result`]): extracts
//!    the legacy `OrchestrationResult` shape from a folded `ExecutionState` +
//!    its source events, so the CLI/TUI/storage code that already knows how
//!    to render/persist an `OrchestrationResult` doesn't need to change.
//!
//! This module is pure bridging plumbing — it does not wire any engine into
//! `run.rs` (that's Lot 5); it only provides the two functions/types Lot 5
//! will need.

use std::collections::BTreeMap;

use super::event::ExecutionEvent;
use super::log::EventLog;
use super::state::ExecutionState;
use crate::core::events::{EventSink, RunEvent};
use crate::core::orchestration::hierarchical::{DelegationEvent, OrchestrationResult};

/// Per-event projection: `ExecutionEvent` → zero or more `RunEvent`s, given a
/// side table of agent metadata (`agent_meta`, roster key → `(prov, model)`).
///
/// The function stays free/testable (not a method): pass an empty `agent_meta`
/// to exercise the metadata-free behavior, or a populated one to check the
/// enrichment. The only event that reads `agent_meta` is `AgentInvoked`; every
/// other arm ignores it.
///
/// This is a **fidelity subset**, not an isomorphism: several `ExecutionEvent`
/// variants have no direct `RunEvent` counterpart, or the counterpart needs
/// aggregate state (e.g. running totals) that this single-event function has
/// no access to. Known, documented gaps:
///
/// - `AgentInvoked` → `AgentStart` fills `prov`/`model` from
///   `agent_meta.get(agent)` (empty strings when the key is absent). Unlike
///   the legacy engine's direct `sink.emit` call site (which knew the provider
///   and model *before* invoking), `ExecutionEvent::AgentInvoked` only carries
///   `agent`/`input`, so the metadata is threaded in from the run's roster via
///   `agent_meta`. **Model choice**: `agent_meta` carries the agent's
///   *configured* model (`agent.metadata.model`), matching what the legacy
///   `AgentStart` emitted — NOT the effectively-resolved model when the agent
///   uses `latest:auto` (resolved later, per turn). The resolved tier is
///   already carried separately by `Route`/`ModelRouted`, so `AgentStart`
///   deliberately keeps the configured value for start/end symmetry.
/// - `Completed` → `[]` (not `Result`): `RunEvent::Result` also needs
///   aggregate `tin`/`tout`/`cost`/`agents` totals that only `ExecutionState`
///   has (via [`to_orchestration_result`]). Building the terminal `Result`
///   line is left to the future `run.rs` call site (Lot 5), which has both
///   the sink and the folded state.
/// - `Warned { code }` → `[RunEvent::Warning { code, from: None, to: None }]`:
///   makes a graceful budget/cost halt (`Warned{token_budget|cost_limit}` +
///   a partial `Complete`, emitted by the blackboard/ring/hierarchical
///   deciders' priority-1 guard) visible in the `--json` stream, where it was
///   previously silently dropped (OH1 Lot 4 Task 4, Bug B). `from`/`to` have
///   no source in `Warned{code}` alone (unlike the `deprecated_model`/
///   `routing_ignored_hierarchical` warnings emitted directly in
///   `src/cli/run.rs`, which do carry them) — `Warning`'s schema keeps them
///   as `Option` (`skip_serializing_if` on `None`) precisely so this arm can
///   omit them rather than guess.
/// - `RunStarted`, `Halted`, `AskedPeer`, `Escalated`, `Synthesized`,
///   `RoundStarted`, `ConsensusReached`, `LapStarted`,
///   `OutcomeResolved` → `[]`: no `RunEvent` equivalent is specified for this
///   lot.
///
/// **`AgentStart`/`AgentEnd` symmetry**: `AgentInvoked` (emitted by the shared
/// `es::engine` invoke loop for *every* pattern, including blackboard/ring)
/// always maps to `AgentStart`. Every event that concludes that agent's turn
/// must therefore also emit an `AgentEnd`, whichever pattern-specific event
/// carries the conclusion:
/// - hierarchical/direct: `AgentObserved` → `AgentEnd` (only, as before).
/// - blackboard: `BoardEntryAdded` → `[Board, AgentEnd]` (tokens/cost/content
///   straight from the event).
/// - ring circulation: `ContributionAdded` → `[AgentEnd]` (no ring-specific
///   `RunEvent` exists for a contribution, so `AgentEnd` is the only line).
/// - ring voting: `VoteCast` → `[Vote, AgentEnd]`. `VoteCast` carries no
///   tokens/cost (voting doesn't re-charge the budget), so `AgentEnd` gets
///   `tin: 0, tout: 0, cost: 0.0`; `content` falls back to `position` (the
///   closest thing to "what the agent produced this turn").
pub fn map_execution_to_run_events(
    e: &ExecutionEvent,
    agent_meta: &BTreeMap<String, (String, String)>,
) -> Vec<RunEvent> {
    match e {
        ExecutionEvent::AgentInvoked { agent, .. } => {
            let (prov, model) = agent_meta.get(agent).cloned().unwrap_or_default();
            vec![RunEvent::AgentStart {
                agent: agent.clone(),
                prov,
                model,
            }]
        }
        ExecutionEvent::AgentObserved {
            agent,
            content,
            tokens_in,
            tokens_out,
            cost,
            model: _,
        } => vec![RunEvent::AgentEnd {
            agent: agent.clone(),
            tin: *tokens_in,
            tout: *tokens_out,
            cost: *cost,
            content: content.clone(),
        }],
        ExecutionEvent::ModelRouted {
            agent,
            tier,
            reason,
        } => vec![RunEvent::Route {
            agent: agent.clone(),
            tier: tier.clone(),
            reason: reason.clone(),
        }],
        ExecutionEvent::Delegated { from, to, .. } => vec![RunEvent::Delegate {
            from: from.clone(),
            to: to.clone(),
        }],
        ExecutionEvent::BoardEntryAdded {
            agent,
            kind,
            content,
            tokens_in,
            tokens_out,
            cost,
            ..
        } => vec![
            RunEvent::Board {
                agent: agent.clone(),
                kind: kind.clone(),
            },
            RunEvent::AgentEnd {
                agent: agent.clone(),
                tin: *tokens_in,
                tout: *tokens_out,
                cost: *cost,
                content: content.clone(),
            },
        ],
        ExecutionEvent::ContributionAdded {
            agent,
            content,
            tokens_in,
            tokens_out,
            cost,
            ..
        } => vec![RunEvent::AgentEnd {
            agent: agent.clone(),
            tin: *tokens_in,
            tout: *tokens_out,
            cost: *cost,
            content: content.clone(),
        }],
        ExecutionEvent::VoteCast {
            agent,
            position,
            confidence,
            ..
        } => vec![
            RunEvent::Vote {
                agent: agent.clone(),
                conf: *confidence,
            },
            RunEvent::AgentEnd {
                agent: agent.clone(),
                tin: 0,
                tout: 0,
                cost: 0.0,
                content: position.clone(),
            },
        ],
        ExecutionEvent::NestedStarted { team_lead, pattern } => vec![RunEvent::NestedStart {
            team_lead: team_lead.clone(),
            pattern: pattern.clone(),
        }],
        ExecutionEvent::NestedEnded { team_lead } => vec![RunEvent::NestedEnd {
            team_lead: team_lead.clone(),
        }],
        ExecutionEvent::Warned { code } => vec![RunEvent::Warning {
            code: code.clone(),
            from: None,
            to: None,
        }],
        ExecutionEvent::Completed { .. }
        | ExecutionEvent::RunStarted { .. }
        | ExecutionEvent::ConfigSnapshot { .. }
        | ExecutionEvent::Halted { .. }
        | ExecutionEvent::AskedPeer { .. }
        | ExecutionEvent::Escalated { .. }
        | ExecutionEvent::Synthesized { .. }
        | ExecutionEvent::RoundStarted { .. }
        | ExecutionEvent::ConsensusReached { .. }
        | ExecutionEvent::LapStarted { .. }
        | ExecutionEvent::OutcomeResolved { .. } => vec![],
    }
}

/// `EventLog` decorator that appends to `inner` as normal, then projects the
/// event onto `RunEvent`s (via [`map_execution_to_run_events`]) and pushes
/// each through `sink`. `events()` delegates straight to `inner`.
///
/// `agent_meta` (roster key → `(prov, model)`) is threaded into the projection
/// so `AgentInvoked → AgentStart` carries the run's real provider/model rather
/// than empty strings. It is the single source of `AgentStart`/`AgentEnd` for
/// the ES run paths, so callers must populate it from the run's roster (via
/// [`SinkProjectingLog::with_meta`]); [`SinkProjectingLog::new`] leaves it
/// empty for call sites that don't have a roster (`AgentStart` then falls back
/// to empty `prov`/`model`, matching the pre-enrichment behavior).
pub struct SinkProjectingLog<'s, L: EventLog> {
    pub inner: L,
    pub sink: &'s dyn EventSink,
    pub agent_meta: BTreeMap<String, (String, String)>,
}

impl<'s, L: EventLog> SinkProjectingLog<'s, L> {
    /// Wrap `inner`, projecting every future `append` onto `sink` with an
    /// empty `agent_meta` (so `AgentStart` carries empty `prov`/`model`).
    pub fn new(inner: L, sink: &'s dyn EventSink) -> Self {
        Self {
            inner,
            sink,
            agent_meta: BTreeMap::new(),
        }
    }

    /// Wrap `inner` with a populated `agent_meta` (roster key → `(prov,
    /// model)`), so `AgentInvoked → AgentStart` carries the run's real
    /// provider/model. This is what the ES run paths use.
    pub fn with_meta(
        inner: L,
        sink: &'s dyn EventSink,
        agent_meta: BTreeMap<String, (String, String)>,
    ) -> Self {
        Self {
            inner,
            sink,
            agent_meta,
        }
    }
}

impl<L: EventLog> EventLog for SinkProjectingLog<'_, L> {
    fn append(&mut self, run_id: &str, event: &ExecutionEvent) -> anyhow::Result<()> {
        self.inner.append(run_id, event)?;
        for re in map_execution_to_run_events(event, &self.agent_meta) {
            self.sink.emit(&re);
        }
        Ok(())
    }

    fn events(&self, run_id: &str) -> anyhow::Result<Vec<ExecutionEvent>> {
        self.inner.events(run_id)
    }
}

/// Extract the legacy hierarchical `OrchestrationResult` shape from a folded
/// `ExecutionState` and its source `events`, for the existing display/storage
/// path.
///
/// - `content`: the last `Completed { content }` in `events`; if the run
///   never completed (e.g. halted), falls back to the last `AgentObserved`
///   content; otherwise empty.
/// - `total_tokens_in`/`total_tokens_out`: `state.budget_tokens_in/out`
///   (`u64` → `u32`, saturating via `unwrap_or(u32::MAX)`).
/// - `total_cost`: `state.budget_cost`.
/// - `trace`: `state.hier.trace` mapped 1:1 onto `DelegationEvent { from, to,
///   message: task, depth }`.
/// - `invocation_count`: number of `AgentInvoked` events in `events`.
pub fn to_orchestration_result(
    state: &ExecutionState,
    events: &[ExecutionEvent],
) -> OrchestrationResult {
    let content = events
        .iter()
        .rev()
        .find_map(|e| match e {
            ExecutionEvent::Completed { content } => Some(content.clone()),
            _ => None,
        })
        .or_else(|| {
            events.iter().rev().find_map(|e| match e {
                ExecutionEvent::AgentObserved { content, .. } => Some(content.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();

    let trace = state
        .hier
        .trace
        .iter()
        .map(|(from, to, task, depth)| DelegationEvent {
            from: from.clone(),
            to: to.clone(),
            message: task.clone(),
            depth: *depth,
        })
        .collect();

    let invocation_count = u32::try_from(
        events
            .iter()
            .filter(|e| matches!(e, ExecutionEvent::AgentInvoked { .. }))
            .count(),
    )
    .unwrap_or(u32::MAX);

    OrchestrationResult {
        content,
        trace,
        total_tokens_in: u32::try_from(state.budget_tokens_in).unwrap_or(u32::MAX),
        total_tokens_out: u32::try_from(state.budget_tokens_out).unwrap_or(u32::MAX),
        total_cost: state.budget_cost,
        invocation_count,
    }
}

/// Build the synthetic `RunEvent::RunStart` bookend for a run reconstructed
/// purely from its persisted `ExecutionEvent` log (`--replay`/`--resume`).
///
/// Neither path gets a `RunStart` "for free" the way a live run does: a live
/// run's `RunStart` is emitted by `run.rs` itself (`run_inner`/
/// `run_orchestrated`), not by the ES engine — `RunStarted` maps to `[]` in
/// [`map_execution_to_run_events`] (see its doc comment) since building the
/// CLI-shaped bookend needs context (the pre-run roster) the per-event
/// projection doesn't have. `--replay`/`--resume` read the log back with no
/// such live context either, so both synthesize it here from what IS
/// reconstructable:
///
/// - `run_id`: the id being replayed/resumed, verbatim.
/// - `v`: `1`, matching every live `RunStart`.
/// - `agents`: the folded roster (`ExecutionState::agents`, from
///   `RunStarted`).
/// - `prov`/`model`: empty strings — not reconstructable from the log alone
///   (same documented gap as replayed `AgentStart`s, see `run_replay.rs`'s
///   module doc).
/// - `in_chars`: recovered from the FIRST `ExecutionEvent::RunStarted.input`
///   found in `events` (`0` if somehow absent). Unlike `prov`/`model`, the
///   original input IS logged verbatim — `ExecutionState::apply` just
///   discards it (keeping only `agents`/`pattern` from `RunStarted`) — so
///   scanning the raw event list recovers it exactly instead of stubbing it.
pub fn synthetic_run_start(run_id: &str, agents: &[String], events: &[ExecutionEvent]) -> RunEvent {
    let in_chars = events
        .iter()
        .find_map(|e| match e {
            ExecutionEvent::RunStarted { input, .. } => Some(input.chars().count()),
            _ => None,
        })
        .unwrap_or(0);

    RunEvent::RunStart {
        run_id: run_id.to_string(),
        v: 1,
        agents: agents.to_vec(),
        prov: String::new(),
        model: String::new(),
        in_chars,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::core::orchestration::es::log::InMemoryLog;
    use crate::core::orchestration::es::state::fold;

    // ── (a) map_execution_to_run_events, per variant ────────────────

    /// Empty `agent_meta` for the arms that ignore it (everything but
    /// `AgentInvoked`) and for the metadata-free `AgentInvoked` fallback case.
    fn no_meta() -> BTreeMap<String, (String, String)> {
        BTreeMap::new()
    }

    #[test]
    fn agent_invoked_maps_to_agent_start_with_empty_prov_model_when_meta_absent() {
        let e = ExecutionEvent::AgentInvoked {
            agent: "core".into(),
            input: "do x".into(),
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [RunEvent::AgentStart { agent, prov, model }] => {
                assert_eq!(agent, "core");
                assert_eq!(prov, "");
                assert_eq!(model, "");
            }
            other => panic!("expected [AgentStart], got {other:?}"),
        }
    }

    #[test]
    fn agent_invoked_maps_to_agent_start_with_prov_model_from_meta() {
        let e = ExecutionEvent::AgentInvoked {
            agent: "core".into(),
            input: "do x".into(),
        };
        let mut meta = BTreeMap::new();
        meta.insert(
            "core".to_string(),
            ("anthropic".to_string(), "claude-x".to_string()),
        );
        let got = map_execution_to_run_events(&e, &meta);
        match &got[..] {
            [RunEvent::AgentStart { agent, prov, model }] => {
                assert_eq!(agent, "core");
                assert_eq!(prov, "anthropic");
                assert_eq!(model, "claude-x");
            }
            other => panic!("expected [AgentStart with meta], got {other:?}"),
        }
    }

    #[test]
    fn agent_observed_maps_to_agent_end() {
        let e = ExecutionEvent::AgentObserved {
            agent: "core".into(),
            content: "done".into(),
            tokens_in: 10,
            tokens_out: 20,
            cost: 0.05,
            model: "claude-x".into(),
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [
                RunEvent::AgentEnd {
                    agent,
                    tin,
                    tout,
                    cost,
                    content,
                },
            ] => {
                assert_eq!(agent, "core");
                assert_eq!(*tin, 10);
                assert_eq!(*tout, 20);
                assert!((*cost - 0.05).abs() < 1e-9);
                assert_eq!(content, "done");
            }
            other => panic!("expected [AgentEnd], got {other:?}"),
        }
    }

    #[test]
    fn model_routed_maps_to_route() {
        let e = ExecutionEvent::ModelRouted {
            agent: "a".into(),
            tier: "Max".into(),
            reason: "Tag".into(),
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [
                RunEvent::Route {
                    agent,
                    tier,
                    reason,
                },
            ] => {
                assert_eq!(agent, "a");
                assert_eq!(tier, "Max");
                assert_eq!(reason, "Tag");
            }
            other => panic!("expected [Route], got {other:?}"),
        }
    }

    #[test]
    fn delegated_maps_to_delegate_dropping_task_and_depth() {
        let e = ExecutionEvent::Delegated {
            from: "lead".into(),
            to: "core".into(),
            task: "do x".into(),
            depth: 1,
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [RunEvent::Delegate { from, to }] => {
                assert_eq!(from, "lead");
                assert_eq!(to, "core");
            }
            other => panic!("expected [Delegate], got {other:?}"),
        }
    }

    #[test]
    fn board_entry_added_maps_to_board_and_agent_end() {
        let e = ExecutionEvent::BoardEntryAdded {
            agent: "a".into(),
            round: 1,
            kind: "finding".into(),
            content: "c".into(),
            refs: vec![],
            confidence: 0.9,
            tokens_in: 1,
            tokens_out: 2,
            cost: 0.03,
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [
                RunEvent::Board { agent, kind },
                RunEvent::AgentEnd {
                    agent: end_agent,
                    tin,
                    tout,
                    cost,
                    content,
                },
            ] => {
                assert_eq!(agent, "a");
                assert_eq!(kind, "finding");
                assert_eq!(end_agent, "a");
                assert_eq!(*tin, 1);
                assert_eq!(*tout, 2);
                assert!((*cost - 0.03).abs() < 1e-9);
                assert_eq!(content, "c");
            }
            other => panic!("expected [Board, AgentEnd], got {other:?}"),
        }
    }

    #[test]
    fn contribution_added_maps_to_agent_end_only() {
        let e = ExecutionEvent::ContributionAdded {
            agent: "a".into(),
            lap: 1,
            position: 0,
            action: "propose".into(),
            content: "c".into(),
            tokens_in: 4,
            tokens_out: 5,
            cost: 0.06,
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [
                RunEvent::AgentEnd {
                    agent,
                    tin,
                    tout,
                    cost,
                    content,
                },
            ] => {
                assert_eq!(agent, "a");
                assert_eq!(*tin, 4);
                assert_eq!(*tout, 5);
                assert!((*cost - 0.06).abs() < 1e-9);
                assert_eq!(content, "c");
            }
            other => panic!("expected [AgentEnd], got {other:?}"),
        }
    }

    #[test]
    fn vote_cast_maps_to_vote_and_agent_end_using_confidence_and_position() {
        let e = ExecutionEvent::VoteCast {
            agent: "r".into(),
            position: "approve".into(),
            confidence: 0.8,
            supports: vec![],
            concerns: vec![],
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [
                RunEvent::Vote { agent, conf },
                RunEvent::AgentEnd {
                    agent: end_agent,
                    tin,
                    tout,
                    cost,
                    content,
                },
            ] => {
                assert_eq!(agent, "r");
                assert!((*conf - 0.8).abs() < 1e-6);
                assert_eq!(end_agent, "r");
                assert_eq!(*tin, 0);
                assert_eq!(*tout, 0);
                assert_eq!(*cost, 0.0);
                assert_eq!(content, "approve");
            }
            other => panic!("expected [Vote, AgentEnd], got {other:?}"),
        }
    }

    /// Regression test for the observability fidelity fix: blackboard/ring
    /// turns must emit `AgentEnd` symmetric to the `AgentStart` produced by
    /// the shared `AgentInvoked` event, exactly like hierarchical/direct do
    /// via `AgentObserved`. Before the fix, `BoardEntryAdded`/`VoteCast`
    /// mapped to `[Board]`/`[Vote]` only, leaving `AgentStart` unmatched.
    #[test]
    fn board_and_ring_turns_are_symmetric_with_agent_start() {
        let invoked = ExecutionEvent::AgentInvoked {
            agent: "a".into(),
            input: "x".into(),
        };
        let board = ExecutionEvent::BoardEntryAdded {
            agent: "a".into(),
            round: 1,
            kind: "finding".into(),
            content: "c".into(),
            refs: vec![],
            confidence: 0.5,
            tokens_in: 1,
            tokens_out: 1,
            cost: 0.0,
        };
        let vote = ExecutionEvent::VoteCast {
            agent: "a".into(),
            position: "approve".into(),
            confidence: 0.5,
            supports: vec![],
            concerns: vec![],
        };

        for concluding in [board, vote] {
            let mut starts = 0usize;
            let mut ends = 0usize;
            for re in map_execution_to_run_events(&invoked, &no_meta())
                .into_iter()
                .chain(map_execution_to_run_events(&concluding, &no_meta()))
            {
                match re {
                    RunEvent::AgentStart { .. } => starts += 1,
                    RunEvent::AgentEnd { .. } => ends += 1,
                    _ => {}
                }
            }
            assert_eq!(starts, 1, "expected exactly one AgentStart");
            assert_eq!(
                ends, 1,
                "expected exactly one AgentEnd matching the AgentStart"
            );
        }
    }

    #[test]
    fn nested_started_maps_to_nested_start() {
        let e = ExecutionEvent::NestedStarted {
            team_lead: "lead".into(),
            pattern: "blackboard".into(),
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [RunEvent::NestedStart { team_lead, pattern }] => {
                assert_eq!(team_lead, "lead");
                assert_eq!(pattern, "blackboard");
            }
            other => panic!("expected [NestedStart], got {other:?}"),
        }
    }

    #[test]
    fn nested_ended_maps_to_nested_end() {
        let e = ExecutionEvent::NestedEnded {
            team_lead: "lead".into(),
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [RunEvent::NestedEnd { team_lead }] => assert_eq!(team_lead, "lead"),
            other => panic!("expected [NestedEnd], got {other:?}"),
        }
    }

    #[test]
    fn completed_has_no_run_event_equivalent_here() {
        let e = ExecutionEvent::Completed {
            content: "done".into(),
        };
        assert!(map_execution_to_run_events(&e, &no_meta()).is_empty());
    }

    /// Regression test for Bug B (OH1 Lot 4 Task 4): a graceful budget/cost
    /// halt (`Warned{code}`) must surface as a `RunEvent::Warning` — before
    /// the fix it silently mapped to `[]`, making `--json` consumers blind to
    /// the halt (they'd just see a `result` with partial content and exit 0,
    /// no indication *why* the run stopped early).
    #[test]
    fn warned_maps_to_warning_with_code_and_no_from_to() {
        let e = ExecutionEvent::Warned {
            code: "token_budget".into(),
        };
        let got = map_execution_to_run_events(&e, &no_meta());
        match &got[..] {
            [RunEvent::Warning { code, from, to }] => {
                assert_eq!(code, "token_budget");
                assert_eq!(*from, None);
                assert_eq!(*to, None);
            }
            other => panic!("expected [Warning], got {other:?}"),
        }
    }

    #[test]
    fn events_without_observability_equivalent_map_to_empty() {
        let no_ops = vec![
            ExecutionEvent::RunStarted {
                run_id: "r".into(),
                pattern: "hierarchical".into(),
                agents: vec![],
                input: "x".into(),
                project: None,
            },
            ExecutionEvent::Halted {
                reason: "budget".into(),
            },
            ExecutionEvent::AskedPeer {
                from: "a".into(),
                to: "b".into(),
                question: "q".into(),
            },
            ExecutionEvent::Escalated {
                from: "a".into(),
                to: "b".into(),
                message: "m".into(),
            },
            ExecutionEvent::Synthesized {
                agent: "a".into(),
                content: "c".into(),
            },
            ExecutionEvent::RoundStarted { round: 1 },
            ExecutionEvent::ConsensusReached { score: 0.9 },
            ExecutionEvent::LapStarted { lap: 1 },
            ExecutionEvent::OutcomeResolved {
                outcome: "x".into(),
            },
        ];
        for e in &no_ops {
            assert!(
                map_execution_to_run_events(e, &no_meta()).is_empty(),
                "expected empty mapping for {e:?}"
            );
        }
    }

    // ── (b) SinkProjectingLog ────────────────────────────────────────

    #[derive(Default)]
    struct CaptureSink {
        tags: Mutex<Vec<String>>,
    }

    impl EventSink for CaptureSink {
        fn emit(&self, ev: &RunEvent) {
            let v = serde_json::to_value(ev).expect("RunEvent always serializes");
            self.tags
                .lock()
                .unwrap()
                .push(v["t"].as_str().unwrap().to_string());
        }
    }

    #[test]
    fn sink_projecting_log_emits_run_events_in_append_order_and_delegates_reads() {
        let sink = CaptureSink::default();
        let mut log = SinkProjectingLog::new(InMemoryLog::default(), &sink);

        let e1 = ExecutionEvent::AgentInvoked {
            agent: "core".into(),
            input: "x".into(),
        };
        let e2 = ExecutionEvent::AgentObserved {
            agent: "core".into(),
            content: "done".into(),
            tokens_in: 1,
            tokens_out: 2,
            cost: 0.0,
            model: "m".into(),
        };
        // `Completed` has no `RunEvent` equivalent (see mapping docs) — it
        // must still be appended to `inner` but must not reach the sink.
        let e3 = ExecutionEvent::Completed {
            content: "done".into(),
        };

        log.append("r1", &e1).unwrap();
        log.append("r1", &e2).unwrap();
        log.append("r1", &e3).unwrap();

        assert_eq!(*sink.tags.lock().unwrap(), vec!["agent_start", "agent_end"]);

        let got = log.events("r1").unwrap();
        assert_eq!(got.len(), 3);
        assert!(matches!(got[0], ExecutionEvent::AgentInvoked { .. }));
        assert!(matches!(got[1], ExecutionEvent::AgentObserved { .. }));
        assert!(matches!(got[2], ExecutionEvent::Completed { .. }));
        assert!(log.events("absent").unwrap().is_empty());
    }

    // ── (c) to_orchestration_result ──────────────────────────────────

    #[test]
    fn to_orchestration_result_extracts_content_tokens_trace_invocation_count() {
        let events = vec![
            ExecutionEvent::RunStarted {
                run_id: "r".into(),
                pattern: "hierarchical".into(),
                agents: vec!["lead".into(), "core".into()],
                input: "task".into(),
                project: None,
            },
            ExecutionEvent::AgentInvoked {
                agent: "lead".into(),
                input: "task".into(),
            },
            ExecutionEvent::AgentObserved {
                agent: "lead".into(),
                content: "@core: do x".into(),
                tokens_in: 10,
                tokens_out: 20,
                cost: 0.01,
                model: "m".into(),
            },
            ExecutionEvent::Delegated {
                from: "lead".into(),
                to: "core".into(),
                task: "do x".into(),
                depth: 1,
            },
            ExecutionEvent::AgentInvoked {
                agent: "core".into(),
                input: "do x".into(),
            },
            ExecutionEvent::AgentObserved {
                agent: "core".into(),
                content: "x done".into(),
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.02,
                model: "m".into(),
            },
            ExecutionEvent::Completed {
                content: "final answer".into(),
            },
        ];
        let state = fold(&events);
        let result = to_orchestration_result(&state, &events);

        assert_eq!(result.content, "final answer");
        assert_eq!(result.total_tokens_in, 15);
        assert_eq!(result.total_tokens_out, 25);
        assert!((result.total_cost - 0.03).abs() < 1e-9);
        assert_eq!(result.invocation_count, 2);
        assert_eq!(result.trace.len(), 1);
        assert_eq!(result.trace[0].from, "lead");
        assert_eq!(result.trace[0].to, "core");
        assert_eq!(result.trace[0].message, "do x");
        assert_eq!(result.trace[0].depth, 1);
    }

    #[test]
    fn to_orchestration_result_falls_back_to_last_agent_observed_without_completed() {
        let events = vec![
            ExecutionEvent::AgentInvoked {
                agent: "a".into(),
                input: "x".into(),
            },
            ExecutionEvent::AgentObserved {
                agent: "a".into(),
                content: "partial".into(),
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
                model: "m".into(),
            },
            ExecutionEvent::Halted {
                reason: "budget".into(),
            },
        ];
        let state = fold(&events);
        let result = to_orchestration_result(&state, &events);
        assert_eq!(result.content, "partial");
        assert_eq!(result.invocation_count, 1);
    }

    #[test]
    fn to_orchestration_result_empty_content_when_no_completed_or_observed() {
        let events = vec![ExecutionEvent::RunStarted {
            run_id: "r".into(),
            pattern: "hierarchical".into(),
            agents: vec!["a".into()],
            input: "x".into(),
            project: None,
        }];
        let state = fold(&events);
        let result = to_orchestration_result(&state, &events);
        assert_eq!(result.content, "");
        assert_eq!(result.invocation_count, 0);
        assert!(result.trace.is_empty());
    }
}
