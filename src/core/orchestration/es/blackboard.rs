//! Pure helpers for the blackboard pattern (OH1 Lot 3): agent eligibility,
//! convergence detection, and `EntryKind` ↔ `(kind, refs)` mapping.
//!
//! Reproduces the *decision* logic of the legacy blackboard engine —
//! `core::orchestration::blackboard::check_convergence`/`Board::consecutive_convergence`
//! and `core::orchestration::llm_agents::LlmBoardAgent::can_contribute` — as
//! pure, synchronous functions over the event-sourced `ExecutionState`
//! projection: no I/O, no clock, no randomness, no `tracing`. Strict
//! coexistence: the legacy `blackboard.rs`/`llm_agents.rs` engines are
//! untouched; this module only *imports* their plain, side-effect-free data
//! types (`BlackboardConfig`, `EntryKind`, `entry_kind_name`) rather than
//! duplicating their definitions.

use std::collections::BTreeMap;

use super::state::{BoardEntryRec, ExecutionState};
use crate::core::agent::Agent;
use crate::core::orchestration::blackboard::{BlackboardConfig, EntryKind, entry_kind_name};

/// Agents eligible to contribute on the board's current round, ordered by
/// the run's roster (`state.agents`) rather than `agents`' own (`BTreeMap`,
/// name-sorted) iteration order.
///
/// Reproduces `LlmBoardAgent::can_contribute`
/// (`core::orchestration::llm_agents`): an agent with no `triggers`
/// configured is always eligible; otherwise every one of the following must
/// hold against `state.board`:
/// - `state.board.round >= triggers.min_round`
/// - `triggers.max_round.is_none()` or `state.board.round <= max_round`
/// - every kind in `triggers.requires` is present among `state.board.entries`
/// - no kind in `triggers.excludes` is present among `state.board.entries`
///
/// A name in `state.agents` with no matching entry in `agents` is skipped
/// (only possible for a malformed/partial state — the roster is expected to
/// be a subset of `agents`' keys in practice).
pub(crate) fn eligible_agents(
    state: &ExecutionState,
    agents: &BTreeMap<String, Agent>,
) -> Vec<String> {
    let present_kinds: Vec<&str> = state
        .board
        .entries
        .iter()
        .map(|e| e.kind.as_str())
        .collect();

    state
        .agents
        .iter()
        .filter(|name| {
            let Some(agent) = agents.get(*name) else {
                return false;
            };
            let Some(triggers) = agent.metadata.triggers.as_ref() else {
                return true;
            };

            if state.board.round < triggers.min_round {
                return false;
            }
            if let Some(max) = triggers.max_round
                && state.board.round > max
            {
                return false;
            }
            if triggers
                .requires
                .iter()
                .any(|req| !present_kinds.contains(&req.to_lowercase().as_str()))
            {
                return false;
            }
            if triggers
                .excludes
                .iter()
                .any(|excl| present_kinds.contains(&excl.to_lowercase().as_str()))
            {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

/// `confirmations / total` for the entries of `entries` matching `round`, or
/// `None` if that round has no entries (division-by-zero guard, mirroring
/// the legacy `check_convergence`'s `last_round_entries.is_empty()` early
/// return).
fn round_confirmation_ratio(entries: &[BoardEntryRec], round: u32) -> Option<f32> {
    let round_entries: Vec<&BoardEntryRec> = entries.iter().filter(|e| e.round == round).collect();
    if round_entries.is_empty() {
        return None;
    }
    let confirmations = round_entries
        .iter()
        .filter(|e| e.kind == "confirmation")
        .count();
    Some(confirmations as f32 / round_entries.len() as f32)
}

/// Confirmation-based convergence signal for `state.board.round`, reproducing
/// the "Consensus" branch of the legacy
/// `core::orchestration::blackboard::check_convergence` purely.
///
/// Deliberately narrower than the legacy function: no `tracing::warn!`
/// budget-warning side effect (irrelevant here — `ExecutionState` carries no
/// budget-warning threshold to log against), and no `Stable`/`Divergence`
/// branches (`ExecutionState`/`BoardEntryRec` don't model a `HaltReason`, only
/// a bare confirmation ratio; the empty-round case that legacy reports as
/// `Some(HaltReason::Stable)` returns `None` here instead — see module docs
/// and the task report for this documented deviation).
///
/// Returns `Some(ratio)` when `confirmations / total >= config.consensus_threshold`
/// for the current round's entries, `None` otherwise (including on an empty
/// round).
pub(crate) fn check_convergence(state: &ExecutionState, config: &BlackboardConfig) -> Option<f32> {
    round_confirmation_ratio(&state.board.entries, state.board.round)
        .filter(|&ratio| ratio >= config.consensus_threshold)
}

/// Number of rounds, counted backward from the latest round present in
/// `state.board.entries`, whose confirmation ratio reaches
/// `config.consensus_threshold` without interruption.
///
/// Pure reconstruction of the legacy `Board::consecutive_convergence` counter
/// — which increments by one each round `check_convergence` detects
/// consensus and resets to `0` the moment it doesn't — from the final entry
/// log alone, since `ExecutionState` carries no running counter of its own.
///
/// Distinct rounds are derived from `state.board.entries` itself (sorted,
/// deduped), not from `0..=state.board.round`, so a state folded from a
/// partial/sparse event log (e.g. skipping straight to a later round)
/// reconstructs correctly. Returns `0` when there are no entries at all.
pub(crate) fn consecutive_convergence(state: &ExecutionState, config: &BlackboardConfig) -> u32 {
    let mut rounds: Vec<u32> = state.board.entries.iter().map(|e| e.round).collect();
    rounds.sort_unstable();
    rounds.dedup();

    let mut count = 0u32;
    for &round in rounds.iter().rev() {
        match round_confirmation_ratio(&state.board.entries, round) {
            Some(ratio) if ratio >= config.consensus_threshold => count += 1,
            _ => break,
        }
    }
    count
}

/// Map a legacy `EntryKind` (`core::orchestration::blackboard`) onto the
/// event-sourced `(kind: String, refs: Vec<usize>)` pair carried by
/// `ExecutionEvent::BoardEntryAdded`/`BoardEntryRec`.
///
/// `kind` reuses `entry_kind_name` — the single source of truth for the
/// lowercase names `eligible_agents`'s requires/excludes matching (and the
/// legacy `can_contribute`) read back. `refs` documents the reference
/// convention:
/// - `Finding`, `Question` → no refs
/// - `Challenge { target }`, `Confirmation { target }` → `refs = [target]`
/// - `Synthesis { sources }` → `refs = sources` (in order)
/// - `Answer { question }` → `refs = [question]`
pub(crate) fn entry_kind_to_rec(kind: &EntryKind) -> (String, Vec<usize>) {
    let refs = match kind {
        EntryKind::Finding | EntryKind::Question => vec![],
        EntryKind::Challenge { target } | EntryKind::Confirmation { target } => vec![*target],
        EntryKind::Synthesis { sources } => sources.clone(),
        EntryKind::Answer { question } => vec![*question],
    };
    (entry_kind_name(kind).to_string(), refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::AgentMetadata;
    use crate::core::orchestration::TriggerConfig;
    use crate::core::orchestration::es::event::ExecutionEvent as E;
    use crate::core::orchestration::es::state::fold;
    use std::path::PathBuf;

    fn test_agent(name: &str, triggers: Option<TriggerConfig>) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
                provider: "anthropic".to_string(),
                model: Some("concrete-model".to_string()),
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
                triggers,
                ring_config: None,
            },
            system_prompt: "prompt".to_string(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    fn run_started(agents: &[&str]) -> E {
        E::RunStarted {
            run_id: "r".into(),
            pattern: "blackboard".into(),
            agents: agents.iter().map(|a| a.to_string()).collect(),
            input: "task".into(),
            project: None,
        }
    }

    fn board_entry(agent: &str, round: u32, kind: &str, refs: Vec<usize>, confidence: f32) -> E {
        E::BoardEntryAdded {
            agent: agent.to_string(),
            round,
            kind: kind.to_string(),
            content: "c".to_string(),
            refs,
            confidence,
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
        }
    }

    // ── eligible_agents ────────────────────────────────────────────

    #[test]
    fn eligible_agents_no_triggers_always_eligible() {
        let mut agents = BTreeMap::new();
        agents.insert("a".to_string(), test_agent("a", None));
        let state = fold(&[run_started(&["a"])]);
        assert_eq!(eligible_agents(&state, &agents), vec!["a".to_string()]);
    }

    #[test]
    fn eligible_agents_respects_min_round() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "a".to_string(),
            test_agent(
                "a",
                Some(TriggerConfig {
                    requires: vec![],
                    excludes: vec![],
                    min_round: 1,
                    max_round: None,
                    priority: 50,
                }),
            ),
        );
        let state0 = fold(&[run_started(&["a"]), E::RoundStarted { round: 0 }]);
        assert!(eligible_agents(&state0, &agents).is_empty());

        let state1 = fold(&[run_started(&["a"]), E::RoundStarted { round: 1 }]);
        assert_eq!(eligible_agents(&state1, &agents), vec!["a".to_string()]);
    }

    #[test]
    fn eligible_agents_respects_max_round() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "a".to_string(),
            test_agent(
                "a",
                Some(TriggerConfig {
                    requires: vec![],
                    excludes: vec![],
                    min_round: 0,
                    max_round: Some(1),
                    priority: 50,
                }),
            ),
        );
        let state_ok = fold(&[run_started(&["a"]), E::RoundStarted { round: 1 }]);
        assert_eq!(eligible_agents(&state_ok, &agents), vec!["a".to_string()]);

        let state_over = fold(&[run_started(&["a"]), E::RoundStarted { round: 2 }]);
        assert!(eligible_agents(&state_over, &agents).is_empty());
    }

    #[test]
    fn eligible_agents_requires_kind_present_on_board() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "a".to_string(),
            test_agent(
                "a",
                Some(TriggerConfig {
                    requires: vec!["confirmation".to_string()],
                    excludes: vec![],
                    min_round: 0,
                    max_round: None,
                    priority: 50,
                }),
            ),
        );
        let no_confirmation = fold(&[
            run_started(&["a"]),
            board_entry("b", 0, "finding", vec![], 0.5),
        ]);
        assert!(eligible_agents(&no_confirmation, &agents).is_empty());

        let with_confirmation = fold(&[
            run_started(&["a"]),
            board_entry("b", 0, "finding", vec![], 0.5),
            board_entry("c", 0, "confirmation", vec![0], 0.9),
        ]);
        assert_eq!(
            eligible_agents(&with_confirmation, &agents),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn eligible_agents_excludes_kind_present_on_board() {
        let mut agents = BTreeMap::new();
        agents.insert(
            "a".to_string(),
            test_agent(
                "a",
                Some(TriggerConfig {
                    requires: vec![],
                    excludes: vec!["challenge".to_string()],
                    min_round: 0,
                    max_round: None,
                    priority: 50,
                }),
            ),
        );
        let clean = fold(&[
            run_started(&["a"]),
            board_entry("b", 0, "finding", vec![], 0.5),
        ]);
        assert_eq!(eligible_agents(&clean, &agents), vec!["a".to_string()]);

        let challenged = fold(&[
            run_started(&["a"]),
            board_entry("b", 0, "challenge", vec![0], 0.5),
        ]);
        assert!(eligible_agents(&challenged, &agents).is_empty());
    }

    #[test]
    fn eligible_agents_ordered_by_roster_not_by_agent_map() {
        let mut agents = BTreeMap::new();
        // BTreeMap iteration order would be "a" then "b" (alphabetic) — the
        // roster deliberately reverses that, and the result must follow it.
        agents.insert("a".to_string(), test_agent("a", None));
        agents.insert("b".to_string(), test_agent("b", None));
        let state = fold(&[run_started(&["b", "a"])]);
        assert_eq!(
            eligible_agents(&state, &agents),
            vec!["b".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn eligible_agents_skips_names_missing_from_agents_map() {
        let agents: BTreeMap<String, Agent> = BTreeMap::new();
        let state = fold(&[run_started(&["ghost"])]);
        assert!(eligible_agents(&state, &agents).is_empty());
    }

    // ── check_convergence ────────────────────────────────────────────

    #[test]
    fn check_convergence_empty_round_is_none() {
        let state = fold(&[run_started(&["a"])]);
        let config = BlackboardConfig::default();
        assert_eq!(check_convergence(&state, &config), None);
    }

    #[test]
    fn check_convergence_high_consensus_matches_legacy_case() {
        // Mirrors `blackboard::tests::test_check_convergence_high_consensus`:
        // round 1 has 4 confirmations + 1 new finding == 5 entries, 4/5 = 0.8
        // >= default threshold 0.75.
        let mut events = vec![run_started(&["a"]), E::RoundStarted { round: 1 }];
        for i in 0..4 {
            events.push(board_entry(
                &format!("agent-{i}"),
                1,
                "confirmation",
                vec![0],
                0.9,
            ));
        }
        events.push(board_entry("agent-4", 1, "finding", vec![], 0.7));
        let state = fold(&events);
        let config = BlackboardConfig::default();
        let ratio = check_convergence(&state, &config).expect("expected consensus");
        assert!((ratio - 0.8).abs() < 1e-6);
    }

    #[test]
    fn check_convergence_no_convergence_matches_legacy_case() {
        // Mirrors `test_check_convergence_no_convergence`: all findings, no
        // confirmations at all.
        let mut events = vec![run_started(&["a"])];
        for i in 0..5 {
            events.push(board_entry(
                &format!("agent-{i}"),
                0,
                "finding",
                vec![],
                0.5,
            ));
        }
        let state = fold(&events);
        let config = BlackboardConfig::default();
        assert_eq!(check_convergence(&state, &config), None);
    }

    #[test]
    fn check_convergence_below_threshold_is_none() {
        // 1 confirmation out of 4 entries = 0.25, below default 0.75.
        let events = vec![
            run_started(&["a"]),
            board_entry("a", 0, "confirmation", vec![0], 0.9),
            board_entry("b", 0, "finding", vec![], 0.5),
            board_entry("c", 0, "finding", vec![], 0.5),
            board_entry("d", 0, "finding", vec![], 0.5),
        ];
        let state = fold(&events);
        let config = BlackboardConfig::default();
        assert_eq!(check_convergence(&state, &config), None);
    }

    // ── consecutive_convergence ──────────────────────────────────────

    #[test]
    fn consecutive_convergence_counts_trailing_convergent_rounds() {
        let config = BlackboardConfig::default(); // consensus_threshold = 0.75
        // Round 0: non-convergent (all findings).
        // Round 1 & 2: convergent (100% confirmations).
        // Latest round (2) counted backward hits round 1 then stops at round 0.
        let events = vec![
            run_started(&["a"]),
            board_entry("a", 0, "finding", vec![], 0.5),
            board_entry("b", 0, "finding", vec![], 0.5),
            board_entry("a", 1, "confirmation", vec![0], 0.9),
            board_entry("b", 1, "confirmation", vec![0], 0.9),
            board_entry("a", 2, "confirmation", vec![0], 0.9),
            board_entry("b", 2, "confirmation", vec![0], 0.9),
        ];
        let state = fold(&events);
        assert_eq!(consecutive_convergence(&state, &config), 2);
    }

    #[test]
    fn consecutive_convergence_resets_at_first_non_convergent_round_from_the_end() {
        let config = BlackboardConfig::default();
        // Round 0 convergent, round 1 NOT convergent (latest) — counting
        // backward from round 1 stops immediately: result is 0, even though
        // an earlier round did converge.
        let events = vec![
            run_started(&["a"]),
            board_entry("a", 0, "confirmation", vec![0], 0.9),
            board_entry("b", 0, "confirmation", vec![0], 0.9),
            board_entry("a", 1, "finding", vec![], 0.5),
            board_entry("b", 1, "finding", vec![], 0.5),
        ];
        let state = fold(&events);
        assert_eq!(consecutive_convergence(&state, &config), 0);
    }

    #[test]
    fn consecutive_convergence_no_entries_is_zero() {
        let state = fold(&[run_started(&["a"])]);
        let config = BlackboardConfig::default();
        assert_eq!(consecutive_convergence(&state, &config), 0);
    }

    // ── entry_kind_to_rec ────────────────────────────────────────────

    #[test]
    fn entry_kind_to_rec_matches_documented_convention() {
        assert_eq!(
            entry_kind_to_rec(&EntryKind::Finding),
            ("finding".to_string(), vec![])
        );
        assert_eq!(
            entry_kind_to_rec(&EntryKind::Question),
            ("question".to_string(), vec![])
        );
        assert_eq!(
            entry_kind_to_rec(&EntryKind::Challenge { target: 3 }),
            ("challenge".to_string(), vec![3])
        );
        assert_eq!(
            entry_kind_to_rec(&EntryKind::Confirmation { target: 7 }),
            ("confirmation".to_string(), vec![7])
        );
        assert_eq!(
            entry_kind_to_rec(&EntryKind::Synthesis {
                sources: vec![1, 2, 4]
            }),
            ("synthesis".to_string(), vec![1, 2, 4])
        );
        assert_eq!(
            entry_kind_to_rec(&EntryKind::Answer { question: 5 }),
            ("answer".to_string(), vec![5])
        );
    }
}
