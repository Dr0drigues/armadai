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

use super::engine::{Action, Decider};
use super::event::ExecutionEvent;
use super::state::{BoardEntryRec, ExecutionState};
use crate::core::agent::Agent;
use crate::core::orchestration::blackboard::{BlackboardConfig, EntryKind, entry_kind_name};
use crate::core::routing::{BudgetState, RoutingRules, route};

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

// ── BlackboardDecider (Task 3): pure decision function ────────────

/// Pure blackboard [`Decider`]: given the current [`ExecutionState`],
/// decides the next batch of [`Action`]s — kick off round 0, invoke a
/// round's eligible agents, advance to the next round, or halt/complete on
/// convergence, `max_rounds`, or budget exhaustion.
///
/// Mirrors `HierarchicalDecider` (`es::hierarchical`, OH1 Lot 2): all fields
/// are immutable inputs captured at construction time, `decide` performs no
/// I/O and reads no mutable state — every decision is a pure function of
/// `state`, which is what keeps event-log replay deterministic.
#[derive(Debug, Clone)]
pub struct BlackboardDecider {
    /// All known agents by name, for model/tag lookups (routing) and
    /// trigger-based eligibility ([`eligible_agents`]).
    pub agents: BTreeMap<String, Agent>,
    /// Declared agent order (as configured). Reserved for seeding the run's
    /// `RunStarted { agents, .. }` roster in a future assembly function
    /// (the blackboard counterpart of `run_hierarchical_es`) — `decide`
    /// itself never reads it, deriving eligibility ordering from
    /// `state.agents` instead (populated from that same roster once the run
    /// has started).
    pub agent_order: Vec<String>,
    /// The original user input/task, given to every invoked agent. The
    /// actual per-round board prompt (filtered to `round < round_courant`,
    /// per the module's Task 4 note) is assembled by the effect runner, not
    /// here.
    pub input: String,
    /// Blackboard configuration (`max_rounds`/`consensus_threshold`/
    /// `convergence_rounds`/…), read by [`eligible_agents`],
    /// [`check_convergence`], and [`consecutive_convergence`].
    pub config: BlackboardConfig,
    /// Routing rules for `latest:auto` agents.
    pub routing_rules: RoutingRules,
    /// Max rounds before the run is force-completed. Kept as its own field
    /// (like `HierarchicalDecider::max_depth`) rather than always reading
    /// `config.max_rounds`, so callers may override it independently.
    pub max_rounds: u32,
    /// Optional total token budget (in + out) before the run is
    /// force-completed.
    pub token_budget: Option<u32>,
    /// Optional total cost budget (USD) before the run is force-completed.
    pub cost_limit: Option<f64>,
}

