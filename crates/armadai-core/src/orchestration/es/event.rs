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
        /// The run's roster metadata: agent name -> `(provider, configured
        /// model)`, mirroring the live path's `agent_meta`/`agent_meta_from_roster`
        /// (`cli::run.rs`) — see [`super::bridge::roster_from_agents`], which
        /// every production `RunStarted` emission site builds this from.
        /// Persisted here (rather than only threaded through in-memory at
        /// emission time) because `ExecutionEvent` otherwise never records an
        /// agent's provider anywhere, and the model configured for an
        /// agent's first invocation isn't reconstructable from the rest of
        /// the log either — so a read-only consumer (`armadai run --replay`)
        /// would have no way to enrich `AgentInvoked -> AgentStart` with the
        /// run's real provider/model without it. `#[serde(default)]` is
        /// required: event logs written before this field existed have no
        /// `roster` key at all, and must still deserialize — they fall back
        /// to an empty roster, i.e. the documented "no roster available"
        /// behavior (`AgentStart` carries empty `prov`/`model`), not a hard
        /// error.
        #[serde(default)]
        roster: std::collections::BTreeMap<String, (String, String)>,
    },
    /// A snapshot of the run's orchestration config (serialized as JSON).
    /// Emitted immediately after `RunStarted` by the blackboard, ring, and
    /// hierarchical engines. Direct runs have no orchestration config and do
    /// not emit this event.
    #[serde(rename = "config")]
    ConfigSnapshot { config_json: String },
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
    /// A delegated invocation failed. Recorded instead of aborting the run
    /// (collect-and-record): the reducer pushes an `assistant` marker so the
    /// agent reads as "settled" (a coordinator that awaits this child then
    /// synthesizes over the partial results), and the run continues.
    AgentFailed { agent: String, error: String },

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

/// The `assistant`-role content recorded for a failed delegation. Single
/// source of the marker string, consumed by both the reducer (`apply`) and
/// the `RunEvent` bridge so they never drift. Contains no `@agent:` marker,
/// so the hierarchical `is_final_answer` reads it as a plain final answer.
pub fn delegation_failed_content(error: &str) -> String {
    format!("[Delegation failed: {error}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Backward-compatibility regression lock (bugfix `fix/replay-prov-model-roster`):
    /// an event log written by a version of ArmadAI that predates the
    /// `roster` field has no `roster` key in its persisted `RunStarted` JSON
    /// at all. `#[serde(default)]` must make that still deserialize
    /// successfully — falling back to an empty roster (today's documented
    /// "no roster available" fallback, see `bridge.rs`'s
    /// `agent_invoked_maps_to_agent_start_with_empty_prov_model_when_meta_absent`)
    /// rather than a hard deserialization error that would make an old log
    /// unreadable by `armadai run --replay`/`--resume`.
    #[test]
    fn run_started_without_roster_field_deserializes_to_empty_default() {
        let json = r#"{
            "t": "run_started",
            "run_id": "r",
            "pattern": "direct",
            "agents": ["solo"],
            "input": "do the thing",
            "project": null
        }"#;

        let event: ExecutionEvent = serde_json::from_str(json).expect(
            "a RunStarted event with no persisted `roster` key must still deserialize \
             (old event logs predate this field)",
        );

        match event {
            ExecutionEvent::RunStarted { roster, .. } => {
                assert!(
                    roster.is_empty(),
                    "roster must default to empty when absent from the JSON, got: {roster:?}"
                );
            }
            other => panic!("expected RunStarted, got {other:?}"),
        }
    }

    /// Round-trip proof that a populated `roster` survives serialize ->
    /// deserialize unchanged, complementing the "absent field" case above.
    #[test]
    fn run_started_roster_round_trips_through_json() {
        let mut roster = std::collections::BTreeMap::new();
        roster.insert(
            "solo".to_string(),
            ("anthropic".to_string(), "concrete-model".to_string()),
        );
        let event = ExecutionEvent::RunStarted {
            run_id: "r".into(),
            pattern: "direct".into(),
            agents: vec!["solo".into()],
            input: "do the thing".into(),
            project: None,
            roster: roster.clone(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let round_tripped: ExecutionEvent = serde_json::from_str(&json).unwrap();

        match round_tripped {
            ExecutionEvent::RunStarted { roster: got, .. } => assert_eq!(got, roster),
            other => panic!("expected RunStarted, got {other:?}"),
        }
    }
}
