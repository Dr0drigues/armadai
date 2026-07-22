//! Result types for hierarchical orchestration.
//!
//! The hierarchical orchestration *engine* is event-sourced and lives in
//! [`super::es::hierarchical`]; it produces these result types via the bridge
//! ([`super::es::bridge::to_orchestration_result`]). This module keeps only the
//! pattern-agnostic result/trace shapes the CLI layer records and displays.

// ── Result types ─────────────────────────────────────────────────

/// Result of a hierarchical orchestration run.
#[derive(Debug)]
pub struct OrchestrationResult {
    /// Final synthesized answer from the coordinator.
    pub content: String,
    /// All delegation events that occurred during the run.
    pub trace: Vec<DelegationEvent>,
    /// Aggregated metrics.
    pub total_tokens_in: u32,
    pub total_tokens_out: u32,
    pub total_cost: f64,
    pub invocation_count: u32,
}

/// A single delegation event in the trace.
#[derive(Debug, Clone)]
pub struct DelegationEvent {
    pub from: String,
    pub to: String,
    pub message: String,
    pub depth: u32,
}
