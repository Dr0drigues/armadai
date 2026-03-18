use std::collections::HashMap;
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
pub struct Token {
    pub id: Uuid,
    pub task: String,
    pub contributions: Vec<Contribution>,
    pub lap: u32,
    pub votes: HashMap<String, Vote>,
    pub status: TokenStatus,
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

// ── Token methods ────────────────────────────────────────────────

impl Token {
    /// Create a new token for a ring execution.
    pub fn new(task: String, ring_order: Vec<String>, token_budget: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            task,
            contributions: Vec::new(),
            lap: 0,
            votes: HashMap::new(),
            status: TokenStatus::Circulating,
            ring_order,
            current_position: 0,
            budget: TokenBudget::new(token_budget),
            created_at: Utc::now(),
        }
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

fn default_max_laps() -> u32 {
    3
}
fn default_ring_agent_timeout_secs() -> u64 {
    90
}
fn default_ring_consensus_threshold() -> f32 {
    0.80
}
fn default_majority_threshold() -> f32 {
    0.60
}
fn default_similarity_threshold() -> f32 {
    0.85
}
fn default_ring_token_budget() -> u64 {
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
}

// ── Execution loop ───────────────────────────────────────────────

/// Summarize contributions so far (for budget exhaustion outcome).
fn summarize_so_far(token: &Token) -> String {
    let mut summary = format!("Task: {}\n\n", token.task);
    for c in &token.contributions {
        summary.push_str(&format!(
            "[Lap {} / {}] {}: {}\n",
            c.lap, c.position_in_lap, c.agent, c.content
        ));
    }
    summary
}

/// Group votes by position similarity and resolve the outcome.
fn resolve_votes(token: &Token, config: &RingConfig) -> RingOutcome {
    if token.votes.is_empty() {
        return RingOutcome::NoConsensus {
            summary: "No votes cast".into(),
            positions: vec![],
        };
    }

    // Simple grouping: exact string match on position.
    // A full implementation would use semantic similarity, but for now we group by
    // lowercased position string.
    let mut groups: HashMap<String, Vec<(String, &Vote)>> = HashMap::new();
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
    token: &mut Token,
    agents: &[Box<dyn RingAgent>],
    providers: &[Box<dyn Provider>],
    config: &RingConfig,
) -> anyhow::Result<()> {
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
    fn test_token_new() {
        let token = Token::new(
            "test task".to_string(),
            vec!["a".to_string(), "b".to_string()],
            40_000,
        );
        assert_eq!(token.task, "test task");
        assert_eq!(token.lap, 0);
        assert_eq!(token.status, TokenStatus::Circulating);
        assert_eq!(token.ring_order, vec!["a", "b"]);
        assert!(token.contributions.is_empty());
        assert!(token.votes.is_empty());
    }

    #[test]
    fn test_token_snapshot() {
        let token = Token::new("task".to_string(), vec!["a".to_string()], 10_000);
        let snap = token.snapshot();
        assert_eq!(snap.task, "task");
        assert_eq!(snap.lap, 0);
        assert_eq!(snap.budget_remaining, 10_000);
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
    fn test_resolve_votes_empty() {
        let token = Token::new("task".to_string(), vec![], 10_000);
        let config = RingConfig::default();
        let outcome = resolve_votes(&token, &config);
        assert!(matches!(outcome, RingOutcome::NoConsensus { .. }));
    }

    #[test]
    fn test_resolve_votes_consensus() {
        let mut token = Token::new("task".to_string(), vec![], 10_000);
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
        let mut token = Token::new("task".to_string(), vec![], 10_000);
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
        let mut token = Token::new("task".to_string(), vec![], 10_000);
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
        let mut token = Token::new("review code".to_string(), vec!["a".to_string()], 10_000);
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
        let mut token = Token::new("task".to_string(), vec![], 10_000);
        let agents: Vec<Box<dyn RingAgent>> = vec![];
        let providers: Vec<Box<dyn Provider>> = vec![];
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

        let mut token = Token::new("task".to_string(), vec![], 10_000);
        let agents: Vec<Box<dyn RingAgent>> = vec![];
        let providers: Vec<Box<dyn Provider>> = vec![Box::new(DummyProvider)];
        let config = RingConfig::default();
        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // With no agents, should go straight to voting then resolution with NoConsensus
        match &token.status {
            TokenStatus::Done { outcome } => {
                assert!(matches!(outcome, RingOutcome::NoConsensus { .. }));
            }
            other => panic!("Expected Done, got {other:?}"),
        }
    }
}
