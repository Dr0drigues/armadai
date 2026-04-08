//! Unified orchestration module.
//!
//! Supports four patterns:
//! - **Direct**: single-shot agent execution (default)
//! - **Blackboard**: parallel shared-state (PR #91)
//! - **Ring**: sequential token-passing with consensus (PR #91)
//! - **Hierarchical**: pyramid topology with coordinator → leads → agents
//!
//! The `Auto` variant uses a classifier to pick the best pattern.

pub mod blackboard;
pub mod classifier;
pub mod context_injection;
#[cfg(test)]
mod e2e_tests;
pub mod hierarchical;
pub mod llm_agents;
pub mod protocol;
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
    /// Hierarchical pyramid: coordinator → leads → agents.
    Hierarchical,
    /// Auto-detect the best pattern from task + config.
    Auto,
}

impl std::fmt::Display for OrchestrationPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => write!(f, "direct"),
            Self::Blackboard => write!(f, "blackboard"),
            Self::Ring => write!(f, "ring"),
            Self::Hierarchical => write!(f, "hierarchical"),
            Self::Auto => write!(f, "auto"),
        }
    }
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

// ── Hierarchical orchestration config ────────────────────────────

/// Unified orchestration configuration (top-level `orchestration:` key in armadai.yaml).
///
/// Combines PR #91 Blackboard/Ring parameters with Hierarchical topology.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct OrchestrationConfig {
    /// Whether orchestration mode is active.
    pub enabled: bool,

    /// Which pattern to use.
    pub pattern: OrchestrationPattern,

    // ── Hierarchical-specific ──────────────────────────────────
    /// Name of the coordinator agent (hierarchical only).
    pub coordinator: Option<String>,

    /// Team topology (hierarchical only).
    pub teams: Vec<TeamConfig>,

    // ── Shared limits (all patterns) ───────────────────────────
    /// Max delegation depth (default: 5).
    pub max_depth: Option<u32>,

    /// Max total iterations across all agents (default: 50).
    pub max_iterations: Option<u32>,

    /// Global timeout in seconds (default: 300).
    pub timeout: Option<u64>,

    // ── PR #91 parameters (blackboard/ring) ────────────────────
    /// Blackboard: max rounds (default: 5).
    pub max_rounds: Option<u32>,

    /// Ring: max token laps (default: 3).
    pub max_laps: Option<u32>,

    /// Consensus threshold (default: 0.75).
    pub consensus_threshold: Option<f32>,

    /// Global token budget (default: 100_000).
    pub token_budget: Option<u64>,

    /// Global cost budget in USD (no default, optional enforcement).
    pub cost_limit: Option<f64>,
}

impl OrchestrationConfig {
    pub fn max_depth(&self) -> u32 {
        self.max_depth.unwrap_or(5)
    }

    pub fn max_iterations(&self) -> u32 {
        self.max_iterations.unwrap_or(50)
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout.unwrap_or(300)
    }
}

/// A team definition within the hierarchical topology.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamConfig {
    /// Optional sub-lead for this team. If absent, agents report
    /// directly to the coordinator.
    pub lead: Option<String>,

    /// Agents in this team.
    #[serde(default)]
    pub agents: Vec<String>,
}

// ── Validation ───────────────────────────────────────────────────

/// Errors that can occur during orchestration config validation.
#[derive(Debug, thiserror::Error)]
pub enum OrchestrationValidationError {
    #[error("hierarchical pattern requires a `coordinator` field")]
    MissingCoordinator,

    #[error("agent '{0}' referenced in teams but not resolvable")]
    UnresolvableAgent(String),

    #[error("agent '{0}' appears in multiple teams")]
    DuplicateAgent(String),

    #[error("coordinator '{0}' must not appear inside a team")]
    CoordinatorInTeam(String),

    #[error("lead '{0}' appears in multiple teams")]
    DuplicateLead(String),

    #[error("teams are empty — hierarchical pattern requires at least one team")]
    EmptyTeams,
}

