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
use std::sync::Arc;

use async_trait::async_trait;

use super::engine::{Action, Decider, EffectRunner, run_event_sourced};
use super::event::ExecutionEvent;
use super::log::EventLog;
use super::state::{BoardEntryRec, ExecutionState};
use crate::core::agent::Agent;
#[cfg(test)]
use crate::core::model_resolution::fallback_model_for_tier;
use crate::core::model_resolution::{ModelTier, resolve_model_for_tier};
use crate::core::orchestration::blackboard::{BlackboardConfig, EntryKind, entry_kind_name};
use crate::core::orchestration::llm_agents::{BOARD_ACTION_INSTRUCTIONS, parse_board_action};
use crate::core::provider::{ChatMessage, CompletionRequest, Provider};
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

// ── BlackboardEffectRunner (Task 4): the sole async/impure effect ────

/// Parse a tier string as stored in `ExecutionState::routed_tiers` back into
/// a `ModelTier`.
///
/// Identical in spirit to `es::hierarchical::parse_routed_tier` — duplicated
/// rather than shared (same rationale as `BlackboardDecider::model_routed_event`
/// above: the two effect runners' `agents`/model-resolution concerns live on
/// unrelated structs with no common trait today). Unrecognized strings fall
/// back to `Pro`, matching the hierarchical counterpart.
fn parse_routed_tier(tier: &str) -> ModelTier {
    match tier.to_lowercase().as_str() {
        "fast" => ModelTier::Fast,
        "max" => ModelTier::Max,
        _ => ModelTier::Pro,
    }
}

/// Executes the actual LLM call behind `Action::Invoke` for the blackboard
/// pattern and turns the raw provider response into the `BoardEntryAdded`
/// event the pure loop/`BlackboardDecider` expect.
///
/// This is the *only* impure/async piece of the event-sourced blackboard
/// engine — every other function in this module is a pure, synchronous
/// helper over `ExecutionState`. Coexists with the legacy
/// `core::orchestration::llm_agents::LlmBoardAgent`/`blackboard::run_blackboard`
/// (this struct is not wired into it, and does not import from it beyond the
/// plain, side-effect-free `parse_board_action` parser and
/// `BOARD_ACTION_INSTRUCTIONS` constant it explicitly reuses — strict
/// coexistence, mirroring `HierarchicalEffectRunner`).
pub struct BlackboardEffectRunner {
    /// All known agents by name (system prompt, model, temperature, …).
    pub agents: BTreeMap<String, Agent>,
    /// Provider instance per agent name.
    pub providers: BTreeMap<String, Arc<dyn Provider>>,
    /// Blackboard configuration — currently read only for `token_budget`,
    /// surfaced in the "Budget remaining" line of the assembled prompt (the
    /// same line `LlmBoardAgent::contribute` sends), for prompt fidelity with
    /// the legacy engine.
    pub config: BlackboardConfig,
}

impl BlackboardEffectRunner {
    /// Construct a new `BlackboardEffectRunner` from its immutable inputs.
    pub fn new(
        agents: BTreeMap<String, Agent>,
        providers: BTreeMap<String, Arc<dyn Provider>>,
        config: BlackboardConfig,
    ) -> Self {
        Self {
            agents,
            providers,
            config,
        }
    }

    /// Assemble the user-turn prompt sent to `agent_name` for the current
    /// round, reproducing the shape of `LlmBoardAgent::contribute`'s
    /// `user_msg` (`core::orchestration::llm_agents`) over the event-sourced
    /// `state.board` projection instead of a live `BoardSnapshot`:
    /// `"Task: …\nRound: …\nBudget remaining: … tokens\n"`, then (only if any
    /// qualify) a `"Recent board entries:\n"` section listing, in
    /// chronological order, the **10 most recent** entries **whose `round`
    /// is strictly less than `state.board.round`** as `"- [{agent}#{index}
    /// {kind}] {content}\n"` — `{index}` being the entry's position in
    /// `state.board.entries` (the same numbering `entry_kind_to_rec`'s
    /// `refs`/the legacy `BoardEntry::index` target — so a `TARGET: <index>`
    /// an LLM emits in its structured response points at the same entries
    /// this snapshot exposed to it) — then `BOARD_ACTION_INSTRUCTIONS`
    /// verbatim.
    ///
    /// Two legacy behaviours are reproduced here, both over the event-sourced
    /// `state.board` projection rather than a live `BoardSnapshot`:
    /// - the round filter (`entry.round < state.board.round`) mirrors
    ///   `Board::snapshot()` being taken once at the *start* of each round
    ///   (before that round's agents post anything), so contributors within
    ///   the same round never see each other's in-flight entries;
    /// - the 10-entry cap (OH1 Lot 4 Task 3, reconciliation B) mirrors
    ///   `LlmBoardAgent::contribute`'s own `board.entries.iter().rev().take(10)`
    ///   (`core::orchestration::llm_agents:377`) — this used to be an
    ///   undocumented-cap gap in the ES path (every qualifying entry was
    ///   included, unbounded), which this reconciliation closes. Unlike
    ///   legacy's raw `rev().take(10)` (which prints the window
    ///   newest-first), the 10 kept here are restored to their original
    ///   chronological (oldest-first) order before formatting — the task's
    ///   explicit intent for this reconciliation — since nothing else about
    ///   this prompt's entry ordering is reversed.
    fn build_prompt(&self, agent_name: &str, input: &str, state: &ExecutionState) -> String {
        let budget_remaining = self
            .config
            .token_budget
            .saturating_sub(state.budget_tokens_in + state.budget_tokens_out);
        let mut user_msg = format!(
            "Task: {input}\nRound: {}\nBudget remaining: {budget_remaining} tokens\n",
            state.board.round
        );

        let snapshot: Vec<(usize, &BoardEntryRec)> = {
            let mut recent: Vec<(usize, &BoardEntryRec)> = state
                .board
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.round < state.board.round)
                .rev()
                .take(10)
                .collect();
            recent.reverse();
            recent
        };
        if !snapshot.is_empty() {
            user_msg.push_str("\nRecent board entries:\n");
            for (index, entry) in snapshot {
                user_msg.push_str(&format!(
                    "- [{}#{index} {}] {}\n",
                    entry.agent, entry.kind, entry.content
                ));
            }
        }

