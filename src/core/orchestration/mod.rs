pub mod blackboard;
pub mod classifier;
pub mod ring;

use serde::{Deserialize, Serialize};

use self::blackboard::BlackboardConfig;
use self::ring::RingConfig;

/// Orchestration pattern for multi-agent execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OrchestrationPattern {
    /// Single agent, no orchestration.
    #[default]
    Direct,
    /// Shared-state blackboard with parallel reactive agents.
    Blackboard,
    /// Sequential token-passing ring with explicit voting.
    Ring,
    /// Agent can participate in both Blackboard and Ring.
    Both,
}

/// Configuration for the selected orchestration pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "pattern", rename_all = "lowercase")]
pub enum PatternConfig {
    Direct { agent: String },
    Blackboard(BlackboardConfig),
    Ring(RingConfig),
}
