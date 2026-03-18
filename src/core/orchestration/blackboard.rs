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
    pub entries: Arc<Vec<BoardEntry>>,
    pub round: u32,
    pub(crate) state: BoardState,
    pub(crate) budget: TokenBudget,
    pub context: HashMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub(crate) consecutive_convergence: u32,
    pub(crate) halt_proposals: Vec<HaltReason>,
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
    /// Percentage of budget consumed that triggers a warning log (e.g., 0.80 = warn at 80% consumed).
    pub budget_warning_pct: f32,
}

impl TokenBudget {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            used: 0,
            budget_warning_pct: 0.80,
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
        self.used += count.input as u64 + count.output as u64;
    }
}

// ── Snapshot & Delta ─────────────────────────────────────────────

/// Immutable view of the board passed to agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardSnapshot {
    pub task: String,
    pub entries: Arc<Vec<BoardEntry>>,
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
            entries: Arc::new(Vec::new()),
            round: 0,
            state: BoardState::Open,
            budget: TokenBudget::new(token_budget),
            context: HashMap::new(),
            created_at: Utc::now(),
            consecutive_convergence: 0,
            halt_proposals: Vec::new(),
        }
    }

    /// Accessor for the board state.
    pub fn state(&self) -> &BoardState {
        &self.state
    }

    /// Check if the board has halted.
    pub fn is_halted(&self) -> bool {
        matches!(self.state, BoardState::Halted { .. })
    }

    /// Accessor for the budget.
    pub fn budget(&self) -> &TokenBudget {
        &self.budget
    }

    /// Accessor for entries as a slice.
    pub fn entries(&self) -> &[BoardEntry] {
        &self.entries
    }

    /// Produce an immutable snapshot for agents (cheap: Arc clone).
    pub fn snapshot(&self) -> BoardSnapshot {
        BoardSnapshot {
            task: self.task.clone(),
            entries: Arc::clone(&self.entries),
            round: self.round,
            state: self.state.clone(),
            context: self.context.clone(),
            budget_remaining: self.budget.remaining(),
        }
    }

    /// Validate that all target indices in an entry kind are within bounds.
    fn validate_entry_references(
        &self,
        entry: &BoardEntry,
        current_len: usize,
    ) -> anyhow::Result<()> {
        match &entry.kind {
            EntryKind::Challenge { target } | EntryKind::Confirmation { target } => {
                if *target >= current_len {
                    anyhow::bail!(
                        "entry references target index {target} but board has {current_len} entries"
                    );
                }
            }
            EntryKind::Synthesis { sources } => {
                for &src in sources {
                    if src >= current_len {
                        anyhow::bail!(
                            "synthesis references source index {src} but board has {current_len} entries"
                        );
                    }
                }
            }
            EntryKind::Answer { question } => {
                if *question >= current_len {
                    anyhow::bail!(
                        "answer references question index {question} but board has {current_len} entries"
                    );
                }
            }
            EntryKind::Finding | EntryKind::Question => {}
        }
        Ok(())
    }

    /// Apply a delta to the board.
    pub fn apply(&mut self, delta: BoardDelta) -> anyhow::Result<()> {
        match delta {
            BoardDelta::AddEntry(mut entry) => {
                let current_len = self.entries.len();
                entry.index = current_len;
                self.validate_entry_references(&entry, current_len)?;
                self.budget.consume(entry.tokens_used);
                Arc::make_mut(&mut self.entries).push(entry);
            }
            BoardDelta::Annotate { target, note } => {
                let entries = Arc::make_mut(&mut self.entries);
                if let Some(entry) = entries.get_mut(target) {
                    entry.content.push_str("\n[annotation] ");
                    entry.content.push_str(&note);
                }
            }
            BoardDelta::ProposeHalt(reason) => {
                self.halt_proposals.push(reason);
            }
        }
        Ok(())
    }

    /// Check if pending halt proposals should trigger a halt.
    /// Halts if majority of agents proposed, or if a round passed with no objection.
    pub(crate) fn check_halt_proposals(&mut self, total_agents: usize) -> bool {
        if self.halt_proposals.is_empty() {
            return false;
        }
        // Majority proposed halt
        if self.halt_proposals.len() * 2 > total_agents {
            let reason = self.halt_proposals[0].clone();
            self.state = BoardState::Halted { reason };
            return true;
        }
        // Check if this round had no objection (no new findings/challenges)
        let has_objection = self
            .entries
            .iter()
            .filter(|e| e.round == self.round)
            .any(|e| matches!(e.kind, EntryKind::Finding | EntryKind::Challenge { .. }));
        if !has_objection {
            let reason = self.halt_proposals[0].clone();
            self.state = BoardState::Halted { reason };
            return true;
        }
        false
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

const fn default_max_rounds() -> u32 {
    5
}
const fn default_agent_timeout_secs() -> u64 {
    60
}
const fn default_bb_consensus_threshold() -> f32 {
    0.75
}
const fn default_divergence_threshold() -> f32 {
    0.60
}
const fn default_bb_token_budget() -> u64 {
    50_000
}
const fn default_convergence_rounds() -> u32 {
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

    /// Validate that config thresholds are in valid range (0.0..=1.0).
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(0.0..=1.0).contains(&self.consensus_threshold) {
            anyhow::bail!(
                "consensus_threshold must be in 0.0..=1.0, got {}",
                self.consensus_threshold
            );
        }
        if !(0.0..=1.0).contains(&self.divergence_threshold) {
            anyhow::bail!(
                "divergence_threshold must be in 0.0..=1.0, got {}",
                self.divergence_threshold
            );
        }
        Ok(())
    }
}

