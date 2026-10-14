//! Orchestration declaration types: the `orchestration:` section of
//! `armadai.yaml` and the per-agent orchestration metadata, their validation,
//! and the Claude Code policy gate (`policy`). No execution engine lives here.

pub mod policy;

use serde::{Deserialize, Serialize};

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

/// Sub-pattern a hierarchical team can run instead of flat delegation (C9).
///
/// A dedicated enum (not `OrchestrationPattern`) makes deeper nesting
/// (hierarchical/direct/auto inside a team) impossible by construction:
/// only one level of nesting can ever exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NestedPattern {
    Blackboard,
    Ring,
}

impl std::fmt::Display for NestedPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blackboard => write!(f, "blackboard"),
            Self::Ring => write!(f, "ring"),
        }
    }
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

    /// C8: named agent routes (route name → agent names). Still parsed;
    /// no command selects a route since `run --route` was removed.
    #[serde(default)]
    pub routes: std::collections::BTreeMap<String, Vec<String>>,

    /// How strictly the declared topology is enforced by the Claude Code
    /// policy gate. Independent of `enabled`. Defaults to `off`.
    #[serde(default)]
    pub policy: policy::PolicyMode,

    /// Assistance agents reachable from anywhere, outside the topology
    /// (e.g. `Explore`, `Plan`). Under `policy: strict`, anything not
    /// declared here or in `teams` is refused.
    #[serde(default)]
    pub free_agents: Vec<String>,

    // ── Engine limits ──────────────────────────────────────────
    // The fields below were limits of the execution engine removed in
    // step 0. They are still parsed so existing configs keep loading, but
    // no command reads them.
    /// Max delegation depth.
    pub max_depth: Option<u32>,

    /// Max total iterations across all agents.
    pub max_iterations: Option<u32>,

    /// Max number of delegations executed concurrently.
    #[serde(default)]
    pub max_concurrency: Option<u32>,

    /// Global timeout in seconds.
    pub timeout: Option<u64>,

    // ── PR #91 parameters (blackboard/ring) ────────────────────
    /// Blackboard: max rounds.
    pub max_rounds: Option<u32>,

    /// Ring: max token laps.
    pub max_laps: Option<u32>,

    /// Consensus threshold.
    pub consensus_threshold: Option<f32>,

    /// Global token budget.
    pub token_budget: Option<u64>,

    /// Global cost budget in USD (no default, optional enforcement).
    pub cost_limit: Option<f64>,
}

/// A team definition within the hierarchical topology.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TeamConfig {
    /// Optional sub-lead for this team. If absent, agents report
    /// directly to the coordinator.
    pub lead: Option<String>,

    /// Agents in this team.
    #[serde(default)]
    pub agents: Vec<String>,

    /// C9: if set, the team runs as this sub-pattern (blackboard/ring)
    /// instead of flat delegation. Requires a `lead` (the arbiter).
    #[serde(default)]
    pub pattern: Option<NestedPattern>,

    /// C9: per-team override for blackboard `max_rounds` (else global/default).
    #[serde(default)]
    pub max_rounds: Option<u32>,

    /// C9: per-team override for ring `max_laps` (else global/default).
    #[serde(default)]
    pub max_laps: Option<u32>,

    /// C9: per-team override for the sub-pattern consensus threshold.
    #[serde(default)]
    pub consensus_threshold: Option<f32>,
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod hierarchical_tests {
    use super::*;

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
        let config: OrchestrationConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.pattern, OrchestrationPattern::Hierarchical);
        assert_eq!(config.coordinator.as_deref(), Some("boulanger-architect"));
        assert_eq!(config.teams.len(), 2);
        assert_eq!(config.teams[0].lead.as_deref(), Some("java-lead"));
        assert_eq!(config.teams[0].agents.len(), 2);
        assert!(config.teams[1].lead.is_none());
        assert_eq!(config.max_depth, Some(3));
        assert_eq!(config.max_iterations, Some(100));
        assert_eq!(config.timeout, Some(600));
        assert_eq!(config.max_rounds, Some(10));
        assert_eq!(config.consensus_threshold, Some(0.85));
    }

    #[test]
    fn test_deserialize_minimal_config() {
        let yaml = "enabled: true\npattern: blackboard\n";
        let config: OrchestrationConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert!(config.enabled);
        assert_eq!(config.pattern, OrchestrationPattern::Blackboard);
        assert!(config.coordinator.is_none());
        assert!(config.teams.is_empty());
        assert!(config.max_depth.is_none());
    }

    #[test]
    fn test_orchestration_config_deserializes_routes() {
        let yaml = r#"
enabled: true
pattern: blackboard
routes:
  security-audit: [rust-security, rust-reviewer]
  frontend: [ui-specialist]
"#;
        let config: OrchestrationConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.routes.get("security-audit").unwrap(),
            &vec!["rust-security".to_string(), "rust-reviewer".to_string()]
        );
        assert_eq!(config.routes.get("frontend").unwrap().len(), 1);
    }

    #[test]
    fn test_orchestration_config_routes_default_empty() {
        let yaml = "enabled: true\npattern: blackboard\n";
        let config: OrchestrationConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert!(config.routes.is_empty());
    }

    #[test]
    fn test_default_pattern_is_direct() {
        let config = OrchestrationConfig::default();
        assert_eq!(config.pattern, OrchestrationPattern::Direct);
        assert!(!config.enabled);
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

    // ── C9: nested pattern tests ──

    #[test]
    fn test_team_config_deserializes_nested_pattern() {
        let yaml = r#"
enabled: true
pattern: hierarchical
coordinator: coord
teams:
  - lead: research-lead
    agents: [searcher, analyst]
    pattern: blackboard
    max_rounds: 4
  - lead: build-lead
    agents: [coder]
    pattern: ring
    max_laps: 2
    consensus_threshold: 0.8
  - lead: doc-lead
    agents: [writer]
"#;
        let config: OrchestrationConfig = armadai_model::yaml::from_str(yaml).unwrap();
        assert_eq!(config.teams[0].pattern, Some(NestedPattern::Blackboard));
        assert_eq!(config.teams[0].max_rounds, Some(4));
        assert_eq!(config.teams[1].pattern, Some(NestedPattern::Ring));
        assert_eq!(config.teams[1].max_laps, Some(2));
        assert_eq!(config.teams[1].consensus_threshold, Some(0.8));
        assert_eq!(config.teams[2].pattern, None); // flat delegation
    }
}