impl BlackboardDecider {
    /// Construct a new `BlackboardDecider`. All arguments become immutable
    /// fields read by `decide`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agents: BTreeMap<String, Agent>,
        agent_order: Vec<String>,
        input: impl Into<String>,
        config: BlackboardConfig,
        routing_rules: RoutingRules,
        max_rounds: u32,
        token_budget: Option<u32>,
        cost_limit: Option<f64>,
    ) -> Self {
        Self {
            agents,
            agent_order,
            input: input.into(),
            config,
            routing_rules,
            max_rounds,
            token_budget,
            cost_limit,
        }
    }

    /// Check budget/cost guards, returning the `Warned` code for whichever
    /// one has been breached (token budget checked first — same convention
    /// as `HierarchicalDecider::breached_limit`).
    fn breached_budget(&self, state: &ExecutionState) -> Option<&'static str> {
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
    /// `routing_input` (pure, via `crate::core::routing::route`) and return
    /// the `ModelRouted` event to emit before invoking it. Concrete models,
    /// other `latest:*` placeholders, and unknown agents all return `None`.
    ///
    /// Identical in spirit to `HierarchicalDecider::model_routed_event` —
    /// duplicated rather than shared, since the two deciders' `agents` /
    /// `routing_rules` / `token_budget` fields live on unrelated structs
    /// with no common trait today.
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

    /// Build the action batch that starts `round`: `Emit(RoundStarted)`
    /// followed by, for each of `eligible` (in roster order), an optional
    /// `Emit(ModelRouted)` then its `Invoke`.
    fn round_actions(
        &self,
        round: u32,
        eligible: &[String],
        state: &ExecutionState,
    ) -> Vec<Action> {
        let mut actions = vec![Action::Emit(ExecutionEvent::RoundStarted { round })];
        for agent in eligible {
            if let Some(event) = self.model_routed_event(agent, &self.input, state) {
                actions.push(Action::Emit(event));
            }
            actions.push(Action::Invoke {
                agent: agent.clone(),
                input: self.input.clone(),
            });
        }
        actions
    }

    /// Whether every one of `eligible` has already produced a
    /// `BoardEntryAdded` for `state.board.round` — the deterministic "round
    /// complete" signal `decide` needs, derived purely from
    /// `state.board.entries` (no separate bookkeeping counter to keep in
    /// sync).
    ///
    /// An empty `eligible` list never reads as complete: with nothing to
    /// wait for, treating it as "complete" would let a round with no
    /// eligible agents silently auto-advance round after round with no
    /// entries ever posted. `decide` therefore idles on that state instead
    /// (see the task report for this documented edge case — it does not
    /// arise in any of the four required test scenarios, all of which keep
    /// at least one agent eligible throughout).
    fn round_complete(&self, state: &ExecutionState, eligible: &[String]) -> bool {
        !eligible.is_empty()
            && eligible.iter().all(|agent| {
                state
                    .board
                    .entries
                    .iter()
                    .any(|e| e.round == state.board.round && &e.agent == agent)
            })
    }
}

impl Decider for BlackboardDecider {
    fn decide(&self, state: &ExecutionState) -> Vec<Action> {
        // 1. Nothing posted yet: kick off round 0 with its eligible agents.
        // This is the only branch reachable before any `BoardEntryAdded` has
        // been folded — in the production loop, `decide` is never called
        // again mid-round (a batch's `Invoke`s and their effects all land
        // before the next `decide`), so this state is exactly "the run just
        // started".
        if state.board.entries.is_empty() {
            let eligible = eligible_agents(state, &self.agents);
            return self.round_actions(0, &eligible, state);
        }

        let round = state.board.round;
        let eligible = eligible_agents(state, &self.agents);

        // 2. Still waiting on some eligible agent's contribution this round
        // — nothing to decide yet. A genuine wait, mirroring
        // `HierarchicalDecider::awaiting_in_flight`: in the real socle loop
        // an `Invoke`'s effect resolves within the same batch, so this only
        // arises for a synthetic/partial state (e.g. in a test).
        if !self.round_complete(state, &eligible) {
            return Vec::new();
        }

        // 3. Round complete: evaluate halt conditions in priority order —
        // budget/cost first (a hard external limit), then `max_rounds` (a
        // hard configured cap), then convergence (the "happy path" halt).
        if let Some(code) = self.breached_budget(state) {
            return vec![
                Action::Emit(ExecutionEvent::Warned {
                    code: code.to_string(),
                }),
                Action::Complete {
                    content: build_board_result(state),
                },
            ];
        }

        if round.saturating_add(1) >= self.max_rounds {
            return vec![
                Action::Emit(ExecutionEvent::Warned {
                    code: "max_rounds".to_string(),
                }),
                Action::Complete {
                    content: build_board_result(state),
                },
            ];
        }

        if consecutive_convergence(state, &self.config) >= self.config.convergence_rounds {
            // `consecutive_convergence >= 1` implies the current round's own
            // confirmation ratio already met `consensus_threshold` (it is
            // the first round counted, backward from the latest one present
            // in `state.board.entries`) — `check_convergence` therefore
            // returns `Some` here in practice. The fallback only guards a
            // theoretical drift between the two functions, never observed
            // in the four required test scenarios.
            let score =
                check_convergence(state, &self.config).unwrap_or(self.config.consensus_threshold);
            return vec![
                Action::Emit(ExecutionEvent::ConsensusReached { score }),
                Action::Complete {
                    content: build_board_result(state),
                },
            ];
        }

        // 4. Otherwise: advance to the next round. Eligibility for the new
        // round can differ from the current one (trigger `min_round`/
        // `max_round` windows), so it is recomputed against a
        // round-advanced *clone* of `state` — pure lookahead, no mutation of
        // the real state (that only happens once the engine applies the
        // `RoundStarted` event this batch emits).
        let next_round = round + 1;
        let mut lookahead = state.clone();
        lookahead.board.round = next_round;
        let next_eligible = eligible_agents(&lookahead, &self.agents);
        self.round_actions(next_round, &next_eligible, state)
    }
}

