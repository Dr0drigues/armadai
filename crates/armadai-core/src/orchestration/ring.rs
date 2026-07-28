use std::time::Duration;

use serde::{Deserialize, Serialize};

// ── Data structures ──────────────────────────────────────────────

/// Type of contribution action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContributionAction {
    Propose,
    Enrich {
        target: usize,
    },
    Contest {
        target: usize,
        counter_argument: String,
    },
    Endorse {
        target: usize,
    },
    Synthesize,
    Pass {
        reason: String,
    },
}

// ── Configuration ────────────────────────────────────────────────

/// Configuration for a Ring orchestration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingConfig {
    #[serde(default = "default_max_laps")]
    pub max_laps: u32,
    #[serde(default = "default_ring_agent_timeout_secs")]
    pub agent_timeout_secs: u64,
    #[serde(default = "default_ring_consensus_threshold")]
    pub consensus_threshold: f32,
    #[serde(default = "default_majority_threshold")]
    pub majority_threshold: f32,
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f32,
    #[serde(default = "default_ring_token_budget")]
    pub token_budget: u64,
}

const fn default_max_laps() -> u32 {
    3
}
const fn default_ring_agent_timeout_secs() -> u64 {
    90
}
const fn default_ring_consensus_threshold() -> f32 {
    0.80
}
const fn default_majority_threshold() -> f32 {
    0.60
}
const fn default_similarity_threshold() -> f32 {
    0.85
}
const fn default_ring_token_budget() -> u64 {
    // Safety cap, not a tight limit: high enough that normal multi-lap rings
    // (verbose real agents, provider-side context overhead) reach the vote
    // phase, low enough to still catch a runaway. The old 40k halted a real
    // 2-agent ring before voting. Tunable via `ring.token_budget`.
    500_000
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            max_laps: default_max_laps(),
            agent_timeout_secs: default_ring_agent_timeout_secs(),
            consensus_threshold: default_ring_consensus_threshold(),
            majority_threshold: default_majority_threshold(),
            similarity_threshold: default_similarity_threshold(),
            token_budget: default_ring_token_budget(),
        }
    }
}

impl RingConfig {
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
        if !(0.0..=1.0).contains(&self.majority_threshold) {
            anyhow::bail!(
                "majority_threshold must be in 0.0..=1.0, got {}",
                self.majority_threshold
            );
        }
        if !(0.0..=1.0).contains(&self.similarity_threshold) {
            anyhow::bail!(
                "similarity_threshold must be in 0.0..=1.0, got {}",
                self.similarity_threshold
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
    fn test_ring_config_defaults() {
        let config = RingConfig::default();
        assert_eq!(config.max_laps, 3);
        assert_eq!(config.agent_timeout_secs, 90);
        assert!((config.consensus_threshold - 0.80).abs() < f32::EPSILON);
        assert!((config.majority_threshold - 0.60).abs() < f32::EPSILON);
        assert!((config.similarity_threshold - 0.85).abs() < f32::EPSILON);
        assert_eq!(config.token_budget, 500_000);
        assert_eq!(config.agent_timeout(), Duration::from_secs(90));
    }

    #[test]
    fn test_ring_config_validate_ok() {
        let config = RingConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_ring_config_validate_bad_consensus() {
        let config = RingConfig {
            consensus_threshold: 1.5,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_ring_config_validate_bad_majority() {
        let config = RingConfig {
            majority_threshold: -0.1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_contribution_action_variants() {
        let _propose = ContributionAction::Propose;
        let _enrich = ContributionAction::Enrich { target: 0 };
        let _contest = ContributionAction::Contest {
            target: 1,
            counter_argument: "no".to_string(),
        };
        let _endorse = ContributionAction::Endorse { target: 2 };
        let _synth = ContributionAction::Synthesize;
        let _pass = ContributionAction::Pass {
            reason: "nothing to add".to_string(),
        };
    }
}