        // Silence the unused `agent_name` parameter warning below: kept for
        // signature symmetry with `run_invoke`/future per-agent prompt
        // customization, even though the current prompt shape doesn't read
        // it directly (the legacy `contribute` prompt is agent-agnostic
        // too — the agent's own perspective comes entirely from its system
        // prompt, sent separately).
        let _ = agent_name;

        user_msg.push_str(BOARD_ACTION_INSTRUCTIONS);
        user_msg
    }
}

#[async_trait]
impl EffectRunner for BlackboardEffectRunner {
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

        let prompt = self.build_prompt(agent, input, state);

        // Same `"latest:auto"` resolution as `HierarchicalEffectRunner`: see
        // its doc comment for the full rationale. `BlackboardDecider`
        // (Task 3) emits `ModelRouted` ahead of every `latest:auto` agent's
        // `Invoke`, so `state.routed_tiers` should already carry its tier by
        // the time this runs; the `None` branch is a defensive fallback for
        // a hand-built state (tests) or a future decider regression.
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

        let round = state.board.round;

        // Graceful degradation (Task 4 brief, point 5): a provider error
        // must NOT abort the run — legacy fidelity requires the blackboard
        // to keep going even when one agent's LLM call fails outright (a
        // timeout, a 5xx, an auth error, …). Instead of propagating the
        // error through `?` (which `es::engine::run_event_sourced` would
        // otherwise let bubble out of the whole run via its own `?` on
        // `run_invoke`), we swallow it here and manufacture a degraded
        // `BoardEntryAdded`: `kind = "finding"` (the neutral default,
        // matching `parse_board_action`'s own fallback for an unparseable
        // response), a `"[agent failed]"` marker prefix so the failure is
        // visible in the board transcript, `confidence: 0.0` (the LLM never
        // actually vouched for anything), and `tokens_in/out = 0, cost =
        // 0.0` (no tokens were actually consumed/billed — the call never
        // returned a response to meter). This keeps `BlackboardDecider`'s
        // round-completion bookkeeping correct (`round_complete` only checks
        // that every eligible agent posted *an* entry for the round, not
        // that it succeeded) without corrupting the budget totals with a
        // cost that was never incurred.
        match provider.complete(request).await {
            Ok(response) => {
                let (kind, confidence, content) = parse_board_action(&response.content);
                let (kind, refs) = entry_kind_to_rec(&kind);
                Ok(ExecutionEvent::BoardEntryAdded {
                    agent: agent.to_string(),
                    round,
                    kind,
                    content,
                    refs,
                    confidence,
                    tokens_in: response.tokens_in,
                    tokens_out: response.tokens_out,
                    cost: response.cost,
                })
            }
            Err(err) => {
                tracing::warn!(
                    agent,
                    error = %err,
                    "blackboard agent provider call failed; recording a degraded entry instead of aborting the run"
                );
                Ok(ExecutionEvent::BoardEntryAdded {
                    agent: agent.to_string(),
                    round,
                    kind: "finding".to_string(),
                    content: format!("[agent failed] {err}"),
                    refs: vec![],
                    confidence: 0.0,
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                })
            }
        }
    }
}

// ── run_blackboard_es (Task 5): end-to-end assembly ──────────────────

