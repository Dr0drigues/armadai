//! Event-sourcing events for orchestration runs (OH1 Lot 1 socle).
//!
//! `ExecutionEvent` is the append-only log entry type. The log itself is the
//! source of truth; `ExecutionState` (see `super::state`) is a pure
//! projection folded from a sequence of these events via `apply`/`fold`.
//!
//! Variants cover the common run lifecycle plus pattern-specific events for
//! the hierarchical, blackboard and ring orchestration patterns.

use serde::{Deserialize, Serialize};

/// A single event in the execution log.
///
/// Serialized with an internal tag (`t`) and snake_case variant names so the
/// log is stable, human-readable JSON/YAML (e.g. `{"t": "run_started", ...}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ExecutionEvent {
    // ── Common (all patterns) ───────────────────────────────────
    /// A run begins.
    RunStarted {
        run_id: String,
        pattern: String,
        agents: Vec<String>,
        input: String,
        project: Option<String>,
    },
    /// An agent was invoked with a given input.
    AgentInvoked { agent: String, input: String },
    /// An agent produced output (with token/cost accounting).
    AgentObserved {
        agent: String,
        content: String,
        tokens_in: u32,
        tokens_out: u32,
        cost: f64,
        model: String,
    },
    /// The model router selected a tier for an agent.
    ModelRouted {
        agent: String,
        tier: String,
        reason: String,
    },
    /// A non-fatal warning was raised during the run.
    Warned { code: String },
    /// The run was halted before completion (e.g. budget/round limit).
    Halted { reason: String },
    /// The run completed successfully with final content.
    Completed { content: String },

    // ── Hierarchical ─────────────────────────────────────────────
    /// A superior delegated a task to a subordinate.
    Delegated {
        from: String,
        to: String,
        task: String,
        depth: u32,
    },
    /// An agent asked a peer a question.
    AskedPeer {
        from: String,
        to: String,
        question: String,
    },
    /// An agent escalated to a superior.
    Escalated {
        from: String,
        to: String,
        message: String,
    },
    /// An agent synthesized subordinate outputs.
    Synthesized { agent: String, content: String },
    /// A nested team sub-run started under a team lead.
    NestedStarted { team_lead: String, pattern: String },
    /// A nested team sub-run ended.
    NestedEnded { team_lead: String },

    // ── Blackboard ────────────────────────────────────────────────
    /// A new blackboard round started.
    RoundStarted { round: u32 },
    /// An agent added an entry to the shared blackboard.
    BoardEntryAdded {
        agent: String,
        round: u32,
        kind: String,
        content: String,
        refs: Vec<usize>,
        confidence: f32,
        tokens_in: u32,
        tokens_out: u32,
        cost: f64,
    },
    /// Consensus was reached on the blackboard.
    ConsensusReached { score: f32 },

    // ── Ring ──────────────────────────────────────────────────────
    /// A new ring lap started.
    LapStarted { lap: u32 },
    /// An agent added a contribution while holding the ring token.
    ContributionAdded {
        agent: String,
        lap: u32,
        position: usize,
        action: String,
        content: String,
        tokens_in: u32,
        tokens_out: u32,
        cost: f64,
    },
    /// An agent cast a vote on the ring outcome.
    VoteCast {
        agent: String,
        position: String,
        confidence: f32,
        supports: Vec<usize>,
        concerns: Vec<String>,
    },
    /// The ring outcome was resolved.
    OutcomeResolved { outcome: String },
}
