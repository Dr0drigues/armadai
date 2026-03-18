use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::providers::traits::Provider;

// ── Data structures ──────────────────────────────────────────────

/// Central shared state read and written by all agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: Uuid,
    pub task: String,
    pub entries: Vec<BoardEntry>,
    pub round: u32,
    pub state: BoardState,
    pub budget: TokenBudget,
    pub context: HashMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Global state of the board.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BoardState {
    Open,
    Converging,
    Halted { reason: HaltReason },
}

/// Why the board halted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HaltReason {
    Stable,
    Consensus { score: f32 },
    BudgetExhausted,
    MaxRounds,
    Divergence { conflicting_entries: Vec<usize> },
    Cancelled,
}

/// A single contribution on the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardEntry {
    pub index: usize,
    pub agent: String,
    pub round: u32,
    pub kind: EntryKind,
    pub content: String,
    pub references: Vec<usize>,
    pub confidence: f32,
    pub tokens_used: TokenCount,
    pub created_at: DateTime<Utc>,
}

/// Type of board contribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntryKind {
    Finding,
    Challenge { target: usize },
    Confirmation { target: usize },
    Synthesis { sources: Vec<usize> },
    Question,
    Answer { question: usize },
}

/// Token usage for a single contribution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TokenCount {
    pub input: u32,
    pub output: u32,
}

/// Token budget tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub total: u64,
    pub used: u64,
    pub warning_threshold: f32,
}

impl TokenBudget {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            used: 0,
            warning_threshold: 0.80,
        }
    }

    pub fn remaining(&self) -> u64 {
        self.total.saturating_sub(self.used)
    }

    pub fn remaining_ratio(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.remaining() as f32 / self.total as f32
    }

    pub fn exhausted(&self) -> bool {
        self.used >= self.total
    }

    pub fn consume(&mut self, count: TokenCount) {
        self.used += (count.input + count.output) as u64;
    }
}

// ── Snapshot & Delta ─────────────────────────────────────────────

/// Immutable view of the board passed to agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardSnapshot {
    pub task: String,
    pub entries: Vec<BoardEntry>,
    pub round: u32,
    pub state: BoardState,
    pub context: HashMap<String, serde_json::Value>,
    pub budget_remaining: u64,
}

/// Atomic modification proposed by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BoardDelta {
    AddEntry(BoardEntry),
    Annotate { target: usize, note: String },
    ProposeHalt(HaltReason),
}

// ── Board methods ────────────────────────────────────────────────

impl Board {
    /// Create a new board for a task.
    pub fn new(task: String, token_budget: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            task,
            entries: Vec::new(),
            round: 0,
            state: BoardState::Open,
            budget: TokenBudget::new(token_budget),
            context: HashMap::new(),
            created_at: Utc::now(),
        }
    }

    /// Produce an immutable snapshot for agents.
    pub fn snapshot(&self) -> BoardSnapshot {
        BoardSnapshot {
            task: self.task.clone(),
            entries: self.entries.clone(),
            round: self.round,
            state: self.state.clone(),
            context: self.context.clone(),
            budget_remaining: self.budget.remaining(),
        }
    }

    /// Apply a delta to the board.
    pub fn apply(&mut self, delta: BoardDelta) {
        match delta {
            BoardDelta::AddEntry(mut entry) => {
                entry.index = self.entries.len();
                self.budget.consume(entry.tokens_used);
                self.entries.push(entry);
            }
            BoardDelta::Annotate { target, note } => {
                if let Some(entry) = self.entries.get_mut(target) {
                    entry.content.push_str(&format!("\n[annotation] {note}"));
                }
            }
            BoardDelta::ProposeHalt(reason) => {
                self.state = BoardState::Halted { reason };
            }
        }
    }
}

// ── Agent trait ───────────────────────────────────────────────────

/// Trait for agents that participate in a Blackboard pattern.
#[async_trait]
pub trait BoardAgent: Send + Sync {
    /// Unique agent identifier.
    fn name(&self) -> &str;

    /// Agent decides itself whether it can contribute based on the board state.
    fn can_contribute(&self, board: &BoardSnapshot) -> bool;