/// Run a complete blackboard orchestration end-to-end through the
/// event-sourced engine: builds the initial `RunStarted` event, constructs a
/// [`BlackboardDecider`] + [`BlackboardEffectRunner`] from `config` /
/// `agents` / `providers` / `routing_rules`, and drives them through
/// [`run_event_sourced`], returning the final [`ExecutionState`].
///
/// `run_id` is accepted explicitly rather than generated internally, so
/// callers — notably tests proving replay determinism — can pass a fixed id
/// and later reconstruct the same state purely from the log via
/// [`super::engine::replay`].
///
/// The run's roster (`RunStarted { agents, .. }`, and `BlackboardDecider`'s
/// `agent_order`) is `agents.keys()`, i.e. name-sorted (`BTreeMap`
/// iteration order) — the same convention `run_hierarchical_es` uses for its
/// own `agent_names`.
///
/// `max_rounds` and `token_budget` are derived from `config`
/// (`BlackboardConfig::max_rounds`/`token_budget`, both concrete `u32`/`u64`
/// values with documented defaults — unlike `OrchestrationConfig`'s
/// `Option` fields, `BlackboardConfig` has no "unset" state for either, so
/// `token_budget` narrows to `Option<u32>` as `Some(..)` unconditionally,
/// saturating at `u32::MAX`). `BlackboardConfig` itself carries no
/// cost-budget field (unlike `OrchestrationConfig::cost_limit`) — legacy's
/// standalone `run_blackboard` gets its cost cap from the `Board` its caller
/// (`run.rs`) constructs via `Board::with_cost_limit(.., cost_limit)`,
/// seeded from `OrchestrationConfig::cost_limit` (see
/// `core::orchestration::blackboard::Board::with_cost_limit` /
/// `TokenBudget::cost_limit` and its `CostLimitExceeded` halt). This
/// function accepts the same `cost_limit: Option<f64>` explicitly (OH1 Lot 4
/// Task 3, reconciliation C) and threads it straight to
/// [`BlackboardDecider`]'s own `cost_limit` field, whose `breached_budget`
/// guard already checks it — `run_blackboard_es`/`decide` previously hard-
/// coded `None` here, silently dropping the legacy cost guard on the ES path.
///
/// Coexists with the legacy `core::orchestration::blackboard::run_blackboard`
/// — this function is not called from `run.rs`; wiring it in as the active
/// engine is a later lot (the bascule).
#[allow(clippy::too_many_arguments)]
pub async fn run_blackboard_es(
    run_id: &str,
    input: &str,
    agents: BTreeMap<String, Agent>,
    providers: BTreeMap<String, Arc<dyn Provider>>,
    config: BlackboardConfig,
    routing_rules: RoutingRules,
    cost_limit: Option<f64>,
    log: &mut impl EventLog,
) -> anyhow::Result<ExecutionState> {
    let agent_order: Vec<String> = agents.keys().cloned().collect();
    let initial = vec![
        ExecutionEvent::RunStarted {
            run_id: run_id.to_string(),
            pattern: "blackboard".to_string(),
            agents: agent_order.clone(),
            input: input.to_string(),
            project: None,
        },
        ExecutionEvent::ConfigSnapshot {
            config_json: serde_json::to_string(&config).unwrap_or_default(),
        },
    ];

    let max_rounds = config.max_rounds;
    let token_budget = Some(u32::try_from(config.token_budget).unwrap_or(u32::MAX));

    let decider = BlackboardDecider::new(
        agents.clone(),
        agent_order,
        input.to_string(),
        config.clone(),
        routing_rules,
        max_rounds,
        token_budget,
        cost_limit,
    );
    let effects = BlackboardEffectRunner::new(agents, providers, config);

    run_event_sourced(run_id, initial, &decider, &effects, log).await
}