/// Deterministic synthesis of the final board result: every entry, in the
/// order it was recorded (`state.board.entries` — a `Vec`, so this is the
/// chronological append order of the underlying event log), formatted as
/// `[agent] content`, one per line.
///
/// Matches the legacy engine's blackboard outcome text verbatim
/// (`cli::run`: `board.entries().iter().map(|entry| format!("[{}] {}",
/// entry.agent, entry.content))...join("\n")`), so the event-sourced and
/// legacy engines produce the same shape of final answer for the same
/// entry sequence.
pub(crate) fn build_board_result(state: &ExecutionState) -> String {
    state
        .board
        .entries
        .iter()
        .map(|entry| format!("[{}] {}", entry.agent, entry.content))
        .collect::<Vec<_>>()
        .join("\n")
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

    // ── BlackboardDecider (Task 3) ───────────────────────────────────

    /// Tests for `BlackboardDecider` (Task 3): pure decision function built
    /// on top of `eligible_agents`/`check_convergence`/
    /// `consecutive_convergence` (Task 2). Named `decide` so
    /// `cargo test es::blackboard::tests::decide` targets this module —
    /// mirrors the naming convention used by `es::hierarchical::tests::decide`.
    mod decide {
        use super::*;
        use crate::core::orchestration::es::engine::{Action, Decider};
        use crate::core::orchestration::es::state::fold;
        use crate::core::routing::RoutingRules;

        #[allow(clippy::too_many_arguments)]
        fn test_decider(
            agent_names: &[&str],
            config: BlackboardConfig,
            max_rounds: u32,
            token_budget: Option<u32>,
            cost_limit: Option<f64>,
        ) -> BlackboardDecider {
            let mut agents = BTreeMap::new();
            for name in agent_names {
                agents.insert((*name).to_string(), test_agent(name, None));
            }
            BlackboardDecider::new(
                agents,
                agent_names.iter().map(|a| (*a).to_string()).collect(),
                "task".to_string(),
                config,
                RoutingRules::default(),
                max_rounds,
                token_budget,
                cost_limit,
            )
        }

        fn invoked_agents(actions: &[Action]) -> Vec<&str> {
            actions
                .iter()
                .filter_map(|a| match a {
                    Action::Invoke { agent, .. } => Some(agent.as_str()),
                    _ => None,
                })
                .collect()
        }

        // (a) empty state → RoundStarted{0} + Invokes of the eligible agents.
        #[test]
        fn empty_state_starts_round_zero_and_invokes_eligible() {
            let dec = test_decider(&["a", "b"], BlackboardConfig::default(), 5, None, None);
            let state = fold(&[run_started(&["a", "b"])]);
            let actions = dec.decide(&state);

            assert!(
                matches!(&actions[0], Action::Emit(E::RoundStarted { round }) if *round == 0),
                "expected Emit(RoundStarted{{round: 0}}) first, got {actions:?}"
            );
            assert_eq!(invoked_agents(&actions), vec!["a", "b"]);
        }

        // (b) round complete, not convergent, < max_rounds → RoundStarted{1}
        // + Invokes of the (re-evaluated) eligible agents.
        #[test]
        fn round_complete_not_convergent_advances_round() {
            let dec = test_decider(&["a", "b"], BlackboardConfig::default(), 5, None, None);
            let events = vec![
                run_started(&["a", "b"]),
                E::RoundStarted { round: 0 },
                board_entry("a", 0, "finding", vec![], 0.5),
                board_entry("b", 0, "finding", vec![], 0.5),
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);

            assert!(
                matches!(&actions[0], Action::Emit(E::RoundStarted { round }) if *round == 1),
                "expected Emit(RoundStarted{{round: 1}}) first, got {actions:?}"
            );
            assert_eq!(invoked_agents(&actions), vec!["a", "b"]);
        }

        // (c) convergence reached `convergence_rounds` times consecutively
        // → ConsensusReached + Complete (synthesis of the board).
        #[test]
        fn convergence_reached_completes_with_consensus() {
            let config = BlackboardConfig {
                convergence_rounds: 1,
                ..BlackboardConfig::default()
            };
            let dec = test_decider(&["a", "b"], config, 5, None, None);
            let events = vec![
                run_started(&["a", "b"]),
                E::RoundStarted { round: 0 },
                board_entry("a", 0, "confirmation", vec![0], 0.9),
                board_entry("b", 0, "confirmation", vec![0], 0.9),
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);

            assert_eq!(actions.len(), 2, "got {actions:?}");
            assert!(
                matches!(&actions[0], Action::Emit(E::ConsensusReached { score }) if (*score - 1.0).abs() < 1e-6),
                "expected ConsensusReached{{score: 1.0}}, got {:?}",
                actions[0]
            );
            assert!(
                matches!(&actions[1], Action::Complete { content } if content.contains("[a]") && content.contains("[b]")),
                "expected Complete with both entries, got {:?}",
                actions[1]
            );
        }

        // (d) round+1 >= max_rounds → Warned{max_rounds} + Complete, even
        // though nothing has converged.
        #[test]
        fn max_rounds_reached_warns_and_completes() {
            let dec = test_decider(&["a", "b"], BlackboardConfig::default(), 1, None, None);
            let events = vec![
                run_started(&["a", "b"]),
                E::RoundStarted { round: 0 },
                board_entry("a", 0, "finding", vec![], 0.5),
                board_entry("b", 0, "finding", vec![], 0.5),
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);

            assert_eq!(actions.len(), 2, "got {actions:?}");
            assert!(
                matches!(&actions[0], Action::Emit(E::Warned { code }) if code == "max_rounds"),
                "expected Warned{{code: \"max_rounds\"}}, got {:?}",
                actions[0]
            );
            assert!(
                matches!(&actions[1], Action::Complete { content } if content.contains("[a]") && content.contains("[b]")),
                "expected Complete with both entries, got {:?}",
                actions[1]
            );
        }

        // Round not yet complete (only one of two eligible agents has
        // contributed) → idle (no actions), the "genuine wait" documented on
        // `BlackboardDecider::decide`.
        #[test]
        fn incomplete_round_is_idle() {
            let dec = test_decider(&["a", "b"], BlackboardConfig::default(), 5, None, None);
            let events = vec![
                run_started(&["a", "b"]),
                E::RoundStarted { round: 0 },
                board_entry("a", 0, "finding", vec![], 0.5),
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);
            assert!(actions.is_empty(), "got {actions:?}");
        }

        // Budget exhaustion is checked ahead of `max_rounds`/convergence: a
        // round complete with the token budget already spent halts
        // immediately, regardless of round count.
        #[test]
        fn token_budget_exhausted_warns_and_completes() {
            let dec = test_decider(&["a", "b"], BlackboardConfig::default(), 5, Some(10), None);
            let events = vec![
                run_started(&["a", "b"]),
                E::RoundStarted { round: 0 },
                E::BoardEntryAdded {
                    agent: "a".into(),
                    round: 0,
                    kind: "finding".into(),
                    content: "c".into(),
                    refs: vec![],
                    confidence: 0.5,
                    tokens_in: 6,
                    tokens_out: 6,
                    cost: 0.0,
                },
                board_entry("b", 0, "finding", vec![], 0.5),
            ];
            let state = fold(&events);
            let actions = dec.decide(&state);

            assert_eq!(actions.len(), 2, "got {actions:?}");
            assert!(
                matches!(&actions[0], Action::Emit(E::Warned { code }) if code == "token_budget"),
                "expected Warned{{code: \"token_budget\"}}, got {:?}",
                actions[0]
            );
            assert!(matches!(&actions[1], Action::Complete { .. }));
        }
    }
}