    /// Contribution priority (higher = earlier when budget is tight).
    fn priority(&self, _board: &BoardSnapshot) -> u8 {
        50
    }

    /// Produce contributions from the current board state.
    async fn contribute(
        &self,
        board: &BoardSnapshot,
        provider: &dyn Provider,
    ) -> anyhow::Result<Vec<BoardDelta>>;
}

// ── Configuration ────────────────────────────────────────────────

/// Configuration for a Blackboard orchestration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardConfig {
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_agent_timeout_secs")]
    pub agent_timeout_secs: u64,
    #[serde(default = "default_bb_consensus_threshold")]
    pub consensus_threshold: f32,
    #[serde(default = "default_divergence_threshold")]
    pub divergence_threshold: f32,
    #[serde(default = "default_bb_token_budget")]
    pub token_budget: u64,
    #[serde(default = "default_convergence_rounds")]
    pub convergence_rounds: u32,
}

fn default_max_rounds() -> u32 {
    5
}
fn default_agent_timeout_secs() -> u64 {
    60
}
fn default_bb_consensus_threshold() -> f32 {
    0.75
}
fn default_divergence_threshold() -> f32 {
    0.60
}
fn default_bb_token_budget() -> u64 {
    50_000
}
fn default_convergence_rounds() -> u32 {
    1
}

impl Default for BlackboardConfig {
    fn default() -> Self {
        Self {
            max_rounds: default_max_rounds(),
            agent_timeout_secs: default_agent_timeout_secs(),
            consensus_threshold: default_bb_consensus_threshold(),
            divergence_threshold: default_divergence_threshold(),
            token_budget: default_bb_token_budget(),
            convergence_rounds: default_convergence_rounds(),
        }
    }
}

impl BlackboardConfig {
    pub fn agent_timeout(&self) -> Duration {
        Duration::from_secs(self.agent_timeout_secs)
    }
}

// ── Execution loop ───────────────────────────────────────────────

/// Check if the board should halt.
fn should_halt(board: &Board, config: &BlackboardConfig) -> bool {
    matches!(board.state, BoardState::Halted { .. })
        || board.round >= config.max_rounds
        || board.budget.exhausted()
}

/// Check convergence conditions.
fn check_convergence(board: &Board, config: &BlackboardConfig) -> Option<HaltReason> {
    let last_round_entries: Vec<_> = board
        .entries
        .iter()
        .filter(|e| e.round == board.round)
        .collect();

    if last_round_entries.is_empty() {
        return Some(HaltReason::Stable);
    }

    let confirmations = last_round_entries
        .iter()
        .filter(|e| matches!(e.kind, EntryKind::Confirmation { .. }))
        .count();
    let challenges = last_round_entries
        .iter()
        .filter(|e| matches!(e.kind, EntryKind::Challenge { .. }))
        .count();

    let total = last_round_entries.len() as f32;
    let consensus_score = confirmations as f32 / total;

    if consensus_score >= config.consensus_threshold {
        return Some(HaltReason::Consensus {
            score: consensus_score,
        });
    }

    // Divergence detection: repeated challenges after round 3
    if board.round >= 3 && challenges as f32 / total > config.divergence_threshold {
        let conflicting = last_round_entries
            .iter()
            .filter(|e| matches!(e.kind, EntryKind::Challenge { .. }))
            .map(|e| e.index)
            .collect();
        return Some(HaltReason::Divergence {
            conflicting_entries: conflicting,
        });
    }

    if board.budget.remaining_ratio() < (1.0 - board.budget.warning_threshold) {
        return Some(HaltReason::BudgetExhausted);
    }

    None
}

