//! ES-native storage recording + display helpers for blackboard/ring runs
//! (OH1 Lot 4).
//!
//! These functions read the pure `ExecutionState` projection (see
//! `armadai_core::orchestration::es::state`) instead of the live
//! `blackboard::Board` / `ring::RingToken` engine types that the legacy
//! `record_orchestration_blackboard_into` / `record_orchestration_ring_into`
//! (in `run.rs`) consume. They reuse the same low-level `insert_*` storage
//! functions from `storage::queries`, so the schema and downstream readers
//! (history, TUI, web) don't need to change.
//!
//! Wired into `run.rs`'s execution path since OH1 Lot 5 (the bascule): the
//! standalone blackboard/ring match arms in `run_orchestrated` call these
//! instead of the legacy `record_orchestration_blackboard`/
//! `record_orchestration_ring` (now dead code, kept for the historical
//! record per the bascule's brief). The legacy `record_orchestration_*_into`
//! functions (`_into`, singular parent+children shape) remain alive too —
//! they're still reachable from `record_hierarchical_into`'s nested-run
//! persistence.
//!
//! ## Documented regressions vs. the legacy path
//!
//! - **Per-entry / per-contribution tokens are always `0`.** `BoardEntryRec`
//!   and `ContribRec` (the ES projection types in `es::state`) don't carry a
//!   per-entry token count — only the run-level
//!   `ExecutionState::budget_tokens_in/out` aggregate is tracked (folded
//!   from `BoardEntryAdded`/`ContributionAdded` events). Legacy's
//!   `board_entries.tokens_in/out` / `ring_contributions.tokens_in/out`
//!   columns are populated from the live engine's per-entry `Tokens`
//!   accounting, which the ES projection doesn't (yet) carry per-entry.
//! - **`outcome_json` and `halt_reason` are always `None`.** Legacy
//!   serializes the live, typed `BoardState`/`TokenStatus` enum (which
//!   carries a structured halt reason / outcome). The ES projection types
//!   (`BoardState`, `RingState`) don't derive `Serialize` and record no
//!   halt-reason field on `ExecutionState` itself (only the event log does,
//!   via `Halted { reason }`) — these `record_*_es_into` functions take
//!   `state` only (no `events`), so reconstructing that text is left to a
//!   future lot if needed.
//! - **`ring_contributions.reactions_json` is always `"[]"`.** `ContribRec`
//!   carries no reactions field (unlike the legacy `Contribution`).

// `BlackboardConfig`/`RingConfig`/`RunStatus` are only referenced from the
// `#[cfg(feature = "storage")]` record functions below (and from tests) —
// under `--features tui` (no `storage`, no `test`) they'd otherwise be
// flagged as unused imports.
#[allow(unused_imports)]
use armadai_core::orchestration::blackboard::BlackboardConfig;
use armadai_core::orchestration::es::event::ExecutionEvent;
use armadai_core::orchestration::es::state::ExecutionState;
#[allow(unused_imports)]
use armadai_core::orchestration::es::state::RunStatus;
#[allow(unused_imports)]
use armadai_core::orchestration::ring::RingConfig;

