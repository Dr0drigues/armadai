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
use std::sync::Arc;

use async_trait::async_trait;

use super::engine::{Action, Decider, EffectRunner};
use super::event::ExecutionEvent;
use super::state::{ExecutionState, RunStatus, VoteRec};
use crate::core::agent::Agent;
use crate::core::orchestration::llm_agents::{
    RING_ACTION_INSTRUCTIONS, parse_ring_action, parse_vote_confidence,
};
use crate::core::orchestration::ring::{ContributionAction, RingConfig};
use crate::core::routing::{BudgetState, RoutingRules, route};
#[cfg(test)]
use crate::linker::model_resolution::fallback_model_for_tier;
use crate::linker::model_resolution::{ModelTier, resolve_model_for_tier};
use crate::providers::traits::{ChatMessage, CompletionRequest, Provider};

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

// ── RingDecider (Task 7): pure decision function ──────────────────

/// Best-effort partial digest of the run so far, built from every recorded
/// ring contribution (`state.ring.contributions`, in append order — the
/// chronological order the underlying event log recorded them), formatted as
/// `[agent] content` per line. Ring counterpart of
/// `es::blackboard::build_board_result`/`es::hierarchical::build_partial_content`,
/// used as the fallback content for a guard-triggered `Complete` when no vote
/// has been cast yet (see [`RingDecider::partial_or_outcome`]).
fn build_ring_partial(state: &ExecutionState) -> String {
    state
        .ring
        .contributions
        .iter()
        .map(|c| format!("[{}] {}", c.agent, c.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pure ring [`Decider`]: given the current [`ExecutionState`], decides the
/// next batch of [`Action`]s across the pattern's three phases — circulation
/// (`Circulate`), voting (`Vote`), and resolution (`Resolve`) — derived via
/// [`ring_phase`].
///
/// Mirrors `HierarchicalDecider`/`BlackboardDecider` (`es::hierarchical`/
/// `es::blackboard`, OH1 Lot 2/3): all fields are immutable inputs captured
/// at construction time, `decide` performs no I/O and reads no mutable
/// state — every decision is a pure function of `state`, which is what keeps
/// event-log replay deterministic.
///
/// # Roster contract
/// `agent_order` must be the same set, in the same order, as `state.agents`
/// (the run's roster, seeded from `RunStarted { agents, .. }`). This decider
/// never checks or reconciles the two — it is the caller's responsibility
/// (the future `run_ring_es`, Task 9's assembly function) to construct both
/// from a single source of truth. [`ring_phase`]'s lap-completeness check
/// reads `state.agents`; [`next_ring_agent`] and this decider's own
/// vote-completeness check (`Vote`/`Resolve` branches) read `agent_order` —
/// a mismatch between the two would desynchronize "who still owes a
/// contribution/vote" from "who gets invoked next".
#[derive(Debug, Clone)]
pub struct RingDecider {
    /// All known agents by name, for model/tag lookups (routing).
    pub agents: BTreeMap<String, Agent>,
    /// Declared agent order — the circulation/voting rotation. Must match
    /// `state.agents` (see the roster contract above).
    pub agent_order: Vec<String>,
    /// The original user input/task, given to every invoked agent.
    pub input: String,
    /// Ring configuration (`max_laps`/`similarity_threshold`/…), read by
    /// [`ring_phase`] and [`resolve_votes`].
    pub config: RingConfig,
    /// Routing rules for `latest:auto` agents.
    pub routing_rules: RoutingRules,
    /// Max laps before the run is force-completed, independent of
    /// `config.max_laps` (which only drives [`ring_phase`]'s circulation →
    /// vote transition). Kept as its own field — like
    /// `BlackboardDecider::max_rounds` alongside `BlackboardConfig::max_rounds`
    /// — so callers may cap circulation more aggressively than the pattern's
    /// own configured lap count without touching `config`.
    pub max_laps: u32,
    /// Per-agent vote weight (default `1.0` when absent), read by
    /// [`resolve_votes`].
    pub vote_weights: BTreeMap<String, f32>,
    /// Optional total token budget (in + out) before the run is
    /// force-completed.
    pub token_budget: Option<u32>,
    /// Optional total cost budget (USD) before the run is force-completed.
    pub cost_limit: Option<f64>,
}

impl RingDecider {
    /// Construct a new `RingDecider`. All arguments become immutable fields
    /// read by `decide`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agents: BTreeMap<String, Agent>,
        agent_order: Vec<String>,
        input: impl Into<String>,
        config: RingConfig,
        routing_rules: RoutingRules,
        max_laps: u32,
        vote_weights: BTreeMap<String, f32>,
        token_budget: Option<u32>,
        cost_limit: Option<f64>,
    ) -> Self {
        Self {
            agents,
            agent_order,
            input: input.into(),
            config,
            routing_rules,
            max_laps,
            vote_weights,
            token_budget,
            cost_limit,
        }
    }

    /// Check budget/cost guards, returning the `Warned` code for whichever
    /// one has been breached (token budget checked first — same convention
    /// as `HierarchicalDecider::breached_limit`/`BlackboardDecider::breached_budget`).
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

    /// Best-effort final content for a guard-triggered `Complete`: the
    /// resolved outcome if at least one vote has been cast (even a partial,
    /// not-yet-complete vote round — [`resolve_votes`] tolerates that), or
    /// else a digest of every ring contribution so far
    /// ([`build_ring_partial`]). Matches the brief's "content: partiel ou
    /// outcome".
    fn partial_or_outcome(&self, state: &ExecutionState) -> String {
        let outcome = resolve_votes(state, &self.vote_weights, &self.config);
        if outcome.is_empty() {
            build_ring_partial(state)
        } else {
            outcome
        }
    }

    /// If `agent_name` is a known agent configured with the exact
    /// `"latest:auto"` model placeholder, resolve the tier for
    /// `routing_input` (pure, via `crate::core::routing::route`) and return
    /// the `ModelRouted` event to emit before invoking it. Concrete models,
    /// other `latest:*` placeholders, and unknown agents all return `None`.
    ///
    /// Identical in spirit to `HierarchicalDecider`/`BlackboardDecider`'s own
    /// `model_routed_event` — duplicated rather than shared (same rationale:
    /// the three deciders' `agents`/`routing_rules`/`token_budget` fields
    /// live on unrelated structs with no common trait today).
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

    /// Build the action batch for circulation phase `lap`.
    ///
    /// **`LapStarted`-before-rotation contract**: whether `lap` has just
    /// begun is decided purely from `state.ring.contributions` — no
    /// contribution recorded yet for `lap` — rather than from
    /// `state.ring.lap` (which still holds the *previous* lap's number until
    /// the `LapStarted { lap }` this same batch emits is folded). When it has
    /// just begun, `Emit(LapStarted { lap })` is pushed **first**, ahead of
    /// anything else: [`next_ring_agent`] indexes by `state.ring.lap`, so
    /// computing it against the raw (stale) `state` here would rotate off
    /// the *previous* lap's contribution count instead of the new lap's
    /// (vacuously `0`). We therefore compute the next agent against a
    /// `state.ring.lap`-corrected lookahead clone — pure, local, no mutation
    /// of the real state (mirrors `BlackboardDecider::decide`'s own
    /// round-advance lookahead) — rather than against `state` directly, so
    /// the very first invocation of a new lap always resolves to
    /// `agent_order[0]` regardless of whether `LapStarted` has actually been
    /// folded into `state` yet.
    fn circulate_actions(&self, lap: u32, state: &ExecutionState) -> Vec<Action> {
        let lap_just_started = !state.ring.contributions.iter().any(|c| c.lap == lap);

        let mut lookahead = state.clone();
        lookahead.ring.lap = lap;
        let agent = next_ring_agent(&lookahead, &self.agent_order);

        let mut actions = Vec::new();
        if lap_just_started {
            actions.push(Action::Emit(ExecutionEvent::LapStarted { lap }));
        }
        if let Some(agent) = agent {
            if let Some(event) = self.model_routed_event(&agent, &self.input, state) {
                actions.push(Action::Emit(event));
            }
            actions.push(Action::Invoke {
                agent,
                input: self.input.clone(),
            });
        }
        actions
    }

    /// Build the action batch for the voting phase: `Invoke` the first agent
    /// in `agent_order` that has not cast a vote yet (`state.ring.votes`).
    /// Covers both the circulation→vote transition (nobody has voted:
    /// resolves to `agent_order[0]`) and an in-progress vote round (resolves
    /// to the next non-voter) — the same lookup serves both, since "first
    /// non-voter in roster order" is exactly "next votant" either way.
    ///
    /// Returns an empty batch if every agent has already voted — defensive
    /// only: `ring_phase` returns `RingPhase::Resolve` in that case, so
    /// `decide` never calls this with an exhausted roster in practice.
    fn vote_actions(&self, state: &ExecutionState) -> Vec<Action> {
        let Some(agent) = self
            .agent_order
            .iter()
            .find(|a| !state.ring.votes.contains_key(*a))
            .cloned()
        else {
            return Vec::new();
        };

        let mut actions = Vec::new();
        if let Some(event) = self.model_routed_event(&agent, &self.input, state) {
            actions.push(Action::Emit(event));
        }
        actions.push(Action::Invoke {
            agent,
            input: self.input.clone(),
        });
        actions
    }
}

impl Decider for RingDecider {
    fn decide(&self, state: &ExecutionState) -> Vec<Action> {
        // Budget/cost guards apply regardless of phase — a hard external
        // limit that can be hit mid-circulation or mid-voting alike (same
        // priority convention as `BlackboardDecider::decide`: checked ahead
        // of any lap/round cap).
        if let Some(code) = self.breached_budget(state) {
            return vec![
                Action::Emit(ExecutionEvent::Warned {
                    code: code.to_string(),
                }),
                Action::Complete {
                    content: self.partial_or_outcome(state),
                },
            ];
        }

        match ring_phase(state, &self.config) {
            RingPhase::Circulate { lap } => {
                // Independent cap: `self.max_laps` (not `config.max_laps`,
                // which only drives `ring_phase`'s own circulation → vote
                // transition — see the field doc comment above) stops
                // circulation early, without having converged, when it is
                // set below what `config` would otherwise allow.
                if lap >= self.max_laps {
                    return vec![
                        Action::Emit(ExecutionEvent::Warned {
                            code: "max_laps".to_string(),
                        }),
                        Action::Complete {
                            content: self.partial_or_outcome(state),
                        },
                    ];
                }
                self.circulate_actions(lap, state)
            }
            RingPhase::Vote => self.vote_actions(state),
            RingPhase::Resolve => {
                let outcome = resolve_votes(state, &self.vote_weights, &self.config);
                vec![
                    Action::Emit(ExecutionEvent::OutcomeResolved {
                        outcome: outcome.clone(),
                    }),
                    Action::Complete { content: outcome },
                ]
            }
            // The generic loop (`run_event_sourced`) only calls `decide`
            // while `state.status == RunStatus::Running`, so this should be
            // unreachable in practice (a `Done` phase implies the run has
            // already terminated) — kept as an explicit, harmless empty
            // batch for exhaustiveness rather than a `match` catch-all that
            // could silently swallow a future `RingPhase` variant.
            RingPhase::Done => Vec::new(),
        }
    }
}

/// Parse a tier string as stored in `ExecutionState::routed_tiers` back into
/// a `ModelTier`.
///
/// Identical in spirit to `es::hierarchical::parse_routed_tier`/
/// `es::blackboard::parse_routed_tier` — duplicated rather than shared (same
/// rationale: the three effect runners' `agents`/model-resolution concerns
/// live on unrelated structs with no common trait today). Unrecognized
/// strings fall back to `Pro`, matching the other two.
fn parse_routed_tier(tier: &str) -> ModelTier {
    match tier.to_lowercase().as_str() {
        "fast" => ModelTier::Fast,
        "max" => ModelTier::Max,
        _ => ModelTier::Pro,
    }
}

/// Map a parsed [`ContributionAction`] onto the lowercase `action` string
/// carried by `ExecutionEvent::ContributionAdded` — the single source of
/// truth [`ring_phase`]/[`next_ring_agent`] read back (via
/// `ContribRec::action`) to detect early-convergence (every contribution in
/// a lap is exactly `"pass"`) and to rotate the circulation. **Contract**:
/// the `Pass` arm must map to the exact lowercase string `"pass"` — nothing
/// else — since that is the literal `ring_phase` compares against
/// (`lap_contribs.iter().all(|c| c.action == "pass")`); any other casing or
/// wording would silently break the early-exit detection.
fn action_string(action: &ContributionAction) -> &'static str {
    match action {
        ContributionAction::Propose => "propose",
        ContributionAction::Enrich { .. } => "enrich",
        ContributionAction::Contest { .. } => "contest",
        ContributionAction::Endorse { .. } => "endorse",
        ContributionAction::Synthesize => "synthesize",
        ContributionAction::Pass { .. } => "pass",
    }
}

// ── RingEffectRunner (Task 8): the sole async/impure effect ───────────

/// Executes the actual LLM call behind `Action::Invoke` for the ring pattern
/// and turns the raw provider response into the `ContributionAdded` (during
/// circulation) or `VoteCast` (during voting) event the pure loop/
/// [`RingDecider`] expect, per the current [`RingPhase`] (derived via
/// [`ring_phase`]).
///
/// This is the *only* impure/async piece of the event-sourced ring engine —
/// every other function in this module is a pure, synchronous helper over
/// `ExecutionState`. Coexists with the legacy
/// `core::orchestration::ring::run_ring`/`llm_agents::LlmRingAgent` (this
/// struct is not wired into it, and does not import from it beyond the
/// plain, side-effect-free `parse_ring_action`/`parse_vote_confidence`
/// parsers and `RING_ACTION_INSTRUCTIONS` constant it explicitly reuses —
/// strict coexistence, mirroring `HierarchicalEffectRunner`/
/// `BlackboardEffectRunner`).
pub struct RingEffectRunner {
    /// All known agents by name (system prompt, model, temperature, …).
    pub agents: BTreeMap<String, Agent>,
    /// Provider instance per agent name.
    pub providers: BTreeMap<String, Arc<dyn Provider>>,
    /// Ring configuration, read by [`ring_phase`] to derive the current
    /// phase for `agent`.
    pub config: RingConfig,
    /// Per-agent vote weight — reserved for a future synthesis/resolution
    /// step; `run_invoke` itself doesn't read it (voting only records each
    /// agent's own position/confidence, unweighted — weighting happens in
    /// [`resolve_votes`], the `Decider`'s concern), kept here for symmetry
    /// with [`RingDecider::vote_weights`] and so callers can construct both
    /// from a single source of truth.
    pub vote_weights: BTreeMap<String, f32>,
}

impl RingEffectRunner {
    /// Construct a new `RingEffectRunner` from its immutable inputs.
    pub fn new(
        agents: BTreeMap<String, Agent>,
        providers: BTreeMap<String, Arc<dyn Provider>>,
        config: RingConfig,
        vote_weights: BTreeMap<String, f32>,
    ) -> Self {
        Self {
            agents,
            providers,
            config,
            vote_weights,
        }
    }

    /// Assemble the circulation prompt for `agent_name` at `lap`, reproducing
    /// the shape of `LlmRingAgent::process`'s `user_msg`
    /// (`core::orchestration::llm_agents`) over the event-sourced
    /// `state.ring` projection instead of a live `TokenSnapshot`: `"Task:
    /// …\nLap: …\nYour position: …/…\n"`, then (only if any exist) a
    /// `"Previous contributions:\n"` section listing *every* contribution
    /// recorded so far (`state.ring.contributions`, in append order) as
    /// `"- [#{index} Lap {lap} / {position}] {agent}: {content}\n"`, then
    /// [`RING_ACTION_INSTRUCTIONS`] verbatim.
    ///
    /// `position` (0-based, "your position in *this* lap") is the number of
    /// contributions already recorded for `lap` — the same count
    /// [`next_ring_agent`] and `ring_phase`'s lap-completeness check use, and
    /// exactly the index this invocation's own `ContributionAdded` will
    /// carry once it completes (see `run_invoke`'s `Circulate` branch below).
    /// The roster size (`state.agents.len()`) stands in for
    /// `TokenSnapshot::ring_order.len()` — the total agents.
    fn build_circulate_prompt(&self, input: &str, lap: u32, state: &ExecutionState) -> String {
        let position = state
            .ring
            .contributions
            .iter()
            .filter(|c| c.lap == lap)
            .count();
        let mut user_msg = format!(
            "Task: {input}\nLap: {lap}\nYour position: {}/{}\n",
            position + 1,
            state.agents.len()
        );

        if !state.ring.contributions.is_empty() {
            user_msg.push_str("\nPrevious contributions:\n");
            for (i, c) in state.ring.contributions.iter().enumerate() {
                user_msg.push_str(&format!(
                    "- [#{i} Lap {} / {}] {}: {}\n",
                    c.lap, c.position, c.agent, c.content
                ));
            }
        }

        user_msg.push_str(RING_ACTION_INSTRUCTIONS);
        user_msg
    }

    /// Assemble the voting prompt, reproducing the shape of
    /// `LlmRingAgent::vote`'s `user_msg` verbatim: `"Task: …\n\nAll
    /// contributions:\n"`, one `"- [Lap … / …] {agent}: {content}\n"` line
    /// per recorded contribution (`state.ring.contributions`, in append
    /// order), then the fixed synthesis instructions asking for a
    /// `CONFIDENCE: <0.0-1.0>` header followed by the agent's final
    /// position.
    fn build_vote_prompt(&self, input: &str, state: &ExecutionState) -> String {
        let mut user_msg = format!("Task: {input}\n\nAll contributions:\n");
        for c in &state.ring.contributions {
            user_msg.push_str(&format!(
                "- [Lap {} / {}] {}: {}\n",
                c.lap, c.position, c.agent, c.content
            ));
        }
        user_msg.push_str(
            "\nSynthesize the contributions above. Identify areas of agreement, \
             unresolved disagreements, and any gaps. Then state your final \
             position in one or two sentences.\n\n\
             Format your response as:\n\
             CONFIDENCE: <0.0-1.0>\n\
             <your synthesized position>",
        );
        user_msg
    }
}

#[async_trait]
impl EffectRunner for RingEffectRunner {
    async fn run_invoke(
        &self,
        agent: &str,
        input: &str,
        state: &ExecutionState,
    ) -> anyhow::Result<ExecutionEvent> {
        let agent_def = self
            .agents
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("Unknown agent '{agent}' — no Agent definition"))?;
        let provider = self
            .providers
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("No provider configured for agent '{agent}'"))?;

        // Same `"latest:auto"` resolution as `HierarchicalEffectRunner`/
        // `BlackboardEffectRunner`: see their doc comments for the full
        // rationale. `RingDecider` (Task 7) emits `ModelRouted` ahead of
        // every `latest:auto` agent's `Invoke`, so `state.routed_tiers`
        // should already carry its tier by the time this runs; the `None`
        // branch is a defensive fallback for a hand-built state (tests) or a
        // future decider regression.
        let raw_model = agent_def
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let model = if raw_model == "latest:auto" {
            let tier = match state.routed_tiers.get(agent) {
                Some(tier_str) => parse_routed_tier(tier_str),
                None => {
                    tracing::warn!(
                        agent,
                        "no ModelRouted tier recorded for latest:auto agent; falling back to Pro tier"
                    );
                    ModelTier::Pro
                }
            };
            resolve_model_for_tier(&agent_def.metadata.provider, tier)
        } else {
            raw_model
        };

        match ring_phase(state, &self.config) {
            RingPhase::Circulate { lap } => {
                let position = state
                    .ring
                    .contributions
                    .iter()
                    .filter(|c| c.lap == lap)
                    .count();
                let prompt = self.build_circulate_prompt(input, lap, state);
                let request = CompletionRequest {
                    model,
                    system_prompt: agent_def.system_prompt.clone(),
                    messages: vec![ChatMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }],
                    temperature: agent_def.metadata.temperature,
                    max_tokens: agent_def.metadata.max_tokens,
                };

                // Graceful degradation (brief, "erreur provider"): a provider
                // error must NOT abort the run — the ring must keep
                // circulating even when one agent's LLM call fails outright.
                // Swallow the error and manufacture a degraded
                // `ContributionAdded` with `action: "pass"` (the exact
                // lowercase string `ring_phase`'s early-convergence check
                // compares against — see `action_string`'s doc comment), a
                // `"[agent failed]"` marker so the failure is visible in the
                // transcript, and zero tokens/cost (nothing was actually
                // consumed/billed).
                match provider.complete(request).await {
                    Ok(response) => {
                        let (action, content) = parse_ring_action(&response.content);
                        Ok(ExecutionEvent::ContributionAdded {
                            agent: agent.to_string(),
                            lap,
                            position,
                            action: action_string(&action).to_string(),
                            content,
                            tokens_in: response.tokens_in,
                            tokens_out: response.tokens_out,
                            cost: response.cost,
                        })
                    }
                    Err(err) => {
                        tracing::warn!(
                            agent,
                            error = %err,
                            "ring agent provider call failed during circulation; recording a Pass instead of aborting the run"
                        );
                        Ok(ExecutionEvent::ContributionAdded {
                            agent: agent.to_string(),
                            lap,
                            position,
                            action: "pass".to_string(),
                            content: "[agent failed]".to_string(),
                            tokens_in: 0,
                            tokens_out: 0,
                            cost: 0.0,
                        })
                    }
                }
            }
            // `Vote` is the only other phase `RingDecider` ever dispatches an
            // `Invoke` for (`Resolve`/`Done` only ever `Emit`/`Complete`, never
            // `Invoke` — see `RingDecider::decide`); the catch-all below is a
            // defensive fallback for a hand-built state (tests) or a future
            // decider regression, treated identically to `Vote` since both
            // have nothing left to circulate.
            _ => {
                let n = state.ring.contributions.len();
                let prompt = self.build_vote_prompt(input, state);
                let request = CompletionRequest {
                    model,
                    system_prompt: agent_def.system_prompt.clone(),
                    messages: vec![ChatMessage {
                        role: "user".to_string(),
                        content: prompt,
                    }],
                    temperature: agent_def.metadata.temperature,
                    max_tokens: agent_def.metadata.max_tokens,
                };

                // Graceful degradation, voting phase: a provider error
                // records a neutral abstention (`confidence: 0.0`, a
                // documented `"[agent failed]"` position) rather than
                // aborting the run — fidelity with the circulation branch
                // above. `supports` still covers the full contribution range
                // (fidèle au legacy shape even for a degraded vote); `concerns`
                // documents the failure so it's distinguishable from a
                // genuine (if unconfident) vote.
                match provider.complete(request).await {
                    Ok(response) => {
                        let (confidence, position) = parse_vote_confidence(&response.content);
                        Ok(ExecutionEvent::VoteCast {
                            agent: agent.to_string(),
                            position,
                            confidence,
                            supports: (0..n).collect(),
                            concerns: vec![],
                        })
                    }
                    Err(err) => {
                        tracing::warn!(
                            agent,
                            error = %err,
                            "ring agent provider call failed during voting; recording a neutral abstention instead of aborting the run"
                        );
                        Ok(ExecutionEvent::VoteCast {
                            agent: agent.to_string(),
                            position: "[agent failed]".to_string(),
                            confidence: 0.0,
                            supports: (0..n).collect(),
                            concerns: vec!["provider error".to_string()],
                        })
                    }
                }
            }
        }
    }
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

    /// Review hardening (Task 6 minor): the tie-break must still pick the
    /// *last* group when there are three tied groups, not just two — proving
    /// `max_by`'s "last element wins" isn't an artifact of only ever
    /// comparing a pair.
    #[test]
    fn resolve_votes_tie_break_three_way_returns_last_key() {
        // Three single-word, mutually dissimilar positions ("alpha", "beta",
        // "gamma" — zero word overlap, so each starts its own group), each
        // with exactly one vote at the default weight 1.0 → all three groups
        // tied at weight 1.0. `groups.values()` iterates key-sorted:
        // "alpha", "beta", "gamma" — the tie must resolve to "Gamma", the
        // last key, never "Alpha" or "Beta".
        let state = fold(&[
            run_started(&["agent-a", "agent-b", "agent-c"]),
            vote("agent-a", "Alpha", 0.9),
            vote("agent-b", "Beta", 0.9),
            vote("agent-c", "Gamma", 0.9),
        ]);
        let config = RingConfig::default();
        assert_eq!(resolve_votes(&state, &BTreeMap::new(), &config), "Gamma");
    }

    /// Review hardening (Task 6 minor): the tie-break follows the *group
    /// key's* (lowercased position) `BTreeMap` order, not the order in which
    /// groups were first created while iterating `state.ring.votes` (itself
    /// agent-name-sorted). `agent-a` (sorted first) casts "Zebra" — so the
    /// "zebra" group is created *before* the "apple" group as iteration
    /// proceeds — yet "apple" < "zebra" alphabetically, so on a tie
    /// `groups.values()` must still resolve to "Zebra" (the alphabetically
    /// *last* key), proving iteration order followed by `max_by` is the
    /// `BTreeMap`'s key order, not creation/insertion order.
    #[test]
    fn resolve_votes_tie_break_follows_key_order_not_creation_order() {
        let state = fold(&[
            run_started(&["agent-a", "agent-b"]),
            vote("agent-a", "Zebra", 0.9),
            vote("agent-b", "Apple", 0.9),
        ]);
        let config = RingConfig::default();
        assert_eq!(resolve_votes(&state, &BTreeMap::new(), &config), "Zebra");
    }

    // ── RingDecider (Task 7) ──────────────────────────────────────────

    /// Tests for `RingDecider` (Task 7): pure decision function built on top
    /// of `ring_phase`/`next_ring_agent`/`resolve_votes`. Named `decide` so
    /// `cargo test es::ring::tests::decide` targets this module — mirrors
    /// the naming convention used by `es::hierarchical::tests::decide` and
    /// `es::blackboard::tests::decide`.
    mod decide {
        use super::*;
        use crate::core::agent::AgentMetadata;
        use crate::core::orchestration::es::engine::{Action, Decider};
        use crate::core::routing::RoutingRules;
        use std::path::PathBuf;

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

        #[allow(clippy::too_many_arguments)]
        fn test_decider(
            agent_names: &[&str],
            config: RingConfig,
            max_laps: u32,
            vote_weights: BTreeMap<String, f32>,
            token_budget: Option<u32>,
            cost_limit: Option<f64>,
        ) -> RingDecider {
            let mut agents = BTreeMap::new();
            for name in agent_names {
                agents.insert((*name).to_string(), test_agent(name, "concrete-model"));
            }
            RingDecider::new(
                agents,
                agent_names.iter().map(|a| (*a).to_string()).collect(),
                "task".to_string(),
                config,
                RoutingRules::default(),
                max_laps,
                vote_weights,
                token_budget,
                cost_limit,
            )
        }

        fn invoked_agent(actions: &[Action]) -> Option<&str> {
            actions.iter().find_map(|a| match a {
                Action::Invoke { agent, .. } => Some(agent.as_str()),
                _ => None,
            })
        }

        // (a) empty state → `LapStarted{0}` first, then `Invoke` of the
        // first agent in `agent_order`.
        #[test]
        fn empty_state_starts_lap_zero_and_invokes_first_agent() {
            let dec = test_decider(
                &["a", "b", "c"],
                RingConfig::default(),
                5,
                BTreeMap::new(),
                None,
                None,
            );
            let state = fold(&[run_started(&["a", "b", "c"])]);
            let actions = dec.decide(&state);

            assert!(
                matches!(&actions[0], Action::Emit(E::LapStarted { lap }) if *lap == 0),
                "expected Emit(LapStarted{{lap: 0}}) first, got {actions:?}"
            );
            assert_eq!(invoked_agent(&actions), Some("a"));
        }

        // (b) circulation in progress (one contribution already recorded
        // this lap) → no `LapStarted` re-emitted, `Invoke` of the next agent
        // in roster order.
        #[test]
        fn circulation_in_progress_invokes_next_agent_without_lap_started() {
            let dec = test_decider(
                &["a", "b", "c"],
                RingConfig::default(),
                5,
                BTreeMap::new(),
                None,
                None,
            );
            let state = fold(&[
                run_started(&["a", "b", "c"]),
                E::LapStarted { lap: 0 },
                contribution("a", 0, 0, "propose"),
            ]);
            let actions = dec.decide(&state);

            assert!(
                !actions
                    .iter()
                    .any(|a| matches!(a, Action::Emit(E::LapStarted { .. }))),
                "must not re-emit LapStarted mid-lap, got {actions:?}"
            );
            assert_eq!(actions.len(), 1, "got {actions:?}");
            assert_eq!(invoked_agent(&actions), Some("b"));
        }

        // (c) end of circulation (max_laps == 1, lap 0 fully contributed,
        // non-pass) → `Invoke` of the first voter (`agent_order[0]`), no
        // agent having voted yet.
        #[test]
        fn end_of_circulation_invokes_first_voter() {
            let config = RingConfig {
                max_laps: 1,
                ..RingConfig::default()
            };
            let dec = test_decider(&["a", "b"], config, 5, BTreeMap::new(), None, None);
            let state = fold(&[
                run_started(&["a", "b"]),
                E::LapStarted { lap: 0 },
                contribution("a", 0, 0, "propose"),
                contribution("b", 0, 1, "propose"),
            ]);
            let actions = dec.decide(&state);

            assert_eq!(actions.len(), 1, "got {actions:?}");
            assert_eq!(invoked_agent(&actions), Some("a"));
        }

        // (d) every agent has voted → `OutcomeResolved` + `Complete`.
        #[test]
        fn all_voted_resolves_and_completes() {
            let config = RingConfig {
                max_laps: 1,
                ..RingConfig::default()
            };
            let dec = test_decider(&["a", "b"], config, 5, BTreeMap::new(), None, None);
            let state = fold(&[
                run_started(&["a", "b"]),
                E::LapStarted { lap: 0 },
                contribution("a", 0, 0, "propose"),
                contribution("b", 0, 1, "propose"),
                vote("a", "Use Rust", 0.9),
                vote("b", "Use Rust", 0.8),
            ]);
            let actions = dec.decide(&state);

            assert_eq!(actions.len(), 2, "got {actions:?}");
            assert!(
                matches!(&actions[0], Action::Emit(E::OutcomeResolved { outcome }) if outcome == "Use Rust"),
                "expected Emit(OutcomeResolved{{outcome: \"Use Rust\"}}), got {:?}",
                actions[0]
            );
            assert!(
                matches!(&actions[1], Action::Complete { content } if content == "Use Rust"),
                "expected Complete{{content: \"Use Rust\"}}, got {:?}",
                actions[1]
            );
        }

        // (e) the decider's own `max_laps` cap (independent of
        // `config.max_laps`) is reached while still circulating, without
        // having converged → `Warned{max_laps}` + `Complete` with a partial
        // digest.
        #[test]
        fn own_max_laps_cap_warns_and_completes() {
            // `config.max_laps` is generous (3, the default) so `ring_phase`
            // would happily advance to lap 1 — but the decider's own
            // `max_laps` field is set to 1, independently capping
            // circulation at lap 0.
            let dec = test_decider(
                &["a", "b"],
                RingConfig::default(),
                1,
                BTreeMap::new(),
                None,
                None,
            );
            let state = fold(&[
                run_started(&["a", "b"]),
                E::LapStarted { lap: 0 },
                contribution("a", 0, 0, "propose"),
                contribution("b", 0, 1, "propose"),
            ]);
            let actions = dec.decide(&state);

            assert_eq!(actions.len(), 2, "got {actions:?}");
            assert!(
                matches!(&actions[0], Action::Emit(E::Warned { code }) if code == "max_laps"),
                "expected Warned{{code: \"max_laps\"}}, got {:?}",
                actions[0]
            );
            assert!(
                matches!(&actions[1], Action::Complete { content } if content.contains('a') && content.contains('b')),
                "expected Complete with a partial digest, got {:?}",
                actions[1]
            );
        }

        // Token budget guard fires ahead of any phase-specific logic, even
        // mid-circulation.
        #[test]
        fn token_budget_exhausted_warns_and_completes() {
            let dec = test_decider(
                &["a", "b"],
                RingConfig::default(),
                5,
                BTreeMap::new(),
                Some(10),
                None,
            );
            let state = fold(&[
                run_started(&["a", "b"]),
                E::LapStarted { lap: 0 },
                E::ContributionAdded {
                    agent: "a".into(),
                    lap: 0,
                    position: 0,
                    action: "propose".into(),
                    content: "c".into(),
                    tokens_in: 6,
                    tokens_out: 6,
                    cost: 0.0,
                },
            ]);
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

    // ── RingEffectRunner (Task 8) ─────────────────────────────────────
    //
    // Named `effect_runner` so `cargo test es::ring::tests::effect_runner`
    // targets this module — mirrors the naming convention used by
    // `es::hierarchical::tests::effect_runner`/
    // `es::blackboard::tests::effect_runner`.
    mod effect_runner {
        use super::*;
        use crate::core::agent::AgentMetadata;
        use crate::providers::traits::{CompletionResponse, ProviderMetadata, TokenStream};
        use std::path::PathBuf;
        use std::sync::Mutex;

        /// Minimal `Agent` for effect-runner tests: a concrete (non
        /// `latest:auto`) model by default.
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
                system_prompt: format!("You are {name}."),
                instructions: None,
                output_format: None,
                pipeline: None,
                context: None,
            }
        }

        /// Records every `CompletionRequest` it receives and always returns
        /// a fixed `response` with `tokens_in: 5, tokens_out: 7, cost: 0.03`
        /// — mirrors `CapturingProvider` in `es::hierarchical`/
        /// `es::blackboard`'s own effect-runner tests, so assertions can
        /// check both what was sent and what came back.
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
                    tokens_in: 5,
                    tokens_out: 7,
                    cost: 0.03,
                })
            }
            async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
                anyhow::bail!("streaming not exercised by RingEffectRunner tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "capturing".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        /// Always fails — used to exercise the graceful-degradation
        /// (Pass/abstention) branches.
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
                anyhow::bail!("streaming not exercised by RingEffectRunner tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "failing".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        fn runner(
            agent_names: &[&str],
            config: RingConfig,
            provider: Arc<dyn Provider>,
        ) -> RingEffectRunner {
            let mut agents = BTreeMap::new();
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            for name in agent_names {
                agents.insert((*name).to_string(), test_agent(name, "concrete-model"));
                providers.insert((*name).to_string(), provider.clone());
            }
            RingEffectRunner::new(agents, providers, config, BTreeMap::new())
        }

        // (a) Circulation phase: `ACTION: PROPOSE\nCONTENT: x` → `ContributionAdded`
        // with the expected agent/lap/position/action/content/tokens.
        #[tokio::test]
        async fn circulate_propose_returns_contribution_added() {
            let capturing = Arc::new(CapturingProvider::new("ACTION: PROPOSE\nCONTENT: x"));
            let runner = runner(
                &["a", "b"],
                RingConfig::default(),
                capturing.clone() as Arc<dyn Provider>,
            );
            let state = fold(&[run_started(&["a", "b"]), E::LapStarted { lap: 0 }]);

            let event = runner.run_invoke("a", "task", &state).await.unwrap();

            match event {
                E::ContributionAdded {
                    agent,
                    lap,
                    position,
                    action,
                    content,
                    tokens_in,
                    tokens_out,
                    cost,
                } => {
                    assert_eq!(agent, "a");
                    assert_eq!(lap, 0);
                    assert_eq!(position, 0);
                    assert_eq!(action, "propose");
                    assert_eq!(content, "x");
                    assert_eq!(tokens_in, 5);
                    assert_eq!(tokens_out, 7);
                    assert!((cost - 0.03).abs() < 1e-9);
                }
                other => panic!("expected ContributionAdded, got {other:?}"),
            }

            let sent = capturing.requests();
            assert_eq!(sent.len(), 1);
            assert!(sent[0].messages[0].content.contains("Lap: 0"));
        }

        // (b) `ACTION: PASS` must yield `action == "pass"` EXACTLY — the
        // literal string `ring_phase`'s early-convergence check compares
        // against (see `action_string`'s doc comment / the Task 6 review
        // contract in the brief).
        #[tokio::test]
        async fn circulate_pass_action_is_exact_lowercase_string() {
            let capturing = Arc::new(CapturingProvider::new(
                "ACTION: PASS\nCONTENT: nothing to add",
            ));
            let runner = runner(
                &["a", "b"],
                RingConfig::default(),
                capturing as Arc<dyn Provider>,
            );
            let state = fold(&[run_started(&["a", "b"]), E::LapStarted { lap: 0 }]);

            let event = runner.run_invoke("a", "task", &state).await.unwrap();

            match event {
                E::ContributionAdded { action, .. } => assert_eq!(action, "pass"),
                other => panic!("expected ContributionAdded, got {other:?}"),
            }
        }

        // (c) Circulation finished (both agents contributed the only lap,
        // max_laps == 1) → phase Vote → `CONFIDENCE: 0.7` → `VoteCast` with
        // the expected confidence and full-range `supports`.
        #[tokio::test]
        async fn vote_phase_returns_vote_cast_with_confidence_and_supports() {
            let config = RingConfig {
                max_laps: 1,
                ..RingConfig::default()
            };
            let capturing = Arc::new(CapturingProvider::new("CONFIDENCE: 0.7\nUse Rust"));
            let runner = runner(&["a", "b"], config, capturing as Arc<dyn Provider>);
            let state = fold(&[
                run_started(&["a", "b"]),
                E::LapStarted { lap: 0 },
                E::ContributionAdded {
                    agent: "a".into(),
                    lap: 0,
                    position: 0,
                    action: "propose".into(),
                    content: "c1".into(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                },
                E::ContributionAdded {
                    agent: "b".into(),
                    lap: 0,
                    position: 1,
                    action: "propose".into(),
                    content: "c2".into(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                },
            ]);
            let event = runner.run_invoke("a", "task", &state).await.unwrap();

            match event {
                E::VoteCast {
                    agent,
                    confidence,
                    supports,
                    concerns,
                    ..
                } => {
                    assert_eq!(agent, "a");
                    assert!((confidence - 0.7).abs() < 1e-6);
                    assert_eq!(supports, vec![0, 1]);
                    assert!(concerns.is_empty());
                }
                other => panic!("expected VoteCast, got {other:?}"),
            }
        }

        // (d) Provider error during circulation → degraded Pass, NOT an
        // `Err` — the run must not abort.
        #[tokio::test]
        async fn circulate_provider_error_degrades_to_pass_not_err() {
            let runner = runner(
                &["a", "b"],
                RingConfig::default(),
                Arc::new(FailingProvider) as Arc<dyn Provider>,
            );
            let state = fold(&[run_started(&["a", "b"]), E::LapStarted { lap: 0 }]);

            let event = runner.run_invoke("a", "task", &state).await.unwrap();

            match event {
                E::ContributionAdded {
                    action,
                    content,
                    tokens_in,
                    tokens_out,
                    cost,
                    ..
                } => {
                    assert_eq!(action, "pass");
                    assert_eq!(content, "[agent failed]");
                    assert_eq!(tokens_in, 0);
                    assert_eq!(tokens_out, 0);
                    assert!((cost - 0.0).abs() < 1e-9);
                }
                other => panic!("expected ContributionAdded (degraded), got {other:?}"),
            }
        }

        // Provider error during voting → neutral abstention, NOT an `Err`.
        #[tokio::test]
        async fn vote_provider_error_degrades_to_neutral_abstention_not_err() {
            let config = RingConfig {
                max_laps: 1,
                ..RingConfig::default()
            };
            let runner = runner(
                &["a", "b"],
                config,
                Arc::new(FailingProvider) as Arc<dyn Provider>,
            );
            let state = fold(&[
                run_started(&["a", "b"]),
                E::LapStarted { lap: 0 },
                E::ContributionAdded {
                    agent: "a".into(),
                    lap: 0,
                    position: 0,
                    action: "propose".into(),
                    content: "c1".into(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                },
                E::ContributionAdded {
                    agent: "b".into(),
                    lap: 0,
                    position: 1,
                    action: "propose".into(),
                    content: "c2".into(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                },
            ]);

            let event = runner.run_invoke("a", "task", &state).await.unwrap();

            match event {
                E::VoteCast {
                    confidence,
                    concerns,
                    ..
                } => {
                    assert!((confidence - 0.0).abs() < 1e-9);
                    assert!(!concerns.is_empty());
                }
                other => panic!("expected VoteCast (degraded), got {other:?}"),
            }
        }

        #[tokio::test]
        async fn run_invoke_errors_for_unknown_agent() {
            let runner = RingEffectRunner::new(
                BTreeMap::new(),
                BTreeMap::new(),
                RingConfig::default(),
                BTreeMap::new(),
            );
            let state = ExecutionState::default();
            let err = runner
                .run_invoke("missing", "task", &state)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("missing"));
        }

        #[tokio::test]
        async fn run_invoke_errors_when_provider_missing_for_known_agent() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            let runner = RingEffectRunner::new(
                agents,
                BTreeMap::new(),
                RingConfig::default(),
                BTreeMap::new(),
            );
            let state = ExecutionState::default();
            let err = runner.run_invoke("a", "task", &state).await.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("provider") && msg.contains("'a'"),
                "expected a distinctive missing-provider message, got: {msg}"
            );
        }

        // `"latest:auto"` resolves to a concrete model via `state.routed_tiers`,
        // the same as `HierarchicalEffectRunner`/`BlackboardEffectRunner`.
        // Uses a deliberately uncached provider name so `resolve_model_for_tier`
        // is forced onto its hermetic, pure `fallback_model_for_tier` path.
        #[tokio::test]
        async fn run_invoke_resolves_latest_auto_to_concrete_model() {
            let mut agent = test_agent("a", "latest:auto");
            agent.metadata.provider = "test-only-uncached-provider".to_string();
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), agent);
            let capturing = Arc::new(CapturingProvider::new("ACTION: PROPOSE\nCONTENT: x"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                RingEffectRunner::new(agents, providers, RingConfig::default(), BTreeMap::new());

            let state = fold(&[
                run_started(&["a"]),
                E::LapStarted { lap: 0 },
                E::ModelRouted {
                    agent: "a".into(),
                    tier: "Fast".into(),
                    reason: "Length".into(),
                },
            ]);
            runner.run_invoke("a", "task", &state).await.unwrap();

            let expected =
                fallback_model_for_tier("test-only-uncached-provider", ModelTier::Fast).to_string();
            let sent = capturing.requests();
            assert_eq!(sent[0].model, expected);
            assert_ne!(sent[0].model, "latest:auto");
        }
    }
}