/// Run the blackboard execution loop.
pub async fn run_blackboard(
    board: &mut Board,
    agents: &[Arc<dyn BoardAgent>],
    providers: &[Arc<dyn Provider>],
    config: &BlackboardConfig,
) -> anyhow::Result<()> {
    // We need at least one provider; agents share them by index (mod len).
    if providers.is_empty() {
        anyhow::bail!("At least one provider is required for blackboard execution");
    }

    while !should_halt(board, config) {
        let snapshot = board.snapshot();

        // Filter eligible agents
        let eligible: Vec<_> = agents
            .iter()
            .enumerate()
            .filter(|(_, a)| a.can_contribute(&snapshot))
            .collect();

        if eligible.is_empty() {
            board.state = BoardState::Halted {
                reason: HaltReason::Stable,
            };
            break;
        }

        // Execute all eligible agents in parallel
        let timeout_dur = config.agent_timeout();
        let mut handles = Vec::new();

        for (idx, agent) in &eligible {
            let snap = snapshot.clone();
            let provider_idx = *idx % providers.len();
            let agent = Arc::clone(agent);
            let provider = Arc::clone(&providers[provider_idx]);
            let agent_name = agent.name().to_string();

            handles.push(tokio::spawn(async move {
                let result = tokio::time::timeout(
                    timeout_dur,
                    agent.contribute(&snap, provider.as_ref()),
                )
                .await;
                (agent_name, result)
            }));
        }

        for handle in handles {
            let (agent_name, result) = handle.await?;
            match result {
                Ok(Ok(deltas)) => {
                    for delta in deltas {
                        board.apply(delta);
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(agent = %agent_name, error = %e, "agent failed");
                }
                Err(_) => {
                    tracing::warn!(agent = %agent_name, "agent timed out");
                }
            }
        }

        // Evaluate convergence
        if let Some(reason) = check_convergence(board, config) {
            match board.state {
                BoardState::Open => {
                    board.state = BoardState::Converging;
                }
                BoardState::Converging => {
                    board.state = BoardState::Halted { reason };
                    break;
                }
                BoardState::Halted { .. } => break,
            }
        }

        board.round += 1;
    }

    // Handle max rounds
    if board.round >= config.max_rounds && !matches!(board.state, BoardState::Halted { .. }) {
        board.state = BoardState::Halted {
            reason: HaltReason::MaxRounds,
        };
    }

    // Handle budget exhaustion
    if board.budget.exhausted() && !matches!(board.state, BoardState::Halted { .. }) {
        board.state = BoardState::Halted {
            reason: HaltReason::BudgetExhausted,
        };
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_budget_new() {
        let budget = TokenBudget::new(10_000);
        assert_eq!(budget.total, 10_000);
        assert_eq!(budget.used, 0);
        assert_eq!(budget.remaining(), 10_000);
        assert!(!budget.exhausted());
    }

    #[test]
    fn test_token_budget_consume() {
        let mut budget = TokenBudget::new(1000);
        budget.consume(TokenCount {
            input: 300,
            output: 200,
        });
        assert_eq!(budget.used, 500);
        assert_eq!(budget.remaining(), 500);
        assert!(!budget.exhausted());
    }

    #[test]
    fn test_token_budget_exhausted() {
        let mut budget = TokenBudget::new(100);
        budget.consume(TokenCount {
            input: 60,
            output: 50,
        });
        assert!(budget.exhausted());
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn test_token_budget_remaining_ratio() {
        let mut budget = TokenBudget::new(1000);
        budget.consume(TokenCount {
            input: 250,
            output: 250,
        });
        assert!((budget.remaining_ratio() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_token_budget_zero_total() {
        let budget = TokenBudget::new(0);
        assert!(budget.exhausted());
        assert!((budget.remaining_ratio() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_board_new() {
        let board = Board::new("test task".to_string(), 50_000);
        assert_eq!(board.task, "test task");
        assert_eq!(board.round, 0);
        assert_eq!(board.state, BoardState::Open);
        assert!(board.entries.is_empty());
        assert_eq!(board.budget.total, 50_000);
    }

    #[test]
    fn test_board_snapshot() {
        let board = Board::new("task".to_string(), 10_000);
        let snap = board.snapshot();
        assert_eq!(snap.task, "task");
        assert_eq!(snap.round, 0);
        assert_eq!(snap.state, BoardState::Open);
        assert_eq!(snap.budget_remaining, 10_000);
    }

    #[test]
    fn test_board_apply_add_entry() {
        let mut board = Board::new("task".to_string(), 10_000);
        let entry = BoardEntry {
            index: 999, // should be overwritten
            agent: "agent-a".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "found something".to_string(),
            references: vec![],
            confidence: 0.9,
            tokens_used: TokenCount {
                input: 100,
                output: 50,
            },
            created_at: Utc::now(),
        };

        board.apply(BoardDelta::AddEntry(entry));

        assert_eq!(board.entries.len(), 1);
        assert_eq!(board.entries[0].index, 0); // auto-assigned
        assert_eq!(board.entries[0].agent, "agent-a");
        assert_eq!(board.budget.used, 150);
    }

    #[test]
    fn test_board_apply_annotate() {
        let mut board = Board::new("task".to_string(), 10_000);
        let entry = BoardEntry {
            index: 0,
            agent: "agent-a".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "original".to_string(),
            references: vec![],
            confidence: 0.8,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(entry));
        board.apply(BoardDelta::Annotate {
            target: 0,
            note: "extra info".to_string(),
        });

        assert!(board.entries[0].content.contains("[annotation] extra info"));
    }

    #[test]
    fn test_board_apply_propose_halt() {
        let mut board = Board::new("task".to_string(), 10_000);
        board.apply(BoardDelta::ProposeHalt(HaltReason::Cancelled));
        assert_eq!(
            board.state,
            BoardState::Halted {
                reason: HaltReason::Cancelled
            }
        );
    }

    #[test]
    fn test_should_halt_on_halted_state() {
        let mut board = Board::new("task".to_string(), 10_000);
        board.state = BoardState::Halted {
            reason: HaltReason::Stable,
        };
        let config = BlackboardConfig::default();
        assert!(should_halt(&board, &config));
    }

    #[test]
    fn test_should_halt_on_max_rounds() {
        let mut board = Board::new("task".to_string(), 10_000);
        board.round = 5;
        let config = BlackboardConfig::default(); // max_rounds = 5
        assert!(should_halt(&board, &config));
    }

    #[test]
    fn test_should_halt_on_budget_exhausted() {
        let mut board = Board::new("task".to_string(), 100);
        board.budget.used = 100;
        let config = BlackboardConfig::default();
        assert!(should_halt(&board, &config));
    }

    #[test]
    fn test_should_not_halt_normal() {
        let board = Board::new("task".to_string(), 10_000);
        let config = BlackboardConfig::default();
        assert!(!should_halt(&board, &config));
    }

    #[test]
    fn test_check_convergence_empty_round() {
        let board = Board::new("task".to_string(), 10_000);
        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert_eq!(result, Some(HaltReason::Stable));
    }

    #[test]
    fn test_check_convergence_high_consensus() {
        let mut board = Board::new("task".to_string(), 50_000);
        // Add 4 confirmations out of 5 entries
        for i in 0..4 {
            board.entries.push(BoardEntry {
                index: i,
                agent: format!("agent-{i}"),
                round: 0,
                kind: EntryKind::Confirmation { target: 0 },
                content: "agree".to_string(),
                references: vec![],
                confidence: 0.9,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            });
        }
        board.entries.push(BoardEntry {
            index: 4,
            agent: "agent-4".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "new finding".to_string(),
            references: vec![],
            confidence: 0.7,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        });

        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert!(matches!(result, Some(HaltReason::Consensus { .. })));
    }

    #[test]
    fn test_check_convergence_no_convergence() {
        let mut board = Board::new("task".to_string(), 50_000);
        // All findings, no confirmations
        for i in 0..5 {
            board.entries.push(BoardEntry {
                index: i,
                agent: format!("agent-{i}"),
                round: 0,
                kind: EntryKind::Finding,
                content: "finding".to_string(),
                references: vec![],
                confidence: 0.5,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            });
        }
        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_convergence_divergence() {
        let mut board = Board::new("task".to_string(), 50_000);
        board.round = 3;
        // All challenges in round 3
        for i in 0..5 {
            board.entries.push(BoardEntry {
                index: i,
                agent: format!("agent-{i}"),
                round: 3,
                kind: EntryKind::Challenge { target: 0 },
                content: "disagree".to_string(),
                references: vec![],
                confidence: 0.5,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            });
        }
        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert!(matches!(result, Some(HaltReason::Divergence { .. })));
    }

    #[test]
    fn test_blackboard_config_defaults() {
        let config = BlackboardConfig::default();
        assert_eq!(config.max_rounds, 5);
        assert_eq!(config.agent_timeout_secs, 60);
        assert!((config.consensus_threshold - 0.75).abs() < f32::EPSILON);
        assert!((config.divergence_threshold - 0.60).abs() < f32::EPSILON);
        assert_eq!(config.token_budget, 50_000);
        assert_eq!(config.convergence_rounds, 1);
        assert_eq!(config.agent_timeout(), Duration::from_secs(60));
    }

    #[test]
    fn test_entry_kind_variants() {
        let _finding = EntryKind::Finding;
        let _challenge = EntryKind::Challenge { target: 0 };
        let _confirmation = EntryKind::Confirmation { target: 1 };
        let _synthesis = EntryKind::Synthesis {
            sources: vec![0, 1],
        };
        let _question = EntryKind::Question;
        let _answer = EntryKind::Answer { question: 2 };
    }

    #[test]
    fn test_board_delta_variants() {
        let entry = BoardEntry {
            index: 0,
            agent: "test".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "test".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        let _add = BoardDelta::AddEntry(entry);
        let _annotate = BoardDelta::Annotate {
            target: 0,
            note: "note".to_string(),
        };
        let _halt = BoardDelta::ProposeHalt(HaltReason::Cancelled);
    }

    #[test]
    fn test_halt_reason_variants() {
        assert_eq!(HaltReason::Stable, HaltReason::Stable);
        assert_ne!(HaltReason::Stable, HaltReason::MaxRounds);
        assert_ne!(HaltReason::Stable, HaltReason::BudgetExhausted);
        assert_ne!(HaltReason::Stable, HaltReason::Cancelled);
    }

    #[test]
    fn test_board_state_variants() {
        assert_eq!(BoardState::Open, BoardState::Open);
        assert_eq!(BoardState::Converging, BoardState::Converging);
        assert_ne!(BoardState::Open, BoardState::Converging);
    }

    #[test]
    fn test_board_multiple_entries_indexing() {
        let mut board = Board::new("task".to_string(), 50_000);
        for i in 0..3 {
            let entry = BoardEntry {
                index: 0,
                agent: format!("agent-{i}"),
                round: 0,
                kind: EntryKind::Finding,
                content: format!("finding {i}"),
                references: vec![],
                confidence: 0.5,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            };
            board.apply(BoardDelta::AddEntry(entry));
        }
        assert_eq!(board.entries.len(), 3);
        assert_eq!(board.entries[0].index, 0);
        assert_eq!(board.entries[1].index, 1);
        assert_eq!(board.entries[2].index, 2);
    }

    #[tokio::test]
    async fn test_run_blackboard_no_providers() {
        let mut board = Board::new("task".to_string(), 10_000);
        let agents: Vec<Arc<dyn BoardAgent>> = vec![];
        let providers: Vec<Arc<dyn Provider>> = vec![];
        let config = BlackboardConfig::default();
        let result = run_blackboard(&mut board, &agents, &providers, &config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_blackboard_no_agents_halts_stable() {
        use crate::providers::traits::*;

        struct DummyProvider;
        #[async_trait]
        impl Provider for DummyProvider {
            async fn complete(&self, _: CompletionRequest) -> anyhow::Result<CompletionResponse> {
                Ok(CompletionResponse {
                    content: "ok".to_string(),
                    model: "test".to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                })
            }
            async fn stream(&self, _: CompletionRequest) -> anyhow::Result<TokenStream> {
                unimplemented!()
            }
            fn metadata(&self) -> ProviderMetadata {
                ProviderMetadata {
                    name: "test".to_string(),
                    models: vec![],
                    supports_streaming: false,
                }
            }
        }

        let mut board = Board::new("task".to_string(), 10_000);
        let agents: Vec<Arc<dyn BoardAgent>> = vec![];
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(DummyProvider)];
        let config = BlackboardConfig::default();
        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();
        assert_eq!(
            board.state,
            BoardState::Halted {
                reason: HaltReason::Stable
            }
        );
    }
}