/// Validate the orchestration configuration for internal consistency.
///
/// This checks structural invariants only (no filesystem resolution).
/// Agent existence is verified separately during CLI execution.
pub fn validate_config(
    config: &OrchestrationConfig,
) -> Result<(), Vec<OrchestrationValidationError>> {
    if !config.enabled {
        return Ok(());
    }

    // Only validate hierarchical-specific fields for that pattern
    if config.pattern != OrchestrationPattern::Hierarchical {
        return Ok(());
    }

    let mut errors = Vec::new();

    // Coordinator is required
    let coordinator = match &config.coordinator {
        Some(c) => c.clone(),
        None => {
            errors.push(OrchestrationValidationError::MissingCoordinator);
            return Err(errors);
        }
    };

    // Must have at least one team
    if config.teams.is_empty() {
        errors.push(OrchestrationValidationError::EmptyTeams);
    }

    // Check for duplicate agents across teams
    let mut seen_agents = std::collections::HashSet::new();
    let mut seen_leads = std::collections::HashSet::new();

    for team in &config.teams {
        // Check lead uniqueness
        if let Some(ref lead) = team.lead {
            if !seen_leads.insert(lead.clone()) {
                errors.push(OrchestrationValidationError::DuplicateLead(lead.clone()));
            }
            // Lead must not be the coordinator
            if lead == &coordinator {
                errors.push(OrchestrationValidationError::CoordinatorInTeam(
                    coordinator.clone(),
                ));
            }
            // Lead must not be in another team's agents
            if !seen_agents.insert(lead.clone()) {
                errors.push(OrchestrationValidationError::DuplicateAgent(lead.clone()));
            }
        }

        for agent in &team.agents {
            // Agent must not be the coordinator
            if agent == &coordinator {
                errors.push(OrchestrationValidationError::CoordinatorInTeam(
                    coordinator.clone(),
                ));
            }
            // Agent must not appear twice
            if !seen_agents.insert(agent.clone()) {
                errors.push(OrchestrationValidationError::DuplicateAgent(agent.clone()));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ── Topology helpers ─────────────────────────────────────────────

/// Relationship between two agents in the hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRelationship {
    /// sender is the lead/coordinator of target
    Superior,
    /// sender and target are in the same team
    Peer,
    /// target is the lead/coordinator of sender
    Subordinate,
    /// no direct relationship
    Unknown,
}

/// Determine the relationship between two agents given the config.
pub fn classify_relationship(
    config: &OrchestrationConfig,
    sender: &str,
    target: &str,
) -> AgentRelationship {
    let coordinator = config.coordinator.as_deref().unwrap_or("");

    // Coordinator → anyone = Superior
    if sender == coordinator {
        return AgentRelationship::Superior;
    }

    // Anyone → coordinator = Subordinate
    if target == coordinator {
        return AgentRelationship::Subordinate;
    }

    for team in &config.teams {
        let lead = team.lead.as_deref();
        let agents = &team.agents;

        // Lead → agent in same team = Superior
        if lead == Some(sender) && agents.iter().any(|a| a == target) {
            return AgentRelationship::Superior;
        }

        // Agent → lead of same team = Subordinate
        if lead == Some(target) && agents.iter().any(|a| a == sender) {
            return AgentRelationship::Subordinate;
        }

        // Both in same team's agents = Peer
        let sender_in_team = agents.iter().any(|a| a == sender) || lead == Some(sender);
        let target_in_team = agents.iter().any(|a| a == target) || lead == Some(target);
        if sender_in_team && target_in_team {
            return AgentRelationship::Peer;
        }
    }

    AgentRelationship::Unknown
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod hierarchical_tests {
    use super::*;

    fn sample_config() -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("java-lead".to_string()),
                    agents: vec![
                        "java-arch".to_string(),
                        "java-sec".to_string(),
                        "java-test".to_string(),
                    ],
                },
                TeamConfig {
                    lead: Some("node-lead".to_string()),
                    agents: vec!["node-arch".to_string(), "node-gql".to_string()],
                },
                TeamConfig {
                    lead: None,
                    agents: vec!["cloud-expert".to_string(), "ops-expert".to_string()],
                },
            ],
            ..Default::default()
        }
    }

    // ── Validation tests ──

    #[test]
    fn test_valid_config() {
        let config = sample_config();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_disabled_config_always_valid() {
        let config = OrchestrationConfig {
            enabled: false,
            pattern: OrchestrationPattern::Hierarchical,
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_non_hierarchical_pattern_skips_validation() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Blackboard,
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_missing_coordinator() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: None,
            ..Default::default()
        };
        let errors = validate_config(&config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OrchestrationValidationError::MissingCoordinator))
        );
    }

    #[test]
    fn test_empty_teams() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![],
            ..Default::default()
        };
        let errors = validate_config(&config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OrchestrationValidationError::EmptyTeams))
        );
    }

    #[test]
    fn test_duplicate_agent() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![
                TeamConfig {
                    lead: None,
                    agents: vec!["agent-a".to_string()],
                },
                TeamConfig {
                    lead: None,
                    agents: vec!["agent-a".to_string()],
                },
            ],
            ..Default::default()
        };
        let errors = validate_config(&config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OrchestrationValidationError::DuplicateAgent(_)))
        );
    }

    #[test]
    fn test_coordinator_in_team() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["coord".to_string()],
            }],
            ..Default::default()
        };
        let errors = validate_config(&config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OrchestrationValidationError::CoordinatorInTeam(_)))
        );
    }

    #[test]
    fn test_duplicate_lead() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![
                TeamConfig {
                    lead: Some("lead-a".to_string()),
                    agents: vec!["agent-1".to_string()],
                },
                TeamConfig {
                    lead: Some("lead-a".to_string()),
                    agents: vec!["agent-2".to_string()],
                },
            ],
            ..Default::default()
        };
        let errors = validate_config(&config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, OrchestrationValidationError::DuplicateLead(_)))
        );
    }

    // ── Deserialization tests ──

    #[test]
    fn test_deserialize_full_config() {
        let yaml = r#"
enabled: true
pattern: hierarchical
coordinator: boulanger-architect
teams:
  - lead: java-lead
    agents:
      - java-arch
      - java-sec
  - agents:
      - cloud-expert
      - ops-expert
max_depth: 3
max_iterations: 100
timeout: 600
max_rounds: 10
consensus_threshold: 0.85
"#;
        let config: OrchestrationConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.pattern, OrchestrationPattern::Hierarchical);
        assert_eq!(config.coordinator.as_deref(), Some("boulanger-architect"));
        assert_eq!(config.teams.len(), 2);
        assert_eq!(config.teams[0].lead.as_deref(), Some("java-lead"));
        assert_eq!(config.teams[0].agents.len(), 2);
        assert!(config.teams[1].lead.is_none());
        assert_eq!(config.max_depth(), 3);
        assert_eq!(config.max_iterations(), 100);
        assert_eq!(config.timeout_secs(), 600);
        assert_eq!(config.max_rounds, Some(10));
        assert_eq!(config.consensus_threshold, Some(0.85));
    }

    #[test]
    fn test_deserialize_minimal_config() {
        let yaml = "enabled: true\npattern: blackboard\n";
        let config: OrchestrationConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.pattern, OrchestrationPattern::Blackboard);
        assert!(config.coordinator.is_none());
        assert!(config.teams.is_empty());
        assert_eq!(config.max_depth(), 5);
        assert_eq!(config.max_iterations(), 50);
        assert_eq!(config.timeout_secs(), 300);
    }

    #[test]
    fn test_default_pattern_is_direct() {
        let config = OrchestrationConfig::default();
        assert_eq!(config.pattern, OrchestrationPattern::Direct);
        assert!(!config.enabled);
    }

    // ── Relationship tests ──

    #[test]
    fn test_coordinator_to_anyone_is_superior() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "coordinator", "java-lead"),
            AgentRelationship::Superior
        );
    }

    #[test]
    fn test_anyone_to_coordinator_is_subordinate() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "java-lead", "coordinator"),
            AgentRelationship::Subordinate
        );
    }

    #[test]
    fn test_lead_to_agent_is_superior() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "java-lead", "java-arch"),
            AgentRelationship::Superior
        );
    }

    #[test]
    fn test_agent_to_lead_is_subordinate() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "java-arch", "java-lead"),
            AgentRelationship::Subordinate
        );
    }

    #[test]
    fn test_same_team_agents_are_peers() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "java-arch", "java-sec"),
            AgentRelationship::Peer
        );
    }

    #[test]
    fn test_different_team_agents_are_unknown() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "java-arch", "node-arch"),
            AgentRelationship::Unknown
        );
    }

    #[test]
    fn test_leadless_team_agents_are_peers() {
        let config = sample_config();
        assert_eq!(
            classify_relationship(&config, "cloud-expert", "ops-expert"),
            AgentRelationship::Peer
        );
    }

    #[test]
    fn test_pattern_display() {
        assert_eq!(OrchestrationPattern::Direct.to_string(), "direct");
        assert_eq!(
            OrchestrationPattern::Hierarchical.to_string(),
            "hierarchical"
        );
        assert_eq!(OrchestrationPattern::Auto.to_string(), "auto");
    }
}
