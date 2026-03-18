use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::blackboard::TokenBudget;
use super::blackboard::TokenCount;
use crate::providers::traits::Provider;

// ── Data structures ──────────────────────────────────────────────

/// Artifact that circulates between agents in the ring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingToken {
    pub id: Uuid,
    pub task: String,
    pub contributions: Vec<Contribution>,
    pub lap: u32,
    pub(crate) votes: BTreeMap<String, Vote>,
    pub(crate) status: TokenStatus,
    pub ring_order: Vec<String>,
    pub current_position: usize,
    pub budget: TokenBudget,
    pub created_at: DateTime<Utc>,
}

/// Status of the ring token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TokenStatus {
    Circulating,
    Voting,
    Done { outcome: RingOutcome },
}

/// Final outcome of a ring execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RingOutcome {
    Consensus {
        resolution: String,
        score: f32,
    },
    Majority {
        resolution: String,
        score: f32,
        dissents: Vec<Dissent>,
    },
    NoConsensus {
        summary: String,
        positions: Vec<Position>,
    },
    BudgetExhausted {
        partial_summary: String,
    },
    Cancelled,
}

/// A single contribution in the ring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub agent: String,
    pub lap: u32,
    pub position_in_lap: usize,
    pub action: ContributionAction,
    pub content: String,
    pub reactions: Vec<Reaction>,
    pub tokens_used: TokenCount,
    pub created_at: DateTime<Utc>,
}

impl Contribution {
    /// Create a Pass contribution (used for errors/timeouts).
    pub fn pass(agent: &str, lap: u32, position: usize, reason: String) -> Self {
        Self {
            agent: agent.to_string(),
            lap,
            position_in_lap: position,
            action: ContributionAction::Pass {
                reason: reason.clone(),
            },
            content: reason,
            reactions: vec![],
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        }
    }
}

/// Type of contribution action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContributionAction {
    Propose,
    Enrich { target: usize },
    Contest { target: usize, counter_argument: String },
    Endorse { target: usize },
    Synthesize,
    Pass { reason: String },
}

/// Reaction to a previous contribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub target: usize,
    pub kind: ReactionKind,
    pub note: Option<String>,
}

/// Type of reaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReactionKind {
    Agree,
    Disagree,
    NeedsMoreDetail,
    OutOfScope,
}

/// An agent's vote during the voting phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub position: String,
    pub confidence: f32,
    pub supporting_contributions: Vec<usize>,
    pub unresolved_concerns: Vec<String>,
}

/// A dissenting position in the final outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dissent {
    pub agent: String,
    pub position: String,
    pub reason: String,
}

/// An agent's final stance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub agent: String,
    pub stance: String,
    pub confidence: f32,
}

/// Role of an agent in the ring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RingRole {
    Initiator,
    Specialist { domain: String },
    Challenger,
    Synthesizer,
}

// ── RingToken methods ────────────────────────────────────────────

impl RingToken {
    /// Create a new token for a ring execution.
    pub fn new(task: String, ring_order: Vec<String>, token_budget: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            task,
            contributions: Vec::new(),
            lap: 0,
            votes: BTreeMap::new(),
            status: TokenStatus::Circulating,
            ring_order,
            current_position: 0,
            budget: TokenBudget::new(token_budget),
            created_at: Utc::now(),
        }
    }

    /// Accessor for the token status.
    pub fn status(&self) -> &TokenStatus {
        &self.status
    }

    /// Accessor for the votes.
    pub fn votes(&self) -> &BTreeMap<String, Vote> {
        &self.votes
    }

    /// Produce a snapshot for agents.
    pub fn snapshot(&self) -> TokenSnapshot {
        TokenSnapshot {
            task: self.task.clone(),
            contributions: self.contributions.clone(),
            lap: self.lap,
            status: self.status.clone(),
            ring_order: self.ring_order.clone(),
            current_position: self.current_position,
            budget_remaining: self.budget.remaining(),
        }
    }
}

