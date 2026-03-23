// orchestration/ — Non-hierarchical multi-agent orchestration patterns.
//
// This module replaces the earlier hub-and-spoke `coordinator.rs` and
// sequential `pipeline.rs` with two richer patterns: Blackboard (shared-state
// parallel reactive agents) and Ring (sequential token-passing with voting).
// Once migration is complete, coordinator.rs and pipeline.rs will be removed.

pub mod blackboard;
pub mod classifier;
pub mod llm_agents;
pub mod ring;
#[cfg(test)]
pub(crate) mod test_helpers;

use serde::{Deserialize, Serialize};

/// Custom serde helpers for `Arc<Vec<T>>` so we can drop the global `serde/rc`
/// feature flag.  Serializes/deserializes as a plain `Vec<T>`.
pub(crate) mod arc_vec_serde {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(
        data: &Arc<Vec<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        data.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Vec<T>>, D::Error> {
        Vec::<T>::deserialize(deserializer).map(Arc::new)
    }
}

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
}

/// Configuration for the selected orchestration pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "pattern", rename_all = "lowercase")]
pub enum PatternConfig {
    Direct { agent: String },
    Blackboard(BlackboardConfig),
    Ring(RingConfig),
}

/// Blackboard trigger configuration for reactive agent activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerConfig {
    /// Board entry kinds that must be present for agent to activate
    #[serde(default)]
    pub requires: Vec<String>,
    /// Board entry kinds that prevent agent from activating
    #[serde(default)]
    pub excludes: Vec<String>,
    /// Earliest round the agent can contribute
    #[serde(default)]
    pub min_round: u32,
    /// Latest round the agent can contribute
    pub max_round: Option<u32>,
    /// Agent priority (0-100, higher = earlier)
    #[serde(default = "default_trigger_priority")]
    pub priority: u8,
}

const fn default_trigger_priority() -> u8 {
    50
}

/// Ring-specific agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRingConfig {
    /// Role in the ring (initiator, specialist, challenger, synthesizer)
    pub role: String,
    /// Position in the ring order (0-indexed)
    pub position: Option<usize>,
    /// Vote weight multiplier (default 1.0)
    #[serde(default = "default_vote_weight")]
    pub vote_weight: f32,
}

fn default_vote_weight() -> f32 {
    1.0
}
