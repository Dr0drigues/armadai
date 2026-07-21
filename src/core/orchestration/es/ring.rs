//! Pure helpers for the ring pattern (OH1 Lot 3): phase derivation, vote
//! resolution (with its tie-break), and circulation rotation.
//!
//! Reproduces the *decision* logic of the legacy ring engine —
//! `core::orchestration::ring::resolve_votes`/`position_similarity` and the
//! three-phase loop (`run_ring`: circulation, voting, resolution) — as pure,
//! synchronous functions over the event-sourced `ExecutionState` projection:
//! no I/O, no clock, no randomness. Strict coexistence: the legacy
//! `ring.rs`/`llm_agents.rs` engines are untouched; this module only reuses
//! their plain, side-effect-free `RingConfig` type rather than duplicating
//! its definition.
//!
//! `ring_phase` is shared by the future `RingDecider` (Task 7) and
//! `RingEffectRunner` (Task 8) — both need the same answer to "what should
//! happen next" from the same `ExecutionState`, which is exactly what keeps
//! the derivation itself the single source of truth for the pattern's state
//! machine.

use std::collections::BTreeMap;

use super::state::{ExecutionState, RunStatus, VoteRec};
use crate::core::orchestration::ring::RingConfig;

/// The ring pattern's current phase, derived purely from [`ExecutionState`].
///
/// Mirrors the legacy `TokenStatus` (`Circulating`/`Voting`/`Done { .. }`)
/// but splits `Circulating` into an explicit lap number (needed by the
/// decider/effect to know *which* lap's agents to invoke next) and adds a
/// dedicated `Resolve` phase: the legacy engine performs voting and
/// resolution back-to-back in the same synchronous block (`run_ring`'s phase
/// 2 then phase 3), but the event-sourced loop re-derives the phase after
/// every batch, so "all votes are in, resolve now" and "still voting" must
/// be distinguishable states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingPhase {
    /// Circulation is in progress: agents still owe a contribution for
    /// `lap`.
    Circulate { lap: u32 },
    /// Circulation has ended (either every configured lap ran, or every
    /// agent passed within a lap — the legacy early-convergence exit) and at
    /// least one agent has not cast its vote yet.
    Vote,
    /// Every agent has voted: ready to resolve the outcome.
    Resolve,
    /// The run has already terminated (`ExecutionState::status` is no
    /// longer `Running`) — nothing left to derive.
    Done,
}