/// Concatenated `[agent] content` display for every blackboard entry, in
/// insertion order. Reproduces the legacy blackboard outcome text (see
/// `run.rs`: `board.entries().iter().map(|e| format!("[{}] {}", e.agent,
/// e.content))`).
pub fn blackboard_display(state: &ExecutionState) -> String {
    state
        .board
        .entries
        .iter()
        .map(|entry| format!("[{}] {}", entry.agent, entry.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Readable summary of a ring run from the ES projection: the resolved
/// outcome (last `OutcomeResolved { outcome }` in `events`, falling back to
/// the last `Completed { content }` — the same fallback order as
/// [`armadai_core::orchestration::es::bridge::to_orchestration_result`]),
/// plus a per-agent vote tally when any votes were cast.
///
/// Reproduces the *spirit* of the legacy `TokenStatus::Done { outcome }`
/// match in `run.rs` (consensus/majority/no-consensus wording) without
/// reconstructing the typed `RingOutcome` variants — those carry
/// engine-computed scores/dissents that the ES projection doesn't track;
/// the plain `outcome` string plus vote tally is the readable equivalent
/// available from the event-sourced state.
pub fn ring_display(state: &ExecutionState, events: &[ExecutionEvent]) -> String {
    let outcome = events
        .iter()
        .rev()
        .find_map(|e| match e {
            ExecutionEvent::OutcomeResolved { outcome } => Some(outcome.clone()),
            _ => None,
        })
        .or_else(|| {
            events.iter().rev().find_map(|e| match e {
                ExecutionEvent::Completed { content } => Some(content.clone()),
                _ => None,
            })
        })
        .unwrap_or_default();

    if state.ring.votes.is_empty() {
        return outcome;
    }

    let votes = state
        .ring
        .votes
        .values()
        .map(|v| format!("{} {} ({:.0}%)", v.agent, v.position, v.confidence * 100.0))
        .collect::<Vec<_>>()
        .join(", ");

    format!("{outcome}\n[votes] {votes}")
}

/// Single source of truth for a run's terminal `RunEvent::Result.content`,
/// branching on the folded `state.pattern` exactly the way `resume_run`
/// (`src/cli/run.rs`) used to inline it: `blackboard` and `ring` need their
/// own display helpers (the ring vote tally in particular has no equivalent
/// in the generic `OrchestrationResult` shape), everything else — `direct`,
/// `hierarchical`, and any unexpected pattern — falls back to
/// `to_orchestration_result`.
///
/// Factored out (re-review fix) so `resume_run` and `--replay`
/// (`crate::cli::run_replay::replay_from_log`) call the SAME branch instead
/// of each hand-rolling it: before this, `replay_from_log` used
/// `to_orchestration_result` unconditionally for every pattern, so a
/// replayed `ring` run silently dropped the `[votes] …` tally that live/
/// `--resume` both include — replay diverged from live for exactly the
/// pattern whose whole point is the vote. Routing both call sites through
/// one function makes that drift structurally impossible to reintroduce.
///
/// Both current callers (`resume_run`, `replay_from_log`) only exist under
/// `#[cfg(feature = "storage")]` (the event log this whole read-back/resume
/// story depends on is only persisted with that feature), so this is gated
/// the same way — unlike `blackboard_display`/`ring_display` above, which
/// stay ungated because the LIVE orchestrated path also calls them
/// unconditionally.
#[cfg(feature = "storage")]
pub(crate) fn final_content(state: &ExecutionState, events: &[ExecutionEvent]) -> String {
    match state.pattern.as_str() {
        "blackboard" => blackboard_display(state),
        "ring" => ring_display(state, events),
        _ => {
            armadai_core::orchestration::es::bridge::to_orchestration_result(state, events).content
        }
    }
}

/// Plain-text summary of ring contributions (`[agent] action: content`, in
/// insertion order). Used as the `runs.output` diagnostic column in
/// [`record_ring_es_into`] — unlike [`ring_display`], it needs no `events`
/// (the parent `events` slice isn't threaded into `record_*_es_into`, see
/// module docs), so it summarizes what `state` alone can offer.
#[cfg(feature = "storage")]
fn ring_contributions_text(state: &ExecutionState) -> String {
    state
        .ring
        .contributions
        .iter()
        .map(|c| format!("[{}] {}: {}", c.agent, c.action, c.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Persist a blackboard orchestration run from its ES projection: the
/// parent `runs` row, `orchestration_runs` metadata, and one `board_entries`
/// row per `state.board.entries` entry. Mirrors
/// `record_orchestration_blackboard_into`'s shape/column choices — reusing
/// the same low-level `insert_run_with_id`/`insert_orchestration_run`/
/// `insert_board_entry` functions — but reads `ExecutionState` instead of a
/// live `blackboard::Board`. Returns the provided `run_id`.
#[cfg(feature = "storage")]
pub fn record_blackboard_es_into(
    db: &armadai_storage::Database,
    run_id: &str,
    state: &ExecutionState,
    config: &BlackboardConfig,
    input: &str,
    parent_run_id: Option<&str>,
    project: Option<&str>,
) -> anyhow::Result<String> {
    use armadai_storage::queries;

    let status = match state.status {
        RunStatus::Halted => "halted",
        RunStatus::Completed => "success",
        RunStatus::Running => "running",
    };

    let parent = queries::RunRecord {
        agent: "orchestration:blackboard".to_string(),
        input: input.to_string(),
        output: blackboard_display(state),
        provider: "orchestration".to_string(),
        model: String::new(),
        tokens_in: i64::from(u32::try_from(state.budget_tokens_in).unwrap_or(u32::MAX)),
        tokens_out: i64::from(u32::try_from(state.budget_tokens_out).unwrap_or(u32::MAX)),
        cost: state.budget_cost,
        duration_ms: 0,
        status: status.to_string(),
        project: project.map(str::to_string),
    };
    queries::insert_run_with_id(db, run_id, parent)?;

    let orch = queries::OrchestrationRunRecord {
        run_id: run_id.to_string(),
        pattern: "blackboard".to_string(),
        config_json: serde_json::to_string(config).unwrap_or_default(),
        // No ES-native equivalent of the legacy typed `BoardState` outcome —
        // documented regression, see module docs.
        outcome_json: None,
        rounds: i64::from(state.board.round),
        // See module docs: halt reason lives on the event log, not `state`.
        halt_reason: None,
        parent_run_id: parent_run_id.map(str::to_string),
    };
    queries::insert_orchestration_run(db, orch)?;

    for entry in &state.board.entries {
        let record = queries::BoardEntryRecord {
            run_id: run_id.to_string(),
            agent: entry.agent.clone(),
            round: i64::from(entry.round),
            kind: entry.kind.clone(),
            content: entry.content.clone(),
            refs_json: serde_json::to_string(&entry.refs).unwrap_or_default(),
            confidence: f64::from(entry.confidence),
            // Per-entry tokens: documented regression, see module docs.
            tokens_in: 0,
            tokens_out: 0,
        };
        queries::insert_board_entry(db, record)?;
    }

    Ok(run_id.to_string())
}

/// Persist a ring orchestration run from its ES projection: the parent
/// `runs` row, `orchestration_runs` metadata, one `ring_contributions` row
/// per `state.ring.contributions` entry, and one `ring_votes` row per
/// `state.ring.votes` entry. Mirrors `record_orchestration_ring_into`'s
/// shape/column choices — reusing the same low-level
/// `insert_run_with_id`/`insert_orchestration_run`/
/// `insert_ring_contribution`/`insert_ring_vote` functions — but reads
/// `ExecutionState` instead of a live `ring::RingToken`. Returns the
/// provided `run_id`.
#[cfg(feature = "storage")]
pub fn record_ring_es_into(
    db: &armadai_storage::Database,
    run_id: &str,
    state: &ExecutionState,
    config: &RingConfig,
    input: &str,
    parent_run_id: Option<&str>,
    project: Option<&str>,
) -> anyhow::Result<String> {
    use armadai_storage::queries;

    let status = match state.status {
        RunStatus::Halted => "halted",
        RunStatus::Completed => "done",
        RunStatus::Running => "incomplete",
    };

    let parent = queries::RunRecord {
        agent: "orchestration:ring".to_string(),
        input: input.to_string(),
        output: ring_contributions_text(state),
        provider: "orchestration".to_string(),
        model: String::new(),
        tokens_in: i64::from(u32::try_from(state.budget_tokens_in).unwrap_or(u32::MAX)),
        tokens_out: i64::from(u32::try_from(state.budget_tokens_out).unwrap_or(u32::MAX)),
        cost: state.budget_cost,
        duration_ms: 0,
        status: status.to_string(),
        project: project.map(str::to_string),
    };
    queries::insert_run_with_id(db, run_id, parent)?;

    let orch = queries::OrchestrationRunRecord {
        run_id: run_id.to_string(),
        pattern: "ring".to_string(),
        config_json: serde_json::to_string(config).unwrap_or_default(),
        // No ES-native equivalent of the legacy typed `TokenStatus` outcome —
        // documented regression, see module docs.
        outcome_json: None,
        rounds: i64::from(state.ring.lap),
        // See module docs: halt reason lives on the event log, not `state`.
        halt_reason: None,
        parent_run_id: parent_run_id.map(str::to_string),
    };
    queries::insert_orchestration_run(db, orch)?;

    for c in &state.ring.contributions {
        let record = queries::RingContributionRecord {
            run_id: run_id.to_string(),
            agent: c.agent.clone(),
            lap: i64::from(c.lap),
            position_in_lap: c.position as i64,
            action: c.action.clone(),
            content: c.content.clone(),
            // `ContribRec` carries no reactions field — documented
            // regression, see module docs.
            reactions_json: "[]".to_string(),
            // Per-contribution tokens: documented regression, see module docs.
            tokens_in: 0,
            tokens_out: 0,
        };
        queries::insert_ring_contribution(db, record)?;
    }

    for vote in state.ring.votes.values() {
        let record = queries::RingVoteRecord {
            run_id: run_id.to_string(),
            agent: vote.agent.clone(),
            position: vote.position.clone(),
            confidence: f64::from(vote.confidence),
            supports: serde_json::to_string(&vote.supports).unwrap_or_default(),
            concerns: serde_json::to_string(&vote.concerns).unwrap_or_default(),
        };
        queries::insert_ring_vote(db, record)?;
    }

    Ok(run_id.to_string())
}

/// Idempotent projector: re-derive all flat-table rows (`runs`,
/// `orchestration_runs`, `board_entries`, `ring_contributions`, `ring_votes`,
/// `delegation_events`) for a given `run_id` from its event log.
///
/// Reads `execution_events[run_id]`, folds them into an `ExecutionState`,
/// extracts the runtime config (from `ConfigSnapshot` → `state.config_json`),
/// deletes any existing projection rows, then calls the pattern-specific
/// `record_*_es_into` function to rebuild them. Multiple calls on the same
/// `run_id` produce the exact same rows (idempotence: DELETE before INSERT).
///
/// Returns `Ok(())` when the projection succeeds, or an error if the event log
/// is malformed (e.g. no `RunStarted`) or storage fails.
#[cfg(feature = "storage")]
pub fn project_run(db: &armadai_storage::Database, run_id: &str) -> anyhow::Result<()> {
    use crate::es_log::SqliteLog;
    use armadai_core::orchestration::es::log::EventLog;
    use armadai_core::orchestration::es::state::fold;

    // 1. Read the event log for this run.
    let log = SqliteLog::new(db.clone());
    let events = log.events(run_id)?;

    if events.is_empty() {
        // No events = nothing to project (run doesn't exist or was never started).
        return Ok(());
    }

    // 2. Fold into ExecutionState.
    let state = fold(&events);

    // 3. Extract pattern/input/project from the first RunStarted event.
    let (pattern, input, project) = events
        .iter()
        .find_map(|e| match e {
            ExecutionEvent::RunStarted {
                pattern,
                input,
                project,
                ..
            } => Some((pattern.clone(), input.clone(), project.clone())),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("No RunStarted event in log for run {}", run_id))?;

    // 4. Delete any existing projection rows (idempotence: clear before rebuilding).
    armadai_storage::queries::delete_projection_for_run(db, run_id)?;

    // 5. Rebuild the projection by calling the pattern-specific record function.
    match pattern.as_str() {
        "blackboard" => {
            let config: BlackboardConfig = state
                .config_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default();
            record_blackboard_es_into(
                db,
                run_id,
                &state,
                &config,
                &input,
                None,
                project.as_deref(),
            )?;
        }
        "ring" => {
            let config: RingConfig = state
                .config_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default();
            record_ring_es_into(
                db,
                run_id,
                &state,
                &config,
                &input,
                None,
                project.as_deref(),
            )?;
        }
        "hierarchical" => {
            use armadai_core::orchestration::OrchestrationConfig;
            use armadai_core::orchestration::es::bridge::to_orchestration_result;

            let result = to_orchestration_result(&state, &events);
            let config: OrchestrationConfig = state
                .config_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_default();

            crate::cli::run::record_hierarchical_into(
                db,
                run_id,
                &result,
                &config,
                &input,
                project.as_deref(),
            )?;
        }
        "direct" => {
            // Direct runs have no orchestration metadata; nothing to project.
        }
        _ => {
            anyhow::bail!(
                "Unknown orchestration pattern '{}' for run {}",
                pattern,
                run_id
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use armadai_core::orchestration::es::state::{BoardEntryRec, ContribRec, VoteRec};

    fn sample_blackboard_state() -> ExecutionState {
        let mut state = ExecutionState {
            run_id: "r1".to_string(),
            pattern: "blackboard".to_string(),
            agents: vec!["a".to_string(), "b".to_string()],
            budget_tokens_in: 100,
            budget_tokens_out: 200,
            budget_cost: 0.05,
            status: RunStatus::Completed,
            ..Default::default()
        };
        state.board.round = 2;
        state.board.entries.push(BoardEntryRec {
            agent: "a".to_string(),
            round: 1,
            kind: "finding".to_string(),
            content: "first finding".to_string(),
            refs: vec![],
            confidence: 0.9,
        });
        state.board.entries.push(BoardEntryRec {
            agent: "b".to_string(),
            round: 2,
            kind: "challenge".to_string(),
            content: "a counter-point".to_string(),
            refs: vec![0],
            confidence: 0.5,
        });
        state
    }

    fn sample_ring_state() -> ExecutionState {
        let mut state = ExecutionState {
            run_id: "r2".to_string(),
            pattern: "ring".to_string(),
            agents: vec!["a".to_string(), "b".to_string()],
            budget_tokens_in: 30,
            budget_tokens_out: 40,
            budget_cost: 0.02,
            status: RunStatus::Completed,
            ..Default::default()
        };
        state.ring.lap = 1;
        state.ring.contributions.push(ContribRec {
            agent: "a".to_string(),
            lap: 1,
            position: 0,
            action: "propose".to_string(),
            content: "initial proposal".to_string(),
        });
        state.ring.votes.insert(
            "b".to_string(),
            VoteRec {
                agent: "b".to_string(),
                position: "approve".to_string(),
                confidence: 0.8,
                supports: vec![0],
                concerns: vec!["timeline".to_string()],
            },
        );
        state
    }

    // ── Display helpers (no feature gate needed) ────────────────────

    #[test]
    fn blackboard_display_concats_agent_and_content_in_order() {
        let state = sample_blackboard_state();
        assert_eq!(
            blackboard_display(&state),
            "[a] first finding\n[b] a counter-point"
        );
    }

    #[test]
    fn blackboard_display_empty_when_no_entries() {
        let state = ExecutionState::default();
        assert_eq!(blackboard_display(&state), "");
    }

    #[test]
    fn ring_display_uses_last_outcome_resolved_and_lists_votes() {
        let state = sample_ring_state();
        // Realistic log order: the run completes, then (in this pattern)
        // the ring outcome is resolved — `ring_display` must prefer the
        // *last* `OutcomeResolved`, not the first event in the log.
        let events = [
            ExecutionEvent::Completed {
                content: "ignored: not the outcome".to_string(),
            },
            ExecutionEvent::OutcomeResolved {
                outcome: "consensus reached".to_string(),
            },
        ];
        let out = ring_display(&state, &events);
        assert!(out.starts_with("consensus reached"));
        assert!(out.contains("[votes] b approve (80%)"));
    }

    #[test]
    fn ring_display_falls_back_to_completed_when_no_outcome_resolved() {
        let state = ExecutionState::default();
        let events = [ExecutionEvent::Completed {
            content: "done".to_string(),
        }];
        assert_eq!(ring_display(&state, &events), "done");
    }

    #[test]
    fn ring_display_empty_without_votes_or_events() {
        let state = ExecutionState::default();
        assert_eq!(ring_display(&state, &[]), "");
    }
}

#[cfg(all(test, feature = "storage"))]
mod storage_tests {
    use super::*;
    use armadai_core::orchestration::es::state::{BoardEntryRec, ContribRec, VoteRec};
    use armadai_storage::{open_in_memory, queries};

    fn sample_blackboard_state() -> ExecutionState {
        let mut state = ExecutionState {
            budget_tokens_in: 100,
            budget_tokens_out: 200,
            budget_cost: 0.05,
            status: RunStatus::Completed,
            ..Default::default()
        };
        state.board.round = 2;
        state.board.entries.push(BoardEntryRec {
            agent: "a".to_string(),
            round: 1,
            kind: "finding".to_string(),
            content: "first finding".to_string(),
            refs: vec![],
            confidence: 0.9,
        });
        state.board.entries.push(BoardEntryRec {
            agent: "b".to_string(),
            round: 2,
            kind: "challenge".to_string(),
            content: "a counter-point".to_string(),
            refs: vec![0],
            confidence: 0.5,
        });
        state
    }

    fn sample_ring_state() -> ExecutionState {
        let mut state = ExecutionState {
            budget_tokens_in: 30,
            budget_tokens_out: 40,
            budget_cost: 0.02,
            status: RunStatus::Halted,
            ..Default::default()
        };
        state.ring.lap = 1;
        state.ring.contributions.push(ContribRec {
            agent: "a".to_string(),
            lap: 1,
            position: 0,
            action: "propose".to_string(),
            content: "initial proposal".to_string(),
        });
        state.ring.contributions.push(ContribRec {
            agent: "b".to_string(),
            lap: 1,
            position: 1,
            action: "enrich".to_string(),
            content: "adds detail".to_string(),
        });
        state.ring.votes.insert(
            "a".to_string(),
            VoteRec {
                agent: "a".to_string(),
                position: "approve".to_string(),
                confidence: 0.8,
                supports: vec![0],
                concerns: vec![],
            },
        );
        state.ring.votes.insert(
            "b".to_string(),
            VoteRec {
                agent: "b".to_string(),
                position: "reject".to_string(),
                confidence: 0.3,
                supports: vec![],
                concerns: vec!["incomplete".to_string()],
            },
        );
        state
    }

    #[test]
    fn record_blackboard_es_into_persists_run_and_entries() {
        let db = open_in_memory().unwrap();
        let state = sample_blackboard_state();
        let config = BlackboardConfig::default();

        let run_id = uuid::Uuid::new_v4().to_string();
        let returned =
            record_blackboard_es_into(&db, &run_id, &state, &config, "do research", None, None)
                .unwrap();
        assert_eq!(returned, run_id);

        let history = queries::get_history(&db, None, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].tokens_in, 100);
        assert_eq!(history[0].tokens_out, 200);
        assert!((history[0].cost - 0.05).abs() < 1e-9);
        assert_eq!(history[0].status, "success");

        let orch = queries::get_orchestration_run(&db, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(orch.pattern, "blackboard");
        assert_eq!(orch.rounds, 2);
        assert_eq!(orch.parent_run_id, None);

        let entries = queries::get_board_entries(&db, &run_id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].agent, "a");
        assert_eq!(entries[0].kind, "finding");
        assert_eq!(entries[1].agent, "b");
        assert_eq!(entries[1].kind, "challenge");
        assert_eq!(entries[1].refs_json, "[0]");
        // Documented regression: per-entry tokens are 0 in the ES-native path.
        assert_eq!(entries[0].tokens_in, 0);
        assert_eq!(entries[0].tokens_out, 0);
    }

    #[test]
    fn record_blackboard_es_into_links_parent_run_id_and_project() {
        let db = open_in_memory().unwrap();
        let state = sample_blackboard_state();
        let config = BlackboardConfig::default();

        let run_id = uuid::Uuid::new_v4().to_string();
        let returned = record_blackboard_es_into(
            &db,
            &run_id,
            &state,
            &config,
            "sub task",
            Some("parent-123"),
            Some("/home/user/project"),
        )
        .unwrap();
        assert_eq!(returned, run_id);

        let orch = queries::get_orchestration_run(&db, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(orch.parent_run_id.as_deref(), Some("parent-123"));

        let history = queries::get_history(&db, None, 10).unwrap();
        assert_eq!(history[0].project.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn record_ring_es_into_persists_run_contributions_and_votes() {
        let db = open_in_memory().unwrap();
        let state = sample_ring_state();
        let config = RingConfig::default();

        let run_id = uuid::Uuid::new_v4().to_string();
        let returned =
            record_ring_es_into(&db, &run_id, &state, &config, "do research", None, None).unwrap();
        assert_eq!(returned, run_id);

        let history = queries::get_history(&db, None, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].tokens_in, 30);
        assert_eq!(history[0].tokens_out, 40);
        assert!((history[0].cost - 0.02).abs() < 1e-9);
        assert_eq!(history[0].status, "halted");

        let orch = queries::get_orchestration_run(&db, &run_id)
            .unwrap()
            .unwrap();
        assert_eq!(orch.pattern, "ring");
        assert_eq!(orch.rounds, 1);

        let contributions = queries::get_ring_contributions(&db, &run_id).unwrap();
        assert_eq!(contributions.len(), 2);
        assert_eq!(contributions[0].agent, "a");
        assert_eq!(contributions[0].action, "propose");
        assert_eq!(contributions[1].agent, "b");
        assert_eq!(contributions[1].action, "enrich");
        // Documented regression: per-contribution tokens are 0.
        assert_eq!(contributions[0].tokens_in, 0);

        let votes = queries::get_ring_votes(&db, &run_id).unwrap();
        assert_eq!(votes.len(), 2);
        let a_vote = votes.iter().find(|v| v.agent == "a").unwrap();
        assert_eq!(a_vote.position, "approve");
        // f32 -> f64 widening isn't exact (0.8_f32 as f64 != 0.8_f64); a
        // wider tolerance than the plain-f64 assertions above is needed here.
        assert!((a_vote.confidence - 0.8).abs() < 1e-6);
        let b_vote = votes.iter().find(|v| v.agent == "b").unwrap();
        assert_eq!(b_vote.position, "reject");
        assert!(b_vote.concerns.contains("incomplete"));
    }

    #[test]
    fn record_blackboard_es_into_uses_caller_run_id() {
        let db = open_in_memory().unwrap();
        let state = sample_blackboard_state();
        let cfg = BlackboardConfig::default();
        let returned =
            record_blackboard_es_into(&db, "fixed-run-id-123", &state, &cfg, "task", None, None)
                .unwrap();
        assert_eq!(returned, "fixed-run-id-123");
        // La ligne persistée porte bien ce run_id.
        let run = queries::get_orchestration_run(&db, "fixed-run-id-123")
            .unwrap()
            .unwrap();
        assert_eq!(run.run_id, "fixed-run-id-123");
    }

    /// Helper: construct a minimal blackboard event log suitable for
    /// projection tests.
    fn sample_blackboard_events(run_id: &str) -> Vec<ExecutionEvent> {
        vec![
            ExecutionEvent::RunStarted {
                run_id: run_id.to_string(),
                pattern: "blackboard".to_string(),
                agents: vec!["a".to_string(), "b".to_string()],
                input: "do research".to_string(),
                project: None,
            },
            ExecutionEvent::ConfigSnapshot {
                config_json:
                    r#"{"max_rounds":5,"convergence_threshold":0.8,"consecutive_rounds":2}"#
                        .to_string(),
            },
            ExecutionEvent::RoundStarted { round: 1 },
            ExecutionEvent::AgentInvoked {
                agent: "a".to_string(),
                input: "task input".to_string(),
            },
            ExecutionEvent::BoardEntryAdded {
                agent: "a".to_string(),
                round: 1,
                kind: "finding".to_string(),
                content: "first finding".to_string(),
                refs: vec![],
                confidence: 0.9,
                tokens_in: 50,
                tokens_out: 100,
                cost: 0.03,
            },
            ExecutionEvent::Completed {
                content: "final result".to_string(),
            },
        ]
    }

    #[test]
    fn project_run_is_idempotent() {
        let db = open_in_memory().unwrap();

        // Persist a minimal blackboard log via SqliteLog.
        let mut log = crate::es_log::SqliteLog::new(db.clone());
        for e in sample_blackboard_events("run-x") {
            use armadai_core::orchestration::es::log::EventLog;
            log.append("run-x", &e).unwrap();
        }

        // Project twice.
        super::project_run(&db, "run-x").unwrap();
        super::project_run(&db, "run-x").unwrap();

        // Exactly one row in runs + orchestration_runs, no duplication.
        let run = queries::get_orchestration_run(&db, "run-x")
            .unwrap()
            .unwrap();
        assert_eq!(run.pattern, "blackboard");
        assert_eq!(run.run_id, "run-x");

        let history = queries::get_history(&db, None, 10).unwrap();
        assert_eq!(
            history
                .iter()
                .filter(|r| r.agent == "orchestration:blackboard")
                .count(),
            1
        );

        // Board entries: exactly one entry, not duplicated.
        let entries = queries::get_board_entries(&db, "run-x").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent, "a");
        assert_eq!(entries[0].kind, "finding");
    }

    // ── final_content: the shared resume/replay branch (storage-gated,
    // same as the function itself — see its doc comment) ────────────

    #[test]
    fn final_content_uses_ring_display_for_ring_pattern() {
        let mut state = ExecutionState {
            pattern: "ring".to_string(),
            ..Default::default()
        };
        state.ring.votes.insert(
            "b".to_string(),
            VoteRec {
                agent: "b".to_string(),
                position: "approve".to_string(),
                confidence: 0.8,
                supports: vec![0],
                concerns: vec![],
            },
        );
        let events = [ExecutionEvent::OutcomeResolved {
            outcome: "consensus reached".to_string(),
        }];
        let content = final_content(&state, &events);
        assert!(content.starts_with("consensus reached"));
        assert!(
            content.contains("[votes]"),
            "ring's final_content must include the vote tally, got: {content}"
        );
    }

    #[test]
    fn final_content_uses_blackboard_display_for_blackboard_pattern() {
        let mut state = ExecutionState {
            pattern: "blackboard".to_string(),
            ..Default::default()
        };
        state.board.entries.push(BoardEntryRec {
            agent: "a".to_string(),
            round: 1,
            kind: "finding".to_string(),
            content: "first finding".to_string(),
            refs: vec![],
            confidence: 0.9,
        });
        assert_eq!(final_content(&state, &[]), "[a] first finding");
    }

    #[test]
    fn final_content_falls_back_to_orchestration_result_for_other_patterns() {
        let state = ExecutionState {
            pattern: "hierarchical".to_string(),
            ..Default::default()
        };
        let events = [ExecutionEvent::Completed {
            content: "final answer".to_string(),
        }];
        assert_eq!(final_content(&state, &events), "final answer");
    }
}