/// Resume a previously interrupted `blackboard` run (OH1 Lot 6, Task 3):
/// recovers the run's original `input` and the pattern's `BlackboardConfig`
/// from the log (`RunStarted`/`ConfigSnapshot` — see
/// [`super::engine::run_started_roster_and_input`]/[`super::engine::config_snapshot`]),
/// rebuilds the SAME [`BlackboardDecider`]/[`BlackboardEffectRunner`] pair
/// [`run_blackboard_es`] would (including `agent_order` derived from
/// `agents.keys()`, exactly like the live path — deterministic given the
/// same roster, so it needs no log-recovered ordering), and drives
/// [`super::engine::resume_event_sourced`] instead of appending a fresh
/// `RunStarted`/`ConfigSnapshot`.
///
/// `agents`/`providers` must be the roster reloaded from the project on disk
/// (keyed by the same roster keys the original run used — see
/// `ExecutionState::agents`, folded from `RunStarted`); `cost_limit` is
/// re-derived from the project's top-level `orchestration.cost_limit`
/// exactly as [`run_blackboard_es`]'s caller does, since it lives outside
/// `BlackboardConfig` and is never captured by `ConfigSnapshot`.
///
/// Bails if `run_id` has no recorded `RunStarted` (unknown run) or isn't
/// currently `Running` (see `resume_event_sourced`).
pub async fn resume_blackboard_es(
    run_id: &str,
    agents: BTreeMap<String, Agent>,
    providers: BTreeMap<String, Arc<dyn Provider>>,
    routing_rules: RoutingRules,
    cost_limit: Option<f64>,
    log: &mut impl EventLog,
) -> anyhow::Result<ExecutionState> {
    use super::engine::{config_snapshot, resume_event_sourced, run_started_roster_and_input};

    let events = log.events(run_id)?;
    let (_roster, input) = run_started_roster_and_input(&events)
        .ok_or_else(|| anyhow::anyhow!("no run found for id {run_id}"))?;
    let config: BlackboardConfig = config_snapshot(&events);

    let agent_order: Vec<String> = agents.keys().cloned().collect();
    let max_rounds = config.max_rounds;
    let token_budget = Some(u32::try_from(config.token_budget).unwrap_or(u32::MAX));

    let decider = BlackboardDecider::new(
        agents.clone(),
        agent_order,
        input,
        config.clone(),
        routing_rules,
        max_rounds,
        token_budget,
        cost_limit,
    );
    let effects = BlackboardEffectRunner::new(agents, providers, config);

    resume_event_sourced(run_id, &decider, &effects, log).await
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

    // ── BlackboardEffectRunner (Task 4) ──────────────────────────────
    //
    // Named `effect_runner` so `cargo test es::blackboard::tests::effect_runner`
    // targets this module — mirrors the naming convention used by
    // `es::hierarchical::tests::effect_runner`.
    mod effect_runner {
        use super::*;
        use crate::core::agent::AgentMetadata;
        use crate::core::provider::{CompletionResponse, ProviderMetadata, TokenStream};
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

        /// Returns a fixed response with fixed token/cost/model metrics,
        /// regardless of the request.
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
                anyhow::bail!("streaming not exercised by BlackboardEffectRunner tests")
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
        /// (namely, the assembled prompt) — mirrors `CapturingProvider` in
        /// `es::hierarchical`'s own effect-runner tests.
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
                anyhow::bail!("streaming not exercised by BlackboardEffectRunner tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "capturing".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        /// Always fails — proves `run_invoke` degrades gracefully (a
        /// `BoardEntryAdded` with a marker content) instead of propagating
        /// the provider's error through the run.
        struct FailingProvider;

        #[async_trait]
        impl Provider for FailingProvider {
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> anyhow::Result<CompletionResponse> {
                anyhow::bail!("simulated provider outage")
            }
            async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
                anyhow::bail!("streaming not exercised by BlackboardEffectRunner tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "failing".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        fn board_run_started(agents: &[&str]) -> ExecutionEvent {
            ExecutionEvent::RunStarted {
                run_id: "r".into(),
                pattern: "blackboard".into(),
                agents: agents.iter().map(|a| a.to_string()).collect(),
                input: "task".into(),
                project: None,
            }
        }

        // (a) Step 1 (brief): a fixed structured response
        // (`ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:ok`) →
        // `BoardEntryAdded` with kind="confirmation", refs=[0], confidence
        // 0.9, the state's current round, and the *real* token/cost figures
        // from the `CompletionResponse` (not the entry-level defaults
        // `parse_board_action` would produce on a fallback).
        #[tokio::test]
        async fn run_invoke_parses_structured_action_into_board_entry_added() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert(
                "a".to_string(),
                Arc::new(FixedProvider {
                    content: "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:ok"
                        .to_string(),
                    tokens_in: 7,
                    tokens_out: 5,
                    cost: 0.03,
                    model: "concrete-model".to_string(),
                }),
            );
            let runner =
                BlackboardEffectRunner::new(agents, providers, BlackboardConfig::default());

            let state = fold(&[
                board_run_started(&["a"]),
                ExecutionEvent::RoundStarted { round: 0 },
            ]);
            let ev = runner.run_invoke("a", "task", &state).await.unwrap();

            match ev {
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
                    assert_eq!(agent, "a");
                    assert_eq!(round, 0);
                    assert_eq!(kind, "confirmation");
                    assert_eq!(refs, vec![0]);
                    assert!((confidence - 0.9).abs() < 1e-6);
                    assert_eq!(content, "ok");
                    assert_eq!(tokens_in, 7);
                    assert_eq!(tokens_out, 5);
                    assert!((cost - 0.03).abs() < 1e-9);
                }
                other => panic!("expected BoardEntryAdded, got {other:?}"),
            }
        }

        // (b) Step 1 (brief): the captured prompt must NOT contain the
        // current round's entries (its peers' in-flight contributions) but
        // MUST contain entries from earlier, already-completed rounds — the
        // snapshot-filter requirement documented on `build_prompt`.
        #[tokio::test]
        async fn run_invoke_prompt_snapshot_excludes_current_round_entries() {
            let mut agents = BTreeMap::new();
            agents.insert("b".to_string(), test_agent("b", "concrete-model"));
            let capturing = Arc::new(CapturingProvider::new(
                "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:noted",
            ));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("b".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                BlackboardEffectRunner::new(agents, providers, BlackboardConfig::default());

            let events = vec![
                board_run_started(&["a", "b"]),
                ExecutionEvent::RoundStarted { round: 0 },
                ExecutionEvent::BoardEntryAdded {
                    agent: "a".into(),
                    round: 0,
                    kind: "finding".into(),
                    content: "prev-round-content".into(),
                    refs: vec![],
                    confidence: 0.7,
                    tokens_in: 1,
                    tokens_out: 1,
                    cost: 0.0,
                },
                ExecutionEvent::RoundStarted { round: 1 },
                ExecutionEvent::BoardEntryAdded {
                    agent: "a".into(),
                    round: 1,
                    kind: "finding".into(),
                    content: "current-round-content".into(),
                    refs: vec![],
                    confidence: 0.7,
                    tokens_in: 1,
                    tokens_out: 1,
                    cost: 0.0,
                },
            ];
            let state = fold(&events);
            runner.run_invoke("b", "task", &state).await.unwrap();

            let sent = capturing.requests();
            assert_eq!(sent.len(), 1);
            let prompt = &sent[0].messages[0].content;
            assert!(
                prompt.contains("prev-round-content"),
                "expected the previous round's entry in the prompt, got: {prompt}"
            );
            assert!(
                !prompt.contains("current-round-content"),
                "must not leak the current round's peer entries into the prompt, got: {prompt}"
            );
        }

        // (b-bis) Step 1 (brief), OH1 Lot 4 Task 3 reconciliation B: with
        // more than 10 already-completed-round entries on the board, the
        // prompt must contain only the 10 most recent — mirroring legacy's
        // `LlmBoardAgent::contribute` (`board.entries.iter().rev().take(10)`,
        // `llm_agents.rs:377`), which this reconciliation reintroduces on the
        // ES path (previously unbounded). 12 round-0 entries are posted
        // (`entry-0` .. `entry-11`, in that order); only `entry-2` ..
        // `entry-11` (the 10 most recent) may appear, and they must appear in
        // chronological order (oldest of the kept ten first).
        #[tokio::test]
        async fn run_invoke_prompt_snapshot_caps_at_10_most_recent_entries() {
            let mut agents = BTreeMap::new();
            agents.insert("b".to_string(), test_agent("b", "concrete-model"));
            let capturing = Arc::new(CapturingProvider::new(
                "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:noted",
            ));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("b".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                BlackboardEffectRunner::new(agents, providers, BlackboardConfig::default());

            let mut events = vec![
                board_run_started(&["a", "b"]),
                ExecutionEvent::RoundStarted { round: 0 },
            ];
            for i in 0..12 {
                events.push(ExecutionEvent::BoardEntryAdded {
                    agent: "a".into(),
                    round: 0,
                    kind: "finding".into(),
                    content: format!("entry-{i}"),
                    refs: vec![],
                    confidence: 0.7,
                    tokens_in: 1,
                    tokens_out: 1,
                    cost: 0.0,
                });
            }
            events.push(ExecutionEvent::RoundStarted { round: 1 });
            let state = fold(&events);
            runner.run_invoke("b", "task", &state).await.unwrap();

            let sent = capturing.requests();
            assert_eq!(sent.len(), 1);
            let prompt = &sent[0].messages[0].content;

            for i in 0..2 {
                assert!(
                    !prompt.contains(&format!("entry-{i}\n")),
                    "entry-{i} is older than the 10 most recent and must be truncated, got: {prompt}"
                );
            }
            let mut last_pos = 0;
            for i in 2..12 {
                let marker = format!("entry-{i}");
                let pos = prompt
                    .find(&marker)
                    .unwrap_or_else(|| panic!("expected {marker} in the prompt, got: {prompt}"));
                assert!(
                    pos > last_pos || i == 2,
                    "expected chronological order, {marker} at {pos} came before an earlier entry (last_pos={last_pos})"
                );
                last_pos = pos;
            }
        }

        // (c) Step 1 (brief): a provider error must NOT propagate as an
        // `Err` — `run_invoke` degrades gracefully into a `BoardEntryAdded`
        // with kind="finding", a "[agent failed]" content marker, confidence
        // 0.0, and zeroed tokens/cost.
        #[tokio::test]
        async fn run_invoke_degrades_gracefully_on_provider_error() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), test_agent("a", "concrete-model"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), Arc::new(FailingProvider));
            let runner =
                BlackboardEffectRunner::new(agents, providers, BlackboardConfig::default());

            let state = fold(&[
                board_run_started(&["a"]),
                ExecutionEvent::RoundStarted { round: 2 },
            ]);
            let ev = runner
                .run_invoke("a", "task", &state)
                .await
                .expect("a provider error must not propagate as Err");

            match ev {
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
                    assert_eq!(agent, "a");
                    assert_eq!(round, 2);
                    assert_eq!(kind, "finding");
                    assert!(
                        content.contains("[agent failed]"),
                        "expected a degraded-entry marker, got: {content}"
                    );
                    assert!(refs.is_empty());
                    assert_eq!(confidence, 0.0);
                    assert_eq!(tokens_in, 0);
                    assert_eq!(tokens_out, 0);
                    assert_eq!(cost, 0.0);
                }
                other => panic!("expected a degraded BoardEntryAdded, got {other:?}"),
            }
        }

        // `"latest:auto"` is resolved the same way as
        // `HierarchicalEffectRunner`: `BlackboardDecider::model_routed_event`
        // emits `ModelRouted{agent, tier, ..}` ahead of the matching
        // `Invoke`, which `es::state::apply` projects into
        // `state.routed_tiers`; `run_invoke` reads that tier back and
        // resolves it via `resolve_model_for_tier` — never leaking the
        // literal `"latest:auto"` string to the provider. The agent's
        // `provider` is deliberately a name no on-disk `models.dev` cache
        // could contain, forcing the hermetic, pure `fallback_model_for_tier`
        // path (see the identical rationale on
        // `es::hierarchical::run_invoke_resolves_latest_auto_to_concrete_model`).
        #[tokio::test]
        async fn run_invoke_resolves_latest_auto_to_concrete_model() {
            let mut agent = test_agent("a", "latest:auto");
            agent.metadata.provider = "test-only-uncached-provider".to_string();
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), agent);
            let capturing = Arc::new(CapturingProvider::new(
                "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:noted",
            ));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), capturing.clone() as Arc<dyn Provider>);
            let runner =
                BlackboardEffectRunner::new(agents, providers, BlackboardConfig::default());

            let state = fold(&[
                board_run_started(&["a"]),
                ExecutionEvent::RoundStarted { round: 0 },
                ExecutionEvent::ModelRouted {
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

        // Config errors (unknown agent / no provider registered) are
        // distinct from a provider *call* failure: these still propagate as
        // `Err`, mirroring `HierarchicalEffectRunner`'s equivalent tests —
        // there is no "agent" to degrade gracefully on behalf of.
        #[tokio::test]
        async fn run_invoke_errors_for_unknown_agent() {
            let runner = BlackboardEffectRunner::new(
                BTreeMap::new(),
                BTreeMap::new(),
                BlackboardConfig::default(),
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
            let runner =
                BlackboardEffectRunner::new(agents, BTreeMap::new(), BlackboardConfig::default());
            let state = ExecutionState::default();
            let err = runner.run_invoke("a", "task", &state).await.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("provider") && msg.contains("'a'"),
                "expected a distinctive missing-provider message, got: {msg}"
            );
        }
    }

    // ── run_blackboard_es (Task 5): end-to-end + replay determinism ──
    //
    // Exercises `run_blackboard_es` as a whole (unlike `decide` and
    // `effect_runner` above, which drive `BlackboardDecider` /
    // `BlackboardEffectRunner` directly) — the same scenarios the legacy
    // `core::orchestration::blackboard::run_blackboard` engine covers, plus
    // a proof that `replay` reconstructs an identical `ExecutionState`
    // purely from the log, with no effect re-executed.
    //
    // IMPORTANT: every agent below uses a CONCRETE model string (never
    // `latest:auto`) — see the identical rationale on
    // `es::hierarchical::tests::run_hierarchical_es`'s own module doc:
    // resolving `latest:auto` reads an on-disk `models.dev` cache, which
    // would make these tests non-hermetic/flaky.
    mod run_blackboard_es_tests {
        use super::*;
        use crate::core::orchestration::es::engine::replay;
        use crate::core::orchestration::es::log::InMemoryLog;
        use crate::core::orchestration::es::state::RunStatus;
        use crate::core::provider::{CompletionResponse, ProviderMetadata, TokenStream};
        use std::collections::VecDeque;
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Minimal `Agent` for `run_blackboard_es` E2E tests: always a
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

        /// Provider scripted with a fixed sequence of responses, one per
        /// call (repeating the last one for any call beyond the scripted
        /// list, so a test can under-provision — e.g. a single response
        /// reused every round — without panicking). Also counts calls, so
        /// `es_blackboard_replay_reconstructs_state` can prove `replay`
        /// triggers none. Identical in spirit to
        /// `es::hierarchical::tests::ScriptedProvider` — duplicated rather
        /// than shared (no common test-helpers module exists for the ES
        /// engines today).
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
            async fn complete(
                &self,
                request: CompletionRequest,
            ) -> anyhow::Result<CompletionResponse> {
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
                anyhow::bail!("streaming not exercised by run_blackboard_es tests")
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "scripted".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
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
        /// `code` exactly. `apply` treats `Warned` as a no-op on
        /// `ExecutionState` (see `es::state::apply`), so — like
        /// `final_content` above — this can only be checked against the
        /// log, not the projected state.
        fn log_has_warned(log: &InMemoryLog, run_id: &str, code: &str) -> bool {
            log.events(run_id)
                .unwrap()
                .iter()
                .any(|e| matches!(e, ExecutionEvent::Warned { code: c } if c == code))
        }

        // Scenario 1: both agents post a `CONFIRMATION` targeting entry 0 in
        // round 0 — a 2/2 = 1.0 confirmation ratio, above the default
        // `consensus_threshold` (0.75), reaching consensus on the very
        // first round (default `convergence_rounds` is 1). The run must
        // `Complete`, and the final board digest must be non-empty.
        #[tokio::test]
        async fn es_blackboard_converges_and_completes() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), es_test_agent("a", "concrete-model"));
            agents.insert("b".to_string(), es_test_agent("b", "concrete-model"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert(
                "a".to_string(),
                Arc::new(ScriptedProvider::new(&[
                    "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:tout est cohérent",
                ])),
            );
            providers.insert(
                "b".to_string(),
                Arc::new(ScriptedProvider::new(&[
                    "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:confirmé",
                ])),
            );

            let mut log = InMemoryLog::default();
            let st = run_blackboard_es(
                "run-converge",
                "task",
                agents,
                providers,
                BlackboardConfig::default(),
                RoutingRules::default(),
                None,
                &mut log,
            )
            .await
            .unwrap();

            assert_eq!(st.status, RunStatus::Completed);
            assert!(
                st.board.entries.iter().all(|e| e.kind == "confirmation"),
                "expected only confirmation entries, got {:?}",
                st.board.entries
            );
            assert!(
                !final_content(&log, "run-converge").trim().is_empty(),
                "expected a non-empty final board digest"
            );
        }

        /// OH1 Lot 6, Task 3: `resume_blackboard_es` reconstructs the same
        /// `BlackboardDecider`/`BlackboardEffectRunner` a fresh
        /// `run_blackboard_es` would from a log that only has `RunStarted` +
        /// `ConfigSnapshot` recorded (simulating a crash immediately after
        /// config capture, before any round started) — proving
        /// `BlackboardConfig` round-trips through `ConfigSnapshot` correctly
        /// (the ConfigSnapshot-sufficiency finding this task investigates).
        /// Same scripted scenario as `es_blackboard_converges_and_completes`
        /// above, seeded by hand instead of via `run_blackboard_es`.
        #[tokio::test]
        async fn resume_blackboard_es_converges_from_a_config_snapshot_only_log() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), es_test_agent("a", "concrete-model"));
            agents.insert("b".to_string(), es_test_agent("b", "concrete-model"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert(
                "a".to_string(),
                Arc::new(ScriptedProvider::new(&[
                    "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:tout est cohérent",
                ])),
            );
            providers.insert(
                "b".to_string(),
                Arc::new(ScriptedProvider::new(&[
                    "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:confirmé",
                ])),
            );

            let config = BlackboardConfig::default();
            let agent_order: Vec<String> = agents.keys().cloned().collect();

            let mut log = InMemoryLog::default();
            log.append(
                "run-resume-bb",
                &ExecutionEvent::RunStarted {
                    run_id: "run-resume-bb".to_string(),
                    pattern: "blackboard".to_string(),
                    agents: agent_order,
                    input: "task".to_string(),
                    project: None,
                },
            )
            .unwrap();
            log.append(
                "run-resume-bb",
                &ExecutionEvent::ConfigSnapshot {
                    config_json: serde_json::to_string(&config).unwrap(),
                },
            )
            .unwrap();

            let st = resume_blackboard_es(
                "run-resume-bb",
                agents,
                providers,
                RoutingRules::default(),
                None,
                &mut log,
            )
            .await
            .unwrap();

            assert_eq!(st.status, RunStatus::Completed);
            assert!(
                st.board.entries.iter().all(|e| e.kind == "confirmation"),
                "expected only confirmation entries, got {:?}",
                st.board.entries
            );
            assert!(!final_content(&log, "run-resume-bb").trim().is_empty());
        }

        #[tokio::test]
        async fn resume_blackboard_es_bails_on_completed_run() {
            let mut log = InMemoryLog::default();
            log.append(
                "run-resume-bb",
                &ExecutionEvent::RunStarted {
                    run_id: "run-resume-bb".to_string(),
                    pattern: "blackboard".to_string(),
                    agents: vec!["a".to_string()],
                    input: "task".to_string(),
                    project: None,
                },
            )
            .unwrap();
            log.append(
                "run-resume-bb",
                &ExecutionEvent::Completed {
                    content: "done".to_string(),
                },
            )
            .unwrap();

            let err = resume_blackboard_es(
                "run-resume-bb",
                BTreeMap::new(),
                BTreeMap::new(),
                RoutingRules::default(),
                None,
                &mut log,
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("not resumable"));
        }

        // Scenario 2: both agents post plain `FINDING`s every round — never
        // a `CONFIRMATION` — so convergence never triggers. With
        // `max_rounds: 2`, the run must halt via `Warned{max_rounds}` (once
        // round 1 completes, since `round + 1 >= max_rounds` first holds at
        // round 1) rather than spin forever, and still end `Completed`
        // (the guard *completes* the run with the board digest, it does
        // not error).
        #[tokio::test]
        async fn es_blackboard_halts_at_max_rounds() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), es_test_agent("a", "concrete-model"));
            agents.insert("b".to_string(), es_test_agent("b", "concrete-model"));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert(
                "a".to_string(),
                Arc::new(ScriptedProvider::new(&[
                    "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:piste A",
                ])),
            );
            providers.insert(
                "b".to_string(),
                Arc::new(ScriptedProvider::new(&[
                    "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:piste B",
                ])),
            );

            let config = BlackboardConfig {
                max_rounds: 2,
                ..BlackboardConfig::default()
            };

            let mut log = InMemoryLog::default();
            let st = run_blackboard_es(
                "run-maxrounds",
                "task",
                agents,
                providers,
                config,
                RoutingRules::default(),
                None,
                &mut log,
            )
            .await
            .unwrap();

            assert_eq!(st.status, RunStatus::Completed);
            assert!(
                log_has_warned(&log, "run-maxrounds", "max_rounds"),
                "expected a max_rounds Warned event in the log"
            );
            assert!(
                !final_content(&log, "run-maxrounds").trim().is_empty(),
                "expected a non-empty board digest"
            );
        }

        // Scenario 2-bis: `cost_limit` plumbing (OH1 Lot 4 Task 3,
        // reconciliation C). Legacy's standalone `run_blackboard` halts via
        // `HaltReason::CostLimitExceeded` once the `Board`'s `TokenBudget`
        // (seeded by the caller from `OrchestrationConfig::cost_limit`)
        // reports its cost spent; the ES `BlackboardDecider::breached_budget`
        // guard already implements the equivalent check, but until this
        // reconciliation `run_blackboard_es` hard-coded `None` for it,
        // silently ignoring any caller-supplied limit. With `cost_limit:
        // Some(0.0)` and `ScriptedProvider` always reporting `cost: 0.0`,
        // round 0 completes (both agents are invoked exactly once — this is
        // a real breach *after* work happened, not a pre-emptive no-op) and
        // `breached_budget` trips ahead of `max_rounds`/convergence (its
        // priority order in `decide`), halting with `Warned{cost_limit}`
        // rather than looping to `max_rounds` or converging.
        #[tokio::test]
        async fn es_blackboard_halts_at_cost_limit() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), es_test_agent("a", "concrete-model"));
            agents.insert("b".to_string(), es_test_agent("b", "concrete-model"));
            let provider_a = Arc::new(ScriptedProvider::new(&[
                "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:piste A",
            ]));
            let provider_b = Arc::new(ScriptedProvider::new(&[
                "ACTION:FINDING\nCONFIDENCE:0.5\nCONTENT:piste B",
            ]));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), provider_a.clone() as Arc<dyn Provider>);
            providers.insert("b".to_string(), provider_b.clone() as Arc<dyn Provider>);

            let config = BlackboardConfig {
                max_rounds: 50,
                ..BlackboardConfig::default()
            };

            let mut log = InMemoryLog::default();
            let st = run_blackboard_es(
                "run-costlimit",
                "task",
                agents,
                providers,
                config,
                RoutingRules::default(),
                Some(0.0),
                &mut log,
            )
            .await
            .unwrap();

            assert_eq!(st.status, RunStatus::Completed);
            assert!(
                log_has_warned(&log, "run-costlimit", "cost_limit"),
                "expected a cost_limit Warned event in the log"
            );
            assert!(
                !log_has_warned(&log, "run-costlimit", "max_rounds"),
                "cost_limit must trip before max_rounds ever could"
            );
            assert_eq!(
                provider_a.call_count(),
                1,
                "round 0 runs once before the cost guard halts the run"
            );
            assert_eq!(provider_b.call_count(), 1);
            assert!(
                !final_content(&log, "run-costlimit").trim().is_empty(),
                "expected a non-empty board digest"
            );
        }

        // Scenario 3: replay determinism. After a full run, `replay(run_id,
        // &log)` must reconstruct an `ExecutionState` identical (`Debug`
        // format) to the one `run_blackboard_es` returned — and it must do
        // so without invoking any provider again (`replay` takes no
        // `EffectRunner` at all; the call-count assertions below are an
        // extra, belt-and-braces proof that no effect silently re-runs).
        #[tokio::test]
        async fn es_blackboard_replay_reconstructs_state() {
            let mut agents = BTreeMap::new();
            agents.insert("a".to_string(), es_test_agent("a", "concrete-model"));
            agents.insert("b".to_string(), es_test_agent("b", "concrete-model"));
            let provider_a = Arc::new(ScriptedProvider::new(&[
                "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:tout est cohérent",
            ]));
            let provider_b = Arc::new(ScriptedProvider::new(&[
                "ACTION:CONFIRMATION\nTARGET:0\nCONFIDENCE:0.9\nCONTENT:confirmé",
            ]));
            let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
            providers.insert("a".to_string(), provider_a.clone() as Arc<dyn Provider>);
            providers.insert("b".to_string(), provider_b.clone() as Arc<dyn Provider>);

            let mut log = InMemoryLog::default();
            let st = run_blackboard_es(
                "run-replay",
                "task",
                agents,
                providers,
                BlackboardConfig::default(),
                RoutingRules::default(),
                None,
                &mut log,
            )
            .await
            .unwrap();
            assert_eq!(st.status, RunStatus::Completed);

            let calls_a_before = provider_a.call_count();
            let calls_b_before = provider_b.call_count();
            assert!(
                calls_a_before > 0,
                "expected provider 'a' to have been invoked during the run"
            );
            assert!(
                calls_b_before > 0,
                "expected provider 'b' to have been invoked during the run"
            );

            let replayed = replay("run-replay", &log).unwrap();

            assert_eq!(
                format!("{st:?}"),
                format!("{replayed:?}"),
                "replay must reconstruct an identical ExecutionState"
            );
            assert_eq!(
                provider_a.call_count(),
                calls_a_before,
                "replay must not re-invoke provider 'a'"
            );
            assert_eq!(
                provider_b.call_count(),
                calls_b_before,
                "replay must not re-invoke provider 'b'"
            );
        }
    }
}