// ── Execution loop ───────────────────────────────────────────────

/// Check if the board should halt.
fn should_halt(board: &Board, config: &BlackboardConfig) -> bool {
    matches!(board.state, BoardState::Halted { .. })
        || board.round >= config.max_rounds
        || board.budget.exhausted()
}

/// Check convergence conditions (budget check removed — budget exhaustion only in should_halt).
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

    // Log a warning when budget nears the threshold (but do not halt)
    if board.budget.remaining_ratio() < (1.0 - board.budget.budget_warning_pct) {
        tracing::warn!(
            remaining_ratio = board.budget.remaining_ratio(),
            "token budget nearing exhaustion"
        );
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
    config.validate()?;

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
                let result =
                    tokio::time::timeout(timeout_dur, agent.contribute(&snap, provider.as_ref()))
                        .await;
                (agent_name, result)
            }));
        }

        for handle in handles {
            let (agent_name, result) = handle.await?;
            match result {
                Ok(Ok(deltas)) => {
                    for delta in deltas {
                        if let Err(e) = board.apply(delta) {
                            tracing::warn!(agent = %agent_name, error = %e, "invalid delta");
                        }
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

        // Check pending halt proposals
        if board.check_halt_proposals(agents.len()) {
            break;
        }

        // Evaluate convergence with consecutive round tracking
        if let Some(reason) = check_convergence(board, config) {
            board.consecutive_convergence += 1;
            if board.consecutive_convergence >= config.convergence_rounds {
                board.state = BoardState::Halted { reason };
                break;
            } else {
                board.state = BoardState::Converging;
            }
        } else {
            // Convergence not detected — reset counter and state
            board.consecutive_convergence = 0;
            if matches!(board.state, BoardState::Converging) {
                board.state = BoardState::Open;
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
    fn test_token_budget_u32_overflow() {
        let mut budget = TokenBudget::new(u64::MAX);
        budget.consume(TokenCount {
            input: u32::MAX,
            output: u32::MAX,
        });
        // Should not overflow: u32::MAX + u32::MAX fits in u64
        assert_eq!(budget.used, u32::MAX as u64 + u32::MAX as u64);
    }

    #[test]
    fn test_board_new() {
        let board = Board::new("test task".to_string(), 50_000);
        assert_eq!(board.task, "test task");
        assert_eq!(board.round, 0);
        assert_eq!(*board.state(), BoardState::Open);
        assert!(board.entries().is_empty());
        assert_eq!(board.budget().total, 50_000);
        assert_eq!(board.consecutive_convergence, 0);
        assert!(board.halt_proposals.is_empty());
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
    fn test_board_snapshot_shares_arc() {
        let mut board = Board::new("task".to_string(), 10_000);
        let entry = BoardEntry {
            index: 0,
            agent: "a".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "f".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(entry)).unwrap();
        let snap = board.snapshot();
        // Snapshot shares the Arc — same pointer
        assert!(Arc::ptr_eq(&board.entries, &snap.entries));
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

        board.apply(BoardDelta::AddEntry(entry)).unwrap();

        assert_eq!(board.entries().len(), 1);
        assert_eq!(board.entries()[0].index, 0); // auto-assigned
        assert_eq!(board.entries()[0].agent, "agent-a");
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
        board.apply(BoardDelta::AddEntry(entry)).unwrap();
        board
            .apply(BoardDelta::Annotate {
                target: 0,
                note: "extra info".to_string(),
            })
            .unwrap();

        assert!(
            board.entries()[0]
                .content
                .contains("[annotation] extra info")
        );
    }

    #[test]
    fn test_board_apply_propose_halt_pending() {
        let mut board = Board::new("task".to_string(), 10_000);
        board
            .apply(BoardDelta::ProposeHalt(HaltReason::Cancelled))
            .unwrap();
        // ProposeHalt no longer immediately halts — it adds a pending proposal
        assert_eq!(*board.state(), BoardState::Open);
        assert_eq!(board.halt_proposals.len(), 1);
    }

    #[test]
    fn test_halt_proposals_majority() {
        let mut board = Board::new("task".to_string(), 10_000);
        board
            .apply(BoardDelta::ProposeHalt(HaltReason::Cancelled))
            .unwrap();
        board
            .apply(BoardDelta::ProposeHalt(HaltReason::Cancelled))
            .unwrap();
        // 2 proposals out of 3 agents = majority
        assert!(board.check_halt_proposals(3));
        assert!(board.is_halted());
    }

    #[test]
    fn test_halt_proposals_no_majority() {
        let mut board = Board::new("task".to_string(), 10_000);
        // Add a finding in the current round (objection)
        let entry = BoardEntry {
            index: 0,
            agent: "agent-a".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "new finding".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(entry)).unwrap();
        board
            .apply(BoardDelta::ProposeHalt(HaltReason::Cancelled))
            .unwrap();
        // 1 proposal out of 5 agents with objection = no halt
        assert!(!board.check_halt_proposals(5));
        assert!(!board.is_halted());
    }

    #[test]
    fn test_halt_proposals_no_objection() {
        let mut board = Board::new("task".to_string(), 10_000);
        // Only a confirmation in this round (not an objection)
        let entry = BoardEntry {
            index: 0,
            agent: "agent-b".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "base".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(entry)).unwrap();
        board.round = 1;
        let confirm = BoardEntry {
            index: 0,
            agent: "agent-c".to_string(),
            round: 1,
            kind: EntryKind::Confirmation { target: 0 },
            content: "agreed".to_string(),
            references: vec![],
            confidence: 0.9,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(confirm)).unwrap();
        board
            .apply(BoardDelta::ProposeHalt(HaltReason::Stable))
            .unwrap();
        // 1 proposal out of 5, but no objection (no findings/challenges in this round)
        assert!(board.check_halt_proposals(5));
        assert!(board.is_halted());
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
        // Need a base entry to confirm
        let base = BoardEntry {
            index: 0,
            agent: "base".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "base".to_string(),
            references: vec![],
            confidence: 0.9,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(base)).unwrap();
        board.round = 1;
        // Add 4 confirmations out of 5 entries in round 1
        for i in 0..4 {
            let entry = BoardEntry {
                index: 0,
                agent: format!("agent-{i}"),
                round: 1,
                kind: EntryKind::Confirmation { target: 0 },
                content: "agree".to_string(),
                references: vec![],
                confidence: 0.9,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            };
            board.apply(BoardDelta::AddEntry(entry)).unwrap();
        }
        let new_finding = BoardEntry {
            index: 0,
            agent: "agent-4".to_string(),
            round: 1,
            kind: EntryKind::Finding,
            content: "new finding".to_string(),
            references: vec![],
            confidence: 0.7,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(new_finding)).unwrap();

        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert!(matches!(result, Some(HaltReason::Consensus { .. })));
    }

    #[test]
    fn test_check_convergence_no_convergence() {
        let mut board = Board::new("task".to_string(), 50_000);
        // All findings, no confirmations
        for i in 0..5 {
            let entry = BoardEntry {
                index: 0,
                agent: format!("agent-{i}"),
                round: 0,
                kind: EntryKind::Finding,
                content: "finding".to_string(),
                references: vec![],
                confidence: 0.5,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            };
            board.apply(BoardDelta::AddEntry(entry)).unwrap();
        }
        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_convergence_divergence() {
        let mut board = Board::new("task".to_string(), 50_000);
        // Add a base entry to challenge
        let base = BoardEntry {
            index: 0,
            agent: "base".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "base".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(base)).unwrap();
        board.round = 3;
        // All challenges in round 3
        for i in 0..5 {
            let entry = BoardEntry {
                index: 0,
                agent: format!("agent-{i}"),
                round: 3,
                kind: EntryKind::Challenge { target: 0 },
                content: "disagree".to_string(),
                references: vec![],
                confidence: 0.5,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            };
            board.apply(BoardDelta::AddEntry(entry)).unwrap();
        }
        let config = BlackboardConfig::default();
        let result = check_convergence(&board, &config);
        assert!(matches!(result, Some(HaltReason::Divergence { .. })));
    }

    #[test]
    fn test_check_convergence_budget_warning_not_halt() {
        let mut board = Board::new("task".to_string(), 1000);
        board.budget.used = 850; // 85% used, past budget_warning_pct (80%)
        // Add a finding so it's not an empty round
        let entry = BoardEntry {
            index: 0,
            agent: "a".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "f".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(entry)).unwrap();
        let config = BlackboardConfig::default();
        // Budget warning should NOT cause halt (only a log warning)
        let result = check_convergence(&board, &config);
        assert!(result.is_none());
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
    fn test_blackboard_config_validate_ok() {
        let config = BlackboardConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_blackboard_config_validate_bad_consensus() {
        let config = BlackboardConfig {
            consensus_threshold: 1.5,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_blackboard_config_validate_bad_divergence() {
        let config = BlackboardConfig {
            divergence_threshold: -0.1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
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
            board.apply(BoardDelta::AddEntry(entry)).unwrap();
        }
        assert_eq!(board.entries().len(), 3);
        assert_eq!(board.entries()[0].index, 0);
        assert_eq!(board.entries()[1].index, 1);
        assert_eq!(board.entries()[2].index, 2);
    }

    #[test]
    fn test_validate_references_invalid_target() {
        let mut board = Board::new("task".to_string(), 10_000);
        let entry = BoardEntry {
            index: 0,
            agent: "a".to_string(),
            round: 0,
            kind: EntryKind::Challenge { target: 99 },
            content: "bad ref".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        assert!(board.apply(BoardDelta::AddEntry(entry)).is_err());
    }

    #[test]
    fn test_validate_references_valid_target() {
        let mut board = Board::new("task".to_string(), 10_000);
        // Add a base entry first
        let base = BoardEntry {
            index: 0,
            agent: "a".to_string(),
            round: 0,
            kind: EntryKind::Finding,
            content: "base".to_string(),
            references: vec![],
            confidence: 0.5,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        board.apply(BoardDelta::AddEntry(base)).unwrap();
        // Now confirm the base entry
        let confirm = BoardEntry {
            index: 0,
            agent: "b".to_string(),
            round: 0,
            kind: EntryKind::Confirmation { target: 0 },
            content: "agreed".to_string(),
            references: vec![],
            confidence: 0.9,
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        };
        assert!(board.apply(BoardDelta::AddEntry(confirm)).is_ok());
    }

    #[test]
    fn test_convergence_rounds_tracking() {
        let mut board = Board::new("task".to_string(), 50_000);
        let config = BlackboardConfig {
            convergence_rounds: 3,
            ..Default::default()
        };

        // Simulate 3 rounds of convergence
        for _ in 0..3 {
            board.consecutive_convergence += 1;
        }
        assert!(board.consecutive_convergence >= config.convergence_rounds);

        // Reset on non-convergence
        board.consecutive_convergence = 0;
        assert!(board.consecutive_convergence < config.convergence_rounds);
    }

    #[test]
    fn test_board_accessors() {
        let board = Board::new("task".to_string(), 10_000);
        assert_eq!(*board.state(), BoardState::Open);
        assert!(!board.is_halted());
        assert_eq!(board.budget().total, 10_000);
        assert!(board.entries().is_empty());
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
        let mut board = Board::new("task".to_string(), 10_000);
        let agents: Vec<Arc<dyn BoardAgent>> = vec![];
        let providers = crate::core::orchestration::test_helpers::noop_providers();
        let config = BlackboardConfig::default();
        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();
        assert_eq!(
            *board.state(),
            BoardState::Halted {
                reason: HaltReason::Stable
            }
        );
    }

    // ── Integration tests with mock agents ────────────────────────

    /// Mock agent that produces Findings, then Confirmations after round 1.
    struct FindThenConfirmAgent {
        id: String,
    }

    #[async_trait]
    impl BoardAgent for FindThenConfirmAgent {
        fn name(&self) -> &str {
            &self.id
        }

        fn can_contribute(&self, _board: &BoardSnapshot) -> bool {
            true
        }

        async fn contribute(
            &self,
            board: &BoardSnapshot,
            _provider: &dyn Provider,
        ) -> anyhow::Result<Vec<BoardDelta>> {
            if board.round == 0 {
                Ok(vec![BoardDelta::AddEntry(BoardEntry {
                    index: 0,
                    agent: self.id.clone(),
                    round: board.round,
                    kind: EntryKind::Finding,
                    content: format!("finding from {}", self.id),
                    references: vec![],
                    confidence: 0.8,
                    tokens_used: TokenCount {
                        input: 10,
                        output: 10,
                    },
                    created_at: Utc::now(),
                })])
            } else {
                // Confirm the first entry
                Ok(vec![BoardDelta::AddEntry(BoardEntry {
                    index: 0,
                    agent: self.id.clone(),
                    round: board.round,
                    kind: EntryKind::Confirmation { target: 0 },
                    content: "confirmed".to_string(),
                    references: vec![],
                    confidence: 0.9,
                    tokens_used: TokenCount {
                        input: 10,
                        output: 10,
                    },
                    created_at: Utc::now(),
                })])
            }
        }
    }

    #[tokio::test]
    async fn test_integration_blackboard_consensus() {
        let mut board = Board::new("review code".to_string(), 50_000);
        let agents: Vec<Arc<dyn BoardAgent>> = vec![
            Arc::new(FindThenConfirmAgent {
                id: "agent-a".into(),
            }),
            Arc::new(FindThenConfirmAgent {
                id: "agent-b".into(),
            }),
        ];
        let providers = crate::core::orchestration::test_helpers::noop_providers();
        let config = BlackboardConfig::default();

        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();

        assert!(board.is_halted());
        match board.state() {
            BoardState::Halted {
                reason: HaltReason::Consensus { .. },
            } => {}
            other => panic!("Expected Consensus halt, got {other:?}"),
        }
    }

    /// Mock agent that always times out.
    struct TimeoutAgent;

    #[async_trait]
    impl BoardAgent for TimeoutAgent {
        fn name(&self) -> &str {
            "timeout-agent"
        }
        fn can_contribute(&self, _board: &BoardSnapshot) -> bool {
            true
        }
        async fn contribute(
            &self,
            _board: &BoardSnapshot,
            _provider: &dyn Provider,
        ) -> anyhow::Result<Vec<BoardDelta>> {
            tokio::time::sleep(Duration::from_secs(300)).await;
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_integration_blackboard_agent_timeout() {
        let mut board = Board::new("task".to_string(), 50_000);
        let agents: Vec<Arc<dyn BoardAgent>> = vec![Arc::new(TimeoutAgent)];
        let providers = crate::core::orchestration::test_helpers::noop_providers();
        let config = BlackboardConfig {
            agent_timeout_secs: 1,
            max_rounds: 2,
            ..Default::default()
        };

        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();

        // Should halt (stable, since timeout agent produces nothing each round)
        assert!(board.is_halted());
    }

    #[tokio::test]
    async fn test_integration_blackboard_max_rounds_zero() {
        let mut board = Board::new("task".to_string(), 50_000);
        let agents: Vec<Arc<dyn BoardAgent>> =
            vec![Arc::new(FindThenConfirmAgent { id: "a".into() })];
        let providers = crate::core::orchestration::test_helpers::noop_providers();
        let config = BlackboardConfig {
            max_rounds: 0,
            ..Default::default()
        };

        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();

        assert!(board.is_halted());
        assert_eq!(
            *board.state(),
            BoardState::Halted {
                reason: HaltReason::MaxRounds
            }
        );
    }

    #[tokio::test]
    async fn test_integration_blackboard_token_budget_zero() {
        let mut board = Board::new("task".to_string(), 0);
        let agents: Vec<Arc<dyn BoardAgent>> =
            vec![Arc::new(FindThenConfirmAgent { id: "a".into() })];
        let providers = crate::core::orchestration::test_helpers::noop_providers();
        let config = BlackboardConfig::default();

        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();

        assert!(board.is_halted());
        assert_eq!(
            *board.state(),
            BoardState::Halted {
                reason: HaltReason::BudgetExhausted
            }
        );
    }
}