/// Derive the ring pattern's current [`RingPhase`] from `state.ring`.
///
/// Reproduces `run_ring`'s phase progression purely:
/// - `state.status != RunStatus::Running` → [`RingPhase::Done`] first,
///   regardless of any other field (a completed/halted run has nothing left
///   to circulate, vote, or resolve).
/// - While `state.ring.lap < config.max_laps`: compare the number of
///   contributions recorded for `state.ring.lap` against `state.agents.len()`
///   (the roster size) to tell whether the current lap is complete yet —
///   [`RingPhase::Circulate { lap: state.ring.lap }`] while it isn't.
/// - Once a lap is complete: if every contribution in it has
///   `action == "pass"` (the early-convergence exit — legacy's
///   `!any_substantive` after a full lap), circulation ends immediately,
///   even mid-run (mirrors `run_ring`'s unconditional `break`, without
///   incrementing `lap`). Otherwise, if there is a next lap
///   (`lap + 1 < config.max_laps`), the phase becomes
///   `Circulate { lap: lap + 1 }`.
/// - Once circulation has ended (by either route above, or by
///   `state.ring.lap >= config.max_laps` from the start — the
///   `max_laps == 0` case, matching `test_integration_ring_max_laps_zero`):
///   [`RingPhase::Resolve`] once every name in `state.agents` has cast a
///   vote (a key in `state.ring.votes`), [`RingPhase::Vote`] otherwise.
///
/// # Edge cases
/// - **Zero agents** (`state.agents.is_empty()`): a lap with zero required
///   contributions is vacuously complete, and "every contribution is
///   `Pass`" is vacuously true over an empty set — circulation ends on the
///   very first check, without ever incrementing `lap` (matching
///   `run_ring`'s `for` loop over zero agents, which never sets
///   `any_substantive` and breaks immediately). Likewise, "every agent has
///   voted" is vacuously true with zero agents, so the phase resolves
///   straight to [`RingPhase::Resolve`] without passing through
///   [`RingPhase::Vote`] — legacy still transiently sets
///   `TokenStatus::Voting` before an instant, agent-less voting loop, but
///   there is no *observable* `ExecutionState` in which that phase has any
///   agent left to vote for, so collapsing it here is a faithful,
///   documented simplification (this pure function has no notion of "phase
///   already visited", only of the state as given).
/// - **`state.ring.contributions`/`state.ring.votes` containing entries for
///   agents outside `state.agents`** (a malformed/partial/synthetic state):
///   ignored — lap-completeness and vote-completeness are both checked
///   against `state.agents`, so a stray, unroute-able contribution or vote
///   from an unknown name can never mask an eligible agent's absence nor
///   count towards its completion.
pub fn ring_phase(state: &ExecutionState, config: &RingConfig) -> RingPhase {
    if state.status != RunStatus::Running {
        return RingPhase::Done;
    }

    let agents = &state.agents;
    let lap = state.ring.lap;

    if lap < config.max_laps {
        let lap_contribs: Vec<_> = state
            .ring
            .contributions
            .iter()
            .filter(|c| c.lap == lap)
            .collect();

        let lap_complete = agents
            .iter()
            .all(|a| lap_contribs.iter().any(|c| &c.agent == a));

        if !lap_complete {
            return RingPhase::Circulate { lap };
        }

        // Lap complete: check the early-convergence exit (every contribution
        // this lap is a `Pass`) before deciding whether another lap follows.
        let all_pass = lap_contribs.iter().all(|c| c.action == "pass");
        if !all_pass && lap + 1 < config.max_laps {
            return RingPhase::Circulate { lap: lap + 1 };
        }
        // Either the lap converged early, or this was the last configured
        // lap: circulation is over — fall through to vote/resolve below.
    }

    let votes_complete = agents.iter().all(|a| state.ring.votes.contains_key(a));
    if votes_complete {
        RingPhase::Resolve
    } else {
        RingPhase::Vote
    }
}

/// Next agent to speak in the circulation, given the configured
/// `agent_order`.
///
/// Position in the lap is the number of contributions already recorded for
/// `state.ring.lap`, modulo `agent_order.len()` — the same indexing
/// `run_ring`'s `for (pos, agent) in agents.iter().enumerate()` uses (`pos`
/// is the loop index within the current lap, and `agent_order` here plays
/// the role of that `agents` slice, addressed by name rather than by
/// `Arc<dyn RingAgent>`).
///
/// Returns `None` for an empty `agent_order` (nothing to rotate through).
/// Not meaningful to call once the current lap is already complete (per
/// [`ring_phase`]) — the modulo wraps back to the start of `agent_order`
/// rather than reporting "nothing left", since this helper has no way to
/// signal that distinctly from "next agent is the first one again"; callers
/// are expected to check [`ring_phase`] first.
pub fn next_ring_agent(state: &ExecutionState, agent_order: &[String]) -> Option<String> {
    if agent_order.is_empty() {
        return None;
    }
    let lap = state.ring.lap;
    let contributed_this_lap = state
        .ring
        .contributions
        .iter()
        .filter(|c| c.lap == lap)
        .count();
    let idx = contributed_this_lap % agent_order.len();
    agent_order.get(idx).cloned()
}

/// Normalised string similarity (1.0 = identical, 0.0 = completely
/// different).
///
/// Verbatim copy of `core::orchestration::ring::position_similarity` — a
/// word-overlap Jaccard coefficient on lowercased words. Duplicated rather
/// than shared (same rationale as `es::blackboard`/`es::hierarchical`'s own
/// duplicated `parse_routed_tier`: the pure event-sourced module and the
/// legacy engine have no common trait to hang a shared implementation off
/// today, and strict coexistence means this module doesn't reach into
/// `ring.rs`'s private items).
fn position_similarity(a: &str, b: &str) -> f32 {
    let words_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let words_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();
    if union == 0 {
        return 1.0; // both empty → identical
    }
    intersection as f32 / union as f32
}

