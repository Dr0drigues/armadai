use std::time::Duration;

use serde::{Deserialize, Serialize};

// ── Data structures ──────────────────────────────────────────────

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

/// Map an `EntryKind` variant to a lowercase name (used for JSONL events and
/// trigger matching in `llm_agents`).
pub(crate) fn entry_kind_name(kind: &EntryKind) -> &'static str {
    match kind {
        EntryKind::Finding => "finding",
        EntryKind::Challenge { .. } => "challenge",
        EntryKind::Confirmation { .. } => "confirmation",
        EntryKind::Synthesis { .. } => "synthesis",
        EntryKind::Question => "question",
        EntryKind::Answer { .. } => "answer",
    }
}

/// Token usage and cost for a single contribution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TokenCount {
    pub input: u32,
    pub output: u32,
    #[serde(default)]
    pub cost: f64,
}

/// Token budget and cost limit tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub total: u64,
    pub used: u64,
    /// Percentage of budget consumed that triggers a warning log (e.g., 0.80 = warn at 80% consumed).
    pub budget_warning_pct: f32,
    /// Optional cost limit in USD.
    #[serde(default)]
    pub cost_limit: Option<f64>,
    /// Total cost consumed so far.
    #[serde(default)]
    pub cost_used: f64,
}

impl TokenBudget {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            used: 0,
            budget_warning_pct: 0.80,
            cost_limit: None,
            cost_used: 0.0,
        }
    }

    pub fn with_cost_limit(total: u64, cost_limit: Option<f64>) -> Self {
        Self {
            total,
            used: 0,
            budget_warning_pct: 0.80,
            cost_limit,
            cost_used: 0.0,
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
        let token_exhausted = self.used >= self.total;
        let cost_exhausted = self.cost_limit.is_some_and(|limit| self.cost_used >= limit);
        token_exhausted || cost_exhausted
    }

    pub fn consume(&mut self, count: TokenCount) {
        self.used += count.input as u64 + count.output as u64;
        self.cost_used += count.cost;
    }
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
    // Safety cap, not a tight limit (aligned with ring): high enough for
    // normal multi-round blackboards with verbose real agents to converge,
    // low enough to catch a runaway. Tunable via `blackboard.token_budget`.
    500_000
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
            cost: 0.0,
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
            cost: 0.0,
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
            cost: 0.0,
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
            cost: 0.0,
        });
        // Should not overflow: u32::MAX + u32::MAX fits in u64
        assert_eq!(budget.used, u32::MAX as u64 + u32::MAX as u64);
    }

    #[test]
    fn test_blackboard_config_defaults() {
        let config = BlackboardConfig::default();
        assert_eq!(config.max_rounds, 5);
        assert_eq!(config.agent_timeout_secs, 60);
        assert!((config.consensus_threshold - 0.75).abs() < f32::EPSILON);
        assert!((config.divergence_threshold - 0.60).abs() < f32::EPSILON);
        assert_eq!(config.token_budget, 500_000);
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
    fn test_token_budget_with_cost_limit() {
        let mut budget = TokenBudget::with_cost_limit(1000, Some(0.01));
        assert_eq!(budget.total, 1000);
        assert_eq!(budget.cost_limit, Some(0.01));
        assert_eq!(budget.cost_used, 0.0);
        assert!(!budget.exhausted());

        // Consume tokens but stay under cost limit
        budget.consume(TokenCount {
            input: 100,
            output: 100,
            cost: 0.005,
        });
        assert!(!budget.exhausted());

        // Exceed cost limit
        budget.consume(TokenCount {
            input: 100,
            output: 100,
            cost: 0.006, // Total: 0.011, exceeds 0.01
        });
        assert!(budget.exhausted());
    }

    #[test]
    fn test_token_budget_exhausted_by_tokens_only() {
        let mut budget = TokenBudget::with_cost_limit(100, Some(1.0));
        budget.consume(TokenCount {
            input: 60,
            output: 50,
            cost: 0.001,
        });
        // Token budget exhausted, cost is fine
        assert!(budget.exhausted());
        assert!(budget.cost_used < 1.0);
    }

    #[test]
    fn test_token_budget_exhausted_by_cost_only() {
        let mut budget = TokenBudget::with_cost_limit(10_000, Some(0.005));
        budget.consume(TokenCount {
            input: 50,
            output: 50,
            cost: 0.006, // Cost exhausted, tokens are fine
        });
        assert!(budget.exhausted());
        assert!(budget.used < 10_000);
    }
}