/// Immutable view of the token passed to agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSnapshot {
    pub task: String,
    pub contributions: Vec<Contribution>,
    pub lap: u32,
    pub status: TokenStatus,
    pub ring_order: Vec<String>,
    pub current_position: usize,
    pub budget_remaining: u64,
}

// ── Agent trait ───────────────────────────────────────────────────

/// Trait for agents that participate in a Ring pattern.
#[async_trait]
pub trait RingAgent: Send + Sync {
    /// Unique agent identifier.
    fn name(&self) -> &str;

    /// Role in the ring.
    fn role(&self) -> RingRole;

    /// Process the token and produce a contribution.
    async fn process(
        &self,
        token: &TokenSnapshot,
        provider: &dyn Provider,
    ) -> anyhow::Result<Contribution>;

    /// Vote phase: agent takes a final position.
    async fn vote(
        &self,
        token: &TokenSnapshot,
        provider: &dyn Provider,
    ) -> anyhow::Result<Vote>;
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
    40_000
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

// ── Execution loop ───────────────────────────────────────────────

/// Summarize contributions so far (for budget exhaustion outcome).
fn summarize_so_far(token: &RingToken) -> String {
    let mut summary = String::with_capacity(256);
    summary.push_str("Task: ");
    summary.push_str(&token.task);
    summary.push_str("\n\n");
    for c in &token.contributions {
        summary.push_str(&format!(
            "[Lap {} / {}] {}: {}\n",
            c.lap, c.position_in_lap, c.agent, c.content
        ));
    }
    summary
}

/// Group votes by position similarity and resolve the outcome.
fn resolve_votes(token: &RingToken, config: &RingConfig) -> RingOutcome {
    if token.votes.is_empty() {
        return RingOutcome::NoConsensus {
            summary: "No votes cast".into(),
            positions: vec![],
        };
    }

    // Group by lowercased position string (deterministic iteration via BTreeMap).
    let mut groups: BTreeMap<String, Vec<(String, &Vote)>> = BTreeMap::new();
    for (agent, vote) in &token.votes {
        let key = vote.position.to_lowercase();
        groups
            .entry(key)
            .or_default()
            .push((agent.clone(), vote));
    }

    let total_voters = token.votes.len() as f32;
    let largest_group = groups.values().max_by_key(|g| g.len()).unwrap();
    let majority_ratio = largest_group.len() as f32 / total_voters;
    let representative = largest_group[0].1.position.clone();
    let largest_members: Vec<String> = largest_group.iter().map(|(n, _)| n.clone()).collect();

    if majority_ratio >= config.consensus_threshold {
        RingOutcome::Consensus {
            resolution: representative,
            score: majority_ratio,
        }
    } else if majority_ratio >= config.majority_threshold {
        let dissents = token
            .votes
            .iter()
            .filter(|(name, _)| !largest_members.contains(name))
            .map(|(name, vote)| Dissent {
                agent: name.clone(),
                position: vote.position.clone(),
                reason: vote.unresolved_concerns.join("; "),
            })
            .collect();

        RingOutcome::Majority {
            resolution: representative,
            score: majority_ratio,
            dissents,
        }
    } else {
        RingOutcome::NoConsensus {
            summary: format!("{} distinct positions, no majority", groups.len()),
            positions: token
                .votes
                .iter()
                .map(|(name, vote)| Position {
                    agent: name.clone(),
                    stance: vote.position.clone(),
                    confidence: vote.confidence,
                })
                .collect(),
        }
    }
}

/// Run the ring execution loop with 3 phases: circulation, voting, resolution.
pub async fn run_ring(
    token: &mut RingToken,
    agents: &[Arc<dyn RingAgent>],
    providers: &[Arc<dyn Provider>],
    config: &RingConfig,
) -> anyhow::Result<()> {
    config.validate()?;

    if providers.is_empty() {
        anyhow::bail!("At least one provider is required for ring execution");
    }

    // Phase 1: Circulation
    while token.lap < config.max_laps && !matches!(token.status, TokenStatus::Done { .. }) {
        let mut any_substantive = false;

        for (pos, agent) in agents.iter().enumerate() {
            token.current_position = pos;
            let snapshot = token.snapshot();
            let provider_idx = pos % providers.len();
            let provider = providers[provider_idx].as_ref();
            let timeout_dur = config.agent_timeout();

            let contribution =
                tokio::time::timeout(timeout_dur, agent.process(&snapshot, provider)).await;

            match contribution {
                Ok(Ok(mut contrib)) => {
                    contrib.lap = token.lap;
                    contrib.position_in_lap = pos;
                    if !matches!(contrib.action, ContributionAction::Pass { .. }) {
                        any_substantive = true;
                    }
                    token.budget.consume(contrib.tokens_used);
                    token.contributions.push(contrib);
                }
                Ok(Err(e)) => {
                    tracing::warn!(agent = %agent.name(), error = %e, "agent error");
                    token.contributions.push(Contribution::pass(
                        agent.name(),
                        token.lap,
                        pos,
                        format!("Error: {e}"),
                    ));
                }
                Err(_) => {
                    tracing::warn!(agent = %agent.name(), "agent timeout");
                    token.contributions.push(Contribution::pass(
                        agent.name(),
                        token.lap,
                        pos,
                        "Timeout".into(),
                    ));
                }
            }

            // Check budget mid-ring
            if token.budget.exhausted() {
                token.status = TokenStatus::Done {
                    outcome: RingOutcome::BudgetExhausted {
                        partial_summary: summarize_so_far(token),
                    },
                };
                return Ok(());
            }
        }

        // If everyone passed → early convergence
        if !any_substantive {
            break;
        }

        token.lap += 1;
    }

    // Phase 2: Voting
    token.status = TokenStatus::Voting;

    for (pos, agent) in agents.iter().enumerate() {
        let snapshot = token.snapshot();
        let provider_idx = pos % providers.len();
        let provider = providers[provider_idx].as_ref();

        match agent.vote(&snapshot, provider).await {
            Ok(vote) => {
                token.votes.insert(agent.name().to_string(), vote);
            }
            Err(e) => {
                tracing::warn!(agent = %agent.name(), "vote failed: {e}");
            }
        }
    }

    // Phase 3: Resolution
    token.status = TokenStatus::Done {
        outcome: resolve_votes(token, config),
    };

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_token_new() {
        let token = RingToken::new(
            "test task".to_string(),
            vec!["a".to_string(), "b".to_string()],
            40_000,
        );
        assert_eq!(token.task, "test task");
        assert_eq!(token.lap, 0);
        assert_eq!(*token.status(), TokenStatus::Circulating);
        assert_eq!(token.ring_order, vec!["a", "b"]);
        assert!(token.contributions.is_empty());
        assert!(token.votes().is_empty());
    }

    #[test]
    fn test_ring_token_snapshot() {
        let token = RingToken::new("task".to_string(), vec!["a".to_string()], 10_000);
        let snap = token.snapshot();
        assert_eq!(snap.task, "task");
        assert_eq!(snap.lap, 0);
        assert_eq!(snap.budget_remaining, 10_000);
    }

    #[test]
    fn test_ring_token_accessors() {
        let token = RingToken::new("task".to_string(), vec![], 10_000);
        assert_eq!(*token.status(), TokenStatus::Circulating);
        assert!(token.votes().is_empty());
    }

    #[test]
    fn test_contribution_pass() {
        let c = Contribution::pass("agent-a", 1, 2, "timeout".to_string());
        assert_eq!(c.agent, "agent-a");
        assert_eq!(c.lap, 1);
        assert_eq!(c.position_in_lap, 2);
        assert!(matches!(c.action, ContributionAction::Pass { .. }));
    }

    #[test]
    fn test_ring_config_defaults() {
        let config = RingConfig::default();
        assert_eq!(config.max_laps, 3);
        assert_eq!(config.agent_timeout_secs, 90);
        assert!((config.consensus_threshold - 0.80).abs() < f32::EPSILON);
        assert!((config.majority_threshold - 0.60).abs() < f32::EPSILON);
        assert!((config.similarity_threshold - 0.85).abs() < f32::EPSILON);
        assert_eq!(config.token_budget, 40_000);
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
    fn test_resolve_votes_empty() {
        let token = RingToken::new("task".to_string(), vec![], 10_000);
        let config = RingConfig::default();
        let outcome = resolve_votes(&token, &config);
        assert!(matches!(outcome, RingOutcome::NoConsensus { .. }));
    }

    #[test]
    fn test_resolve_votes_consensus() {
        let mut token = RingToken::new("task".to_string(), vec![], 10_000);
        // 5 agents all vote the same
        for i in 0..5 {
            token.votes.insert(
                format!("agent-{i}"),
                Vote {
                    position: "Rust/Axum".to_string(),
                    confidence: 0.9,
                    supporting_contributions: vec![],
                    unresolved_concerns: vec![],
                },
            );
        }
        let config = RingConfig::default();
        let outcome = resolve_votes(&token, &config);
        match outcome {
            RingOutcome::Consensus { score, .. } => {
                assert!((score - 1.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected Consensus, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_votes_majority() {
        let mut token = RingToken::new("task".to_string(), vec![], 10_000);
        // 3 out of 4 vote the same → 0.75, which is >= majority (0.60) but < consensus (0.80)
        for i in 0..3 {
            token.votes.insert(
                format!("agent-{i}"),
                Vote {
                    position: "Option A".to_string(),
                    confidence: 0.8,
                    supporting_contributions: vec![],
                    unresolved_concerns: vec![],
                },
            );
        }
        token.votes.insert(
            "agent-3".to_string(),
            Vote {
                position: "Option B".to_string(),
                confidence: 0.7,
                supporting_contributions: vec![],
                unresolved_concerns: vec!["concern".to_string()],
            },
        );

        let config = RingConfig::default();
        let outcome = resolve_votes(&token, &config);
        match outcome {
            RingOutcome::Majority { dissents, .. } => {
                assert_eq!(dissents.len(), 1);
                assert_eq!(dissents[0].agent, "agent-3");
            }
            other => panic!("Expected Majority, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_votes_no_consensus() {
        let mut token = RingToken::new("task".to_string(), vec![], 10_000);
        // Each agent votes differently → no majority
        for i in 0..5 {
            token.votes.insert(
                format!("agent-{i}"),
                Vote {
                    position: format!("Option {i}"),
                    confidence: 0.5,
                    supporting_contributions: vec![],
                    unresolved_concerns: vec![],
                },
            );
        }
        let config = RingConfig::default();
        let outcome = resolve_votes(&token, &config);
        match outcome {
            RingOutcome::NoConsensus { positions, .. } => {
                assert_eq!(positions.len(), 5);
            }
            other => panic!("Expected NoConsensus, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_votes_deterministic_btreemap() {
        // With BTreeMap, iteration order is deterministic
        let mut token = RingToken::new("task".to_string(), vec![], 10_000);
        for i in (0..5).rev() {
            token.votes.insert(
                format!("agent-{i}"),
                Vote {
                    position: "same".to_string(),
                    confidence: 0.9,
                    supporting_contributions: vec![],
                    unresolved_concerns: vec![],
                },
            );
        }
        let config = RingConfig::default();
        let outcome = resolve_votes(&token, &config);
        // Should deterministically resolve to consensus
        assert!(matches!(outcome, RingOutcome::Consensus { .. }));
    }

    #[test]
    fn test_ring_role_variants() {
        let _init = RingRole::Initiator;
        let _spec = RingRole::Specialist {
            domain: "security".to_string(),
        };
        let _chal = RingRole::Challenger;
        let _synth = RingRole::Synthesizer;
    }

    #[test]
    fn test_token_status_variants() {
        assert_eq!(TokenStatus::Circulating, TokenStatus::Circulating);
        assert_eq!(TokenStatus::Voting, TokenStatus::Voting);
        assert_ne!(TokenStatus::Circulating, TokenStatus::Voting);
    }

    #[test]
    fn test_summarize_so_far() {
        let mut token = RingToken::new("review code".to_string(), vec!["a".to_string()], 10_000);
        token.contributions.push(Contribution {
            agent: "agent-a".to_string(),
            lap: 0,
            position_in_lap: 0,
            action: ContributionAction::Propose,
            content: "Use Rust".to_string(),
            reactions: vec![],
            tokens_used: TokenCount::default(),
            created_at: Utc::now(),
        });
        let summary = summarize_so_far(&token);
        assert!(summary.contains("review code"));
        assert!(summary.contains("agent-a"));
        assert!(summary.contains("Use Rust"));
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

    #[test]
    fn test_reaction_kind_variants() {
        let _agree = ReactionKind::Agree;
        let _disagree = ReactionKind::Disagree;
        let _detail = ReactionKind::NeedsMoreDetail;
        let _scope = ReactionKind::OutOfScope;
    }

    #[test]
    fn test_ring_outcome_variants() {
        let _consensus = RingOutcome::Consensus {
            resolution: "yes".to_string(),
            score: 1.0,
        };
        let _majority = RingOutcome::Majority {
            resolution: "yes".to_string(),
            score: 0.75,
            dissents: vec![],
        };
        let _no = RingOutcome::NoConsensus {
            summary: "split".to_string(),
            positions: vec![],
        };
        let _budget = RingOutcome::BudgetExhausted {
            partial_summary: "partial".to_string(),
        };
        let _cancelled = RingOutcome::Cancelled;
    }

    #[tokio::test]
    async fn test_run_ring_no_providers() {
        let mut token = RingToken::new("task".to_string(), vec![], 10_000);
        let agents: Vec<Arc<dyn RingAgent>> = vec![];
        let providers: Vec<Arc<dyn Provider>> = vec![];
        let config = RingConfig::default();
        let result = run_ring(&mut token, &agents, &providers, &config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_run_ring_no_agents_votes_empty() {
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

        let mut token = RingToken::new("task".to_string(), vec![], 10_000);
        let agents: Vec<Arc<dyn RingAgent>> = vec![];
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(DummyProvider)];
        let config = RingConfig::default();
        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // With no agents, should go straight to voting then resolution with NoConsensus
        match token.status() {
            TokenStatus::Done { outcome } => {
                assert!(matches!(outcome, RingOutcome::NoConsensus { .. }));
            }
            other => panic!("Expected Done, got {other:?}"),
        }
    }

    // ── Integration tests with mock agents ────────────────────────

    use crate::providers::traits::*;

    struct MockProvider;
    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(&self, _: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: "ok".to_string(),
                model: "mock".to_string(),
                tokens_in: 10,
                tokens_out: 10,
                cost: 0.0,
            })
        }
        async fn stream(&self, _: CompletionRequest) -> anyhow::Result<TokenStream> {
            unimplemented!()
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "mock".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// Mock ring agent that Proposes on lap 0, Enriches on lap 1, then Endorses.
    struct ProposeEnrichEndorseAgent {
        id: String,
    }

    #[async_trait]
    impl RingAgent for ProposeEnrichEndorseAgent {
        fn name(&self) -> &str {
            &self.id
        }
        fn role(&self) -> RingRole {
            RingRole::Initiator
        }
        async fn process(
            &self,
            token: &TokenSnapshot,
            _provider: &dyn Provider,
        ) -> anyhow::Result<Contribution> {
            let action = match token.lap {
                0 => ContributionAction::Propose,
                1 if !token.contributions.is_empty() => ContributionAction::Enrich { target: 0 },
                _ if !token.contributions.is_empty() => ContributionAction::Endorse { target: 0 },
                _ => ContributionAction::Propose,
            };
            Ok(Contribution {
                agent: self.id.clone(),
                lap: token.lap,
                position_in_lap: token.current_position,
                action,
                content: format!("{} contribution lap {}", self.id, token.lap),
                reactions: vec![],
                tokens_used: TokenCount {
                    input: 10,
                    output: 10,
                },
                created_at: Utc::now(),
            })
        }
        async fn vote(
            &self,
            _token: &TokenSnapshot,
            _provider: &dyn Provider,
        ) -> anyhow::Result<Vote> {
            Ok(Vote {
                position: "Use Rust".to_string(),
                confidence: 0.9,
                supporting_contributions: vec![0],
                unresolved_concerns: vec![],
            })
        }
    }

    #[tokio::test]
    async fn test_integration_ring_propose_enrich_endorse_consensus() {
        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut token = RingToken::new("design API".to_string(), order, 50_000);
        let agents: Vec<Arc<dyn RingAgent>> = vec![
            Arc::new(ProposeEnrichEndorseAgent { id: "a".into() }),
            Arc::new(ProposeEnrichEndorseAgent { id: "b".into() }),
            Arc::new(ProposeEnrichEndorseAgent { id: "c".into() }),
        ];
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider)];
        let config = RingConfig {
            max_laps: 2,
            ..Default::default()
        };

        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // All 3 agents vote "Use Rust" → consensus
        match token.status() {
            TokenStatus::Done {
                outcome: RingOutcome::Consensus { score, .. },
            } => {
                assert!((score - 1.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected Consensus, got {other:?}"),
        }
    }

    /// Mock agent that always passes.
    struct AlwaysPassAgent {
        id: String,
    }

    #[async_trait]
    impl RingAgent for AlwaysPassAgent {
        fn name(&self) -> &str {
            &self.id
        }
        fn role(&self) -> RingRole {
            RingRole::Specialist {
                domain: "none".into(),
            }
        }
        async fn process(
            &self,
            token: &TokenSnapshot,
            _provider: &dyn Provider,
        ) -> anyhow::Result<Contribution> {
            Ok(Contribution::pass(
                &self.id,
                token.lap,
                token.current_position,
                "nothing to add".into(),
            ))
        }
        async fn vote(
            &self,
            _token: &TokenSnapshot,
            _provider: &dyn Provider,
        ) -> anyhow::Result<Vote> {
            Ok(Vote {
                position: "No opinion".to_string(),
                confidence: 0.1,
                supporting_contributions: vec![],
                unresolved_concerns: vec![],
            })
        }
    }

    #[tokio::test]
    async fn test_integration_ring_all_pass_early_exit() {
        let mut token = RingToken::new("task".to_string(), vec!["a".into(), "b".into()], 50_000);
        let agents: Vec<Arc<dyn RingAgent>> = vec![
            Arc::new(AlwaysPassAgent { id: "a".into() }),
            Arc::new(AlwaysPassAgent { id: "b".into() }),
        ];
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider)];
        let config = RingConfig::default();

        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // All agents passed → early exit after lap 0, should still vote and resolve
        assert!(matches!(token.status(), TokenStatus::Done { .. }));
        // Only 1 lap's worth of contributions (lap 0, no lap 1)
        assert_eq!(token.lap, 0);
    }

    #[tokio::test]
    async fn test_integration_ring_max_laps_zero() {
        let mut token = RingToken::new("task".to_string(), vec!["a".into()], 50_000);
        let agents: Vec<Arc<dyn RingAgent>> = vec![Arc::new(ProposeEnrichEndorseAgent {
            id: "a".into(),
        })];
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider)];
        let config = RingConfig {
            max_laps: 0,
            ..Default::default()
        };

        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // max_laps=0 means no circulation, straight to voting
        assert!(matches!(token.status(), TokenStatus::Done { .. }));
        assert!(token.contributions.is_empty());
    }

    #[tokio::test]
    async fn test_integration_ring_token_budget_zero() {
        let mut token = RingToken::new("task".to_string(), vec!["a".into()], 0);
        let agents: Vec<Arc<dyn RingAgent>> = vec![Arc::new(ProposeEnrichEndorseAgent {
            id: "a".into(),
        })];
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider)];
        let config = RingConfig::default();

        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // Budget is 0, should exhaust immediately after first contribution
        match token.status() {
            TokenStatus::Done {
                outcome: RingOutcome::BudgetExhausted { .. },
            } => {}
            other => panic!("Expected BudgetExhausted, got {other:?}"),
        }
    }
}