/// Resolve `state.ring.votes` to the winning position's text, replicating
/// the grouping + tie-break half of `core::orchestration::ring::resolve_votes`.
///
/// Deliberately narrower than the legacy function: legacy returns a full
/// `RingOutcome` (`Consensus`/`Majority`/`NoConsensus`, each carrying a score
/// and — for `Majority` — the dissenting votes), classified against
/// `config.consensus_threshold`/`majority_threshold`. Classifying the
/// majority ratio into that three-way outcome is effect/decider-level
/// synthesis (a later task's concern, layered on top of this helper); this
/// function reproduces only the delicate, easy-to-get-wrong part — grouping
/// votes by position similarity and picking the winning group's
/// representative text — which is exactly the part the task brief singles
/// out as "le point le plus délicat".
///
/// Algorithm (identical to legacy, `ring.rs:387-429`):
/// 1. Iterate `state.ring.votes` in `BTreeMap` order (agent-name-sorted).
///    Each vote's lowercased position is compared, via
///    [`position_similarity`], against every existing group's
///    representative key (in the order those groups were first created); the
///    vote joins the first group whose representative meets
///    `config.similarity_threshold`, or starts a new group keyed by its own
///    lowercased position.
/// 2. Group weight = the sum of `vote_weights` (default `1.0` for an agent
///    absent from the map) of every vote in that group.
/// 3. The winning group is the one returned by `Iterator::max_by` comparing
///    group weights — **and therefore, on a tie, the *last* group in
///    iteration order wins**, not the first (`Iterator::max_by`'s documented
///    behavior: "If several elements are equally maximum, the last element
///    is returned"). Groups are keyed by their (lowercased) representative
///    position text in a `BTreeMap`, so iteration order is alphabetical —
///    this is the exact ordering fidelity the task brief calls for
///    ("reproduis l'ordre EXACT du legacy").
/// 4. The resolution string is the *original-case* `position` of the first
///    vote ever inserted into the winning group (`largest_group[0]`) — the
///    representative, not the lowercased grouping key.
///
/// Returns an empty `String` when `state.ring.votes` is empty (no votes to
/// resolve — legacy's empty-votes case returns `RingOutcome::NoConsensus`
/// with no representative position at all; there is no meaningful
/// "resolution text" for this pure helper to produce, so it returns the
/// empty string rather than panicking or fabricating one).
pub fn resolve_votes(
    state: &ExecutionState,
    vote_weights: &BTreeMap<String, f32>,
    config: &RingConfig,
) -> String {
    if state.ring.votes.is_empty() {
        return String::new();
    }

    let weight_of = |name: &str| -> f32 { vote_weights.get(name).copied().unwrap_or(1.0) };

    let mut groups: BTreeMap<String, Vec<(String, &VoteRec)>> = BTreeMap::new();
    let mut group_reps: Vec<String> = Vec::new();

    for (agent, vote) in &state.ring.votes {
        let pos_lower = vote.position.to_lowercase();
        let mut assigned = false;
        for rep in &group_reps {
            if position_similarity(&pos_lower, rep) >= config.similarity_threshold {
                // SAFETY: `rep` always exists in `groups` because it comes
                // from `group_reps`, which only ever tracks keys already
                // inserted into `groups` below.
                groups
                    .get_mut(rep)
                    .expect("group representative must exist in groups map")
                    .push((agent.clone(), vote));
                assigned = true;
                break;
            }
        }
        if !assigned {
            group_reps.push(pos_lower.clone());
            groups
                .entry(pos_lower)
                .or_default()
                .push((agent.clone(), vote));
        }
    }

    // SAFETY: `groups` is non-empty because `state.ring.votes` is non-empty
    // (early return above).
    let largest_group = groups
        .values()
        .max_by(|a, b| {
            let wa: f32 = a.iter().map(|(n, _)| weight_of(n)).sum();
            let wb: f32 = b.iter().map(|(n, _)| weight_of(n)).sum();
            wa.partial_cmp(&wb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("groups must be non-empty because votes is non-empty");

    largest_group[0].1.position.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::es::event::ExecutionEvent as E;
    use crate::core::orchestration::es::state::fold;

    fn run_started(agents: &[&str]) -> E {
        E::RunStarted {
            run_id: "r".into(),
            pattern: "ring".into(),
            agents: agents.iter().map(|a| a.to_string()).collect(),
            input: "task".into(),
            project: None,
        }
    }

    fn contribution(agent: &str, lap: u32, position: usize, action: &str) -> E {
        E::ContributionAdded {
            agent: agent.to_string(),
            lap,
            position,
            action: action.to_string(),
            content: "c".to_string(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
        }
    }

    fn vote(agent: &str, position: &str, confidence: f32) -> E {
        E::VoteCast {
            agent: agent.to_string(),
            position: position.to_string(),
            confidence,
            supports: vec![],
            concerns: vec![],
        }
    }

    // ── ring_phase ────────────────────────────────────────────────────

    #[test]
    fn ring_phase_circulation_partial_stays_on_current_lap() {
        let state = fold(&[run_started(&["a", "b"]), contribution("a", 0, 0, "propose")]);
        let config = RingConfig::default();
        assert_eq!(ring_phase(&state, &config), RingPhase::Circulate { lap: 0 });
    }

    #[test]
    fn ring_phase_circulation_complete_advances_to_next_lap() {
        // max_laps default is 3, lap 0 fully contributed (non-pass) → lap 1.
        let state = fold(&[
            run_started(&["a", "b"]),
            contribution("a", 0, 0, "propose"),
            contribution("b", 0, 1, "propose"),
        ]);
        let config = RingConfig::default();
        assert_eq!(ring_phase(&state, &config), RingPhase::Circulate { lap: 1 });
    }

    #[test]
    fn ring_phase_last_lap_complete_moves_to_vote() {
        // max_laps = 1: lap 0 is the only lap. Once it's complete (and not
        // all-pass), there is no lap 1 to advance to — straight to Vote.
        let config = RingConfig {
            max_laps: 1,
            ..RingConfig::default()
        };
        let state = fold(&[
            run_started(&["a", "b"]),
            contribution("a", 0, 0, "propose"),
            contribution("b", 0, 1, "propose"),
        ]);
        assert_eq!(ring_phase(&state, &config), RingPhase::Vote);
    }

    #[test]
    fn ring_phase_all_pass_exits_circulation_early() {
        // max_laps = 3, but every contribution this lap is a Pass → early
        // convergence exit straight to Vote, never advancing to lap 1.
        let state = fold(&[
            run_started(&["a", "b"]),
            contribution("a", 0, 0, "pass"),
            contribution("b", 0, 1, "pass"),
        ]);
        let config = RingConfig::default();
        assert_eq!(ring_phase(&state, &config), RingPhase::Vote);
    }

    #[test]
    fn ring_phase_vote_phase_partial_votes() {
        let config = RingConfig {
            max_laps: 1,
            ..RingConfig::default()
        };
        let state = fold(&[
            run_started(&["a", "b"]),
            contribution("a", 0, 0, "propose"),
            contribution("b", 0, 1, "propose"),
            vote("a", "Use Rust", 0.9),
        ]);
        assert_eq!(ring_phase(&state, &config), RingPhase::Vote);
    }

    #[test]
    fn ring_phase_resolve_once_every_agent_voted() {
        let config = RingConfig {
            max_laps: 1,
            ..RingConfig::default()
        };
        let state = fold(&[
            run_started(&["a", "b"]),
            contribution("a", 0, 0, "propose"),
            contribution("b", 0, 1, "propose"),
            vote("a", "Use Rust", 0.9),
            vote("b", "Use Rust", 0.8),
        ]);
        assert_eq!(ring_phase(&state, &config), RingPhase::Resolve);
    }

    #[test]
    fn ring_phase_max_laps_zero_goes_straight_to_vote_or_resolve() {
        // Matches `test_integration_ring_max_laps_zero`: no circulation at
        // all when max_laps == 0.
        let config = RingConfig {
            max_laps: 0,
            ..RingConfig::default()
        };
        let state = fold(&[run_started(&["a"])]);
        assert_eq!(ring_phase(&state, &config), RingPhase::Vote);
    }

    #[test]
    fn ring_phase_zero_agents_resolves_immediately() {
        // Vacuous completeness on both the lap and the votes side, per the
        // documented edge case.
        let config = RingConfig::default();
        let state = fold(&[run_started(&[])]);
        assert_eq!(ring_phase(&state, &config), RingPhase::Resolve);
    }

    #[test]
    fn ring_phase_done_when_status_not_running() {
        let config = RingConfig::default();
        let state = fold(&[
            run_started(&["a", "b"]),
            E::Halted {
                reason: "budget".into(),
            },
        ]);
        assert_eq!(ring_phase(&state, &config), RingPhase::Done);
    }

    // ── next_ring_agent ───────────────────────────────────────────────

    #[test]
    fn next_ring_agent_starts_at_first_agent() {
        let state = fold(&[run_started(&["a", "b", "c"])]);
        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(next_ring_agent(&state, &order), Some("a".to_string()));
    }

    #[test]
    fn next_ring_agent_rotates_after_a_contribution() {
        let state = fold(&[
            run_started(&["a", "b", "c"]),
            contribution("a", 0, 0, "propose"),
        ]);
        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(next_ring_agent(&state, &order), Some("b".to_string()));
    }

    #[test]
    fn next_ring_agent_wraps_around_after_a_full_lap() {
        let state = fold(&[
            run_started(&["a", "b"]),
            contribution("a", 0, 0, "propose"),
            contribution("b", 0, 1, "propose"),
        ]);
        let order = vec!["a".to_string(), "b".to_string()];
        assert_eq!(next_ring_agent(&state, &order), Some("a".to_string()));
    }

    #[test]
    fn next_ring_agent_empty_order_is_none() {
        let state = fold(&[run_started(&["a"])]);
        assert_eq!(next_ring_agent(&state, &[]), None);
    }

    // ── resolve_votes ─────────────────────────────────────────────────

    #[test]
    fn resolve_votes_empty_is_empty_string() {
        let state = fold(&[run_started(&["a"])]);
        let config = RingConfig::default();
        assert_eq!(resolve_votes(&state, &BTreeMap::new(), &config), "");
    }

    #[test]
    fn resolve_votes_single_position_wins_outright() {
        let state = fold(&[
            run_started(&["a", "b", "c"]),
            vote("a", "Rust/Axum", 0.9),
            vote("b", "Rust/Axum", 0.9),
            vote("c", "Rust/Axum", 0.9),
        ]);
        let config = RingConfig::default();
        assert_eq!(
            resolve_votes(&state, &BTreeMap::new(), &config),
            "Rust/Axum"
        );
    }

    #[test]
    fn resolve_votes_largest_weighted_group_wins() {
        // "Option A" carries 3 votes at default weight 1.0 (=3.0), "Option B"
        // carries 1 vote weighted heavily (=5.0) — the weighted group must
        // win even though it has fewer members.
        let state = fold(&[
            run_started(&["a", "b", "c", "d"]),
            vote("a", "Option A", 0.8),
            vote("b", "Option A", 0.8),
            vote("c", "Option A", 0.8),
            vote("d", "Option B", 0.7),
        ]);
        let mut weights = BTreeMap::new();
        weights.insert("d".to_string(), 5.0);
        let config = RingConfig::default();
        assert_eq!(resolve_votes(&state, &weights, &config), "Option B");
    }

    /// THE property to lock in: on a tied group weight, `resolve_votes` must
    /// return the representative of the *last* group in (alphabetical,
    /// `BTreeMap`-ordered) iteration order — reproducing `Iterator::max_by`'s
    /// documented "last element wins on a tie" behavior, not the first.
    #[test]
    fn resolve_votes_tie_break_returns_last_max_group_not_first() {
        // Two groups, "alpha" and "beta" (lowercased grouping keys), each
        // with 2 votes at default weight 1.0 → tied group weight (2.0 each).
        // `groups.values()` iterates in BTreeMap key order: "alpha" before
        // "beta". A tie must resolve to "beta" (the last group), never
        // "alpha" (the first).
        let state = fold(&[
            run_started(&["agent-a", "agent-b", "agent-c", "agent-d"]),
            vote("agent-a", "Alpha", 0.9),
            vote("agent-b", "Alpha", 0.9),
            vote("agent-c", "Beta", 0.9),
            vote("agent-d", "Beta", 0.9),
        ]);
        let config = RingConfig::default();
        assert_eq!(resolve_votes(&state, &BTreeMap::new(), &config), "Beta");
    }

    #[test]
    fn resolve_votes_representative_is_first_vote_inserted_in_group() {
        // Within the winning group, the representative text is the
        // *original-case* position of the first vote inserted (agent-name
        // order), not any later vote's differently-cased but
        // similarity-matching position text.
        let state = fold(&[
            run_started(&["agent-a", "agent-b"]),
            vote("agent-a", "Use Rust for the backend", 0.9),
            vote("agent-b", "use rust for the backend", 0.8),
        ]);
        let config = RingConfig::default();
        assert_eq!(
            resolve_votes(&state, &BTreeMap::new(), &config),
            "Use Rust for the backend"
        );
    }
}
