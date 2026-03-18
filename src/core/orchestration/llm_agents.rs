use async_trait::async_trait;
use chrono::Utc;

use super::blackboard::{BoardAgent, BoardDelta, BoardEntry, BoardSnapshot, EntryKind, TokenCount};
use super::ring::{Contribution, ContributionAction, RingAgent, RingRole, TokenSnapshot, Vote};
use crate::core::agent::Agent;
use crate::providers::traits::{ChatMessage, CompletionRequest, Provider};

// ── Helpers ──────────────────────────────────────────────────────

/// Map an `EntryKind` variant to a lowercase name for trigger matching.
fn entry_kind_name(kind: &EntryKind) -> &str {
    match kind {
        EntryKind::Finding => "finding",
        EntryKind::Challenge { .. } => "challenge",
        EntryKind::Confirmation { .. } => "confirmation",
        EntryKind::Synthesis { .. } => "synthesis",
        EntryKind::Question => "question",
        EntryKind::Answer { .. } => "answer",
    }
}

/// Resolve the model string from agent metadata.
fn agent_model(agent: &Agent) -> String {
    agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string())
}

// ── LlmBoardAgent ────────────────────────────────────────────────

/// LLM-backed agent that participates in Blackboard orchestration.
///
/// Wraps an `Agent` definition and delegates to its configured provider.
/// The `can_contribute` check honours the agent's `TriggerConfig` (if any),
/// otherwise the agent participates in every round.
pub struct LlmBoardAgent {
    agent: Agent,
}

impl LlmBoardAgent {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }
}

#[async_trait]
impl BoardAgent for LlmBoardAgent {
    fn name(&self) -> &str {
        &self.agent.name
    }

    fn can_contribute(&self, board: &BoardSnapshot) -> bool {
        let Some(ref triggers) = self.agent.metadata.triggers else {
            return true;
        };

        // Round bounds
        if board.round < triggers.min_round {
            return false;
        }
        if let Some(max) = triggers.max_round
            && board.round > max
        {
            return false;
        }

        // All required entry kinds must be present on the board
        if !triggers.requires.is_empty() {
            let present_kinds: Vec<&str> = board
                .entries
                .iter()
                .map(|e| entry_kind_name(&e.kind))
                .collect();
            for req in &triggers.requires {
                let req_lower = req.to_lowercase();
                if !present_kinds.iter().any(|k| *k == req_lower) {
                    return false;
                }
            }
        }

        // None of the excluded entry kinds may be present
        if !triggers.excludes.is_empty() {
            let present_kinds: Vec<&str> = board
                .entries
                .iter()
                .map(|e| entry_kind_name(&e.kind))
                .collect();
            for excl in &triggers.excludes {
                let excl_lower = excl.to_lowercase();
                if present_kinds.iter().any(|k| *k == excl_lower) {
                    return false;
                }
            }
        }

        true
    }

    fn priority(&self, _board: &BoardSnapshot) -> u8 {
        self.agent
            .metadata
            .triggers
            .as_ref()
            .map(|t| t.priority)
            .unwrap_or(50)
    }

    async fn contribute(
        &self,
        board: &BoardSnapshot,
        provider: &dyn Provider,
    ) -> anyhow::Result<Vec<BoardDelta>> {
        let mut user_msg = format!(
            "Task: {}\nRound: {}\nBudget remaining: {} tokens\n",
            board.task, board.round, board.budget_remaining
        );

        // Include recent entries for context
        if !board.entries.is_empty() {
            user_msg.push_str("\nRecent board entries:\n");
            for entry in board.entries.iter().rev().take(10) {
                user_msg.push_str(&format!(
                    "- [{}] {:?}: {}\n",
                    entry.agent, entry.kind, entry.content
                ));
            }
        }

        let request = CompletionRequest {
            model: agent_model(&self.agent),
            system_prompt: self.agent.system_prompt.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_msg,
            }],
            temperature: self.agent.metadata.temperature,
            max_tokens: self.agent.metadata.max_tokens,
        };

        let response = provider.complete(request).await?;

        let entry = BoardEntry {
            index: 0, // assigned by Board::apply_deltas
            agent: self.agent.name.clone(),
            round: board.round,
            kind: EntryKind::Finding,
            content: response.content,
            references: vec![],
            confidence: 0.8,
            tokens_used: TokenCount {
                input: response.tokens_in,
                output: response.tokens_out,
            },
            created_at: Utc::now(),
        };

        Ok(vec![BoardDelta::AddEntry(entry)])
    }
}

// ── LlmRingAgent ─────────────────────────────────────────────────

/// LLM-backed agent that participates in Ring orchestration.
///
/// Wraps an `Agent` definition. The role is derived from the agent's
/// `AgentRingConfig` if present, defaulting to `Specialist { domain: "general" }`.
pub struct LlmRingAgent {
    agent: Agent,
}

impl LlmRingAgent {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }
}

#[async_trait]
impl RingAgent for LlmRingAgent {
    fn name(&self) -> &str {
        &self.agent.name
    }

    fn role(&self) -> RingRole {
        match &self.agent.metadata.ring_config {
            Some(config) => match config.role.to_lowercase().as_str() {
                "initiator" => RingRole::Initiator,
                "challenger" => RingRole::Challenger,
                "synthesizer" => RingRole::Synthesizer,
                other => RingRole::Specialist {
                    domain: other.to_string(),
                },
            },
            None => RingRole::Specialist {
                domain: "general".to_string(),
            },
        }
    }

    async fn process(
        &self,
        token: &TokenSnapshot,
        provider: &dyn Provider,
    ) -> anyhow::Result<Contribution> {
        let mut user_msg = format!(
            "Task: {}\nLap: {}\nYour position: {}/{}\n",
            token.task,
            token.lap,
            token.current_position + 1,
            token.ring_order.len()
        );

        if !token.contributions.is_empty() {
            user_msg.push_str("\nPrevious contributions:\n");
            for c in &token.contributions {
                user_msg.push_str(&format!(
                    "- [Lap {} / {}] {}: {}\n",
                    c.lap, c.position_in_lap, c.agent, c.content
                ));
            }
        }

        user_msg.push_str("\nProvide your contribution to this task.");

        let request = CompletionRequest {
            model: agent_model(&self.agent),
            system_prompt: self.agent.system_prompt.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_msg,
            }],
            temperature: self.agent.metadata.temperature,
            max_tokens: self.agent.metadata.max_tokens,
        };

        let response = provider.complete(request).await?;

        Ok(Contribution {
            agent: self.agent.name.clone(),
            lap: token.lap,
            position_in_lap: token.current_position,
            action: ContributionAction::Propose,
            content: response.content,
            reactions: vec![],
            tokens_used: TokenCount {
                input: response.tokens_in,
                output: response.tokens_out,
            },
            created_at: Utc::now(),
        })
    }

    async fn vote(&self, token: &TokenSnapshot, provider: &dyn Provider) -> anyhow::Result<Vote> {
        let mut user_msg = format!("Task: {}\n\nAll contributions:\n", token.task);

        for c in &token.contributions {
            user_msg.push_str(&format!(
                "- [Lap {} / {}] {}: {}\n",
                c.lap, c.position_in_lap, c.agent, c.content
            ));
        }

        user_msg.push_str(
            "\nBased on all contributions above, state your final position \
             in one sentence. Rate your confidence from 0.0 to 1.0.",
        );

        let request = CompletionRequest {
            model: agent_model(&self.agent),
            system_prompt: self.agent.system_prompt.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_msg,
            }],
            temperature: self.agent.metadata.temperature,
            max_tokens: self.agent.metadata.max_tokens,
        };

        let response = provider.complete(request).await?;

        Ok(Vote {
            position: response.content,
            confidence: 0.8,
            supporting_contributions: (0..token.contributions.len()).collect(),
            unresolved_concerns: vec![],
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::core::agent::{Agent, AgentMetadata};
    use crate::core::orchestration::blackboard::{
        BlackboardConfig, Board, BoardState, run_blackboard,
    };
    use crate::core::orchestration::ring::{
        RingConfig, RingOutcome, RingToken, TokenStatus, run_ring,
    };
    use crate::core::orchestration::test_helpers::noop_providers;
    use crate::core::orchestration::{AgentRingConfig, TriggerConfig};

    fn make_agent(name: &str) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
                provider: "anthropic".to_string(),
                model: Some("test-model".to_string()),
                command: None,
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout: None,
                tags: vec![],
                stacks: vec![],
                scope: vec![],
                model_fallback: vec![],
                cost_limit: None,
                rate_limit: None,
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: "You are a test agent.".to_string(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    fn make_agent_with_triggers(name: &str, triggers: TriggerConfig) -> Agent {
        let mut agent = make_agent(name);
        agent.metadata.triggers = Some(triggers);
        agent
    }

    fn make_agent_with_ring(name: &str, role: &str) -> Agent {
        let mut agent = make_agent(name);
        agent.metadata.ring_config = Some(AgentRingConfig {
            role: role.to_string(),
            position: None,
            vote_weight: 1.0,
        });
        agent
    }

    fn empty_snapshot(round: u32) -> BoardSnapshot {
        BoardSnapshot {
            task: "test task".to_string(),
            entries: Arc::new(vec![]),
            round,
            state: BoardState::Open,
            context: Default::default(),
            budget_remaining: 50_000,
        }
    }

    fn snapshot_with_entries(round: u32, kinds: Vec<EntryKind>) -> BoardSnapshot {
        let entries: Vec<BoardEntry> = kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| BoardEntry {
                index: i,
                agent: "other".to_string(),
                round: 0,
                kind,
                content: "entry".to_string(),
                references: vec![],
                confidence: 0.8,
                tokens_used: TokenCount::default(),
                created_at: Utc::now(),
            })
            .collect();
        BoardSnapshot {
            task: "test task".to_string(),
            entries: Arc::new(entries),
            round,
            state: BoardState::Open,
            context: Default::default(),
            budget_remaining: 50_000,
        }
    }

    // ── can_contribute tests ─────────────────────────────────────

    #[test]
    fn test_can_contribute_no_triggers_always_true() {
        let agent = LlmBoardAgent::new(make_agent("a"));
        assert!(agent.can_contribute(&empty_snapshot(0)));
        assert!(agent.can_contribute(&empty_snapshot(99)));
    }

    #[test]
    fn test_can_contribute_min_round() {
        let agent = LlmBoardAgent::new(make_agent_with_triggers(
            "a",
            TriggerConfig {
                requires: vec![],
                excludes: vec![],
                min_round: 2,
                max_round: None,
                priority: 50,
            },
        ));
        assert!(!agent.can_contribute(&empty_snapshot(0)));
        assert!(!agent.can_contribute(&empty_snapshot(1)));
        assert!(agent.can_contribute(&empty_snapshot(2)));
        assert!(agent.can_contribute(&empty_snapshot(5)));
    }

    #[test]
    fn test_can_contribute_max_round() {
        let agent = LlmBoardAgent::new(make_agent_with_triggers(
            "a",
            TriggerConfig {
                requires: vec![],
                excludes: vec![],
                min_round: 0,
                max_round: Some(3),
                priority: 50,
            },
        ));
        assert!(agent.can_contribute(&empty_snapshot(0)));
        assert!(agent.can_contribute(&empty_snapshot(3)));
        assert!(!agent.can_contribute(&empty_snapshot(4)));
    }

    #[test]
    fn test_can_contribute_requires_present() {
        let agent = LlmBoardAgent::new(make_agent_with_triggers(
            "a",
            TriggerConfig {
                requires: vec!["finding".to_string()],
                excludes: vec![],
                min_round: 0,
                max_round: None,
                priority: 50,
            },
        ));

        // No entries → requires not met
        assert!(!agent.can_contribute(&empty_snapshot(0)));

        // Has a Finding → requires met
        let snap = snapshot_with_entries(0, vec![EntryKind::Finding]);
        assert!(agent.can_contribute(&snap));
    }

    #[test]
    fn test_can_contribute_requires_missing() {
        let agent = LlmBoardAgent::new(make_agent_with_triggers(
            "a",
            TriggerConfig {
                requires: vec!["challenge".to_string()],
                excludes: vec![],
                min_round: 0,
                max_round: None,
                priority: 50,
            },
        ));

        // Only has Finding, not Challenge
        let snap = snapshot_with_entries(0, vec![EntryKind::Finding]);
        assert!(!agent.can_contribute(&snap));
    }

    #[test]
    fn test_can_contribute_excludes_blocks() {
        let agent = LlmBoardAgent::new(make_agent_with_triggers(
            "a",
            TriggerConfig {
                requires: vec![],
                excludes: vec!["synthesis".to_string()],
                min_round: 0,
                max_round: None,
                priority: 50,
            },
        ));

        // No synthesis → allowed
        let snap = snapshot_with_entries(0, vec![EntryKind::Finding]);
        assert!(agent.can_contribute(&snap));

        // Has Synthesis → blocked
        let snap = snapshot_with_entries(0, vec![EntryKind::Synthesis { sources: vec![] }]);
        assert!(!agent.can_contribute(&snap));
    }

    // ── priority tests ───────────────────────────────────────────

    #[test]
    fn test_priority_default() {
        let agent = LlmBoardAgent::new(make_agent("a"));
        assert_eq!(agent.priority(&empty_snapshot(0)), 50);
    }

    #[test]
    fn test_priority_from_triggers() {
        let agent = LlmBoardAgent::new(make_agent_with_triggers(
            "a",
            TriggerConfig {
                requires: vec![],
                excludes: vec![],
                min_round: 0,
                max_round: None,
                priority: 90,
            },
        ));
        assert_eq!(agent.priority(&empty_snapshot(0)), 90);
    }

    // ── role() mapping tests ─────────────────────────────────────

    #[test]
    fn test_ring_role_initiator() {
        let agent = LlmRingAgent::new(make_agent_with_ring("a", "initiator"));
        assert_eq!(agent.role(), RingRole::Initiator);
    }

    #[test]
    fn test_ring_role_challenger() {
        let agent = LlmRingAgent::new(make_agent_with_ring("a", "challenger"));
        assert_eq!(agent.role(), RingRole::Challenger);
    }

    #[test]
    fn test_ring_role_synthesizer() {
        let agent = LlmRingAgent::new(make_agent_with_ring("a", "synthesizer"));
        assert_eq!(agent.role(), RingRole::Synthesizer);
    }

    #[test]
    fn test_ring_role_specialist_from_unknown() {
        let agent = LlmRingAgent::new(make_agent_with_ring("a", "security"));
        assert_eq!(
            agent.role(),
            RingRole::Specialist {
                domain: "security".to_string()
            }
        );
    }

    #[test]
    fn test_ring_role_default_no_config() {
        let agent = LlmRingAgent::new(make_agent("a"));
        assert_eq!(
            agent.role(),
            RingRole::Specialist {
                domain: "general".to_string()
            }
        );
    }

    #[test]
    fn test_ring_role_case_insensitive() {
        let agent = LlmRingAgent::new(make_agent_with_ring("a", "INITIATOR"));
        assert_eq!(agent.role(), RingRole::Initiator);
    }

    // ── Integration: blackboard with LlmBoardAgents ──────────────

    #[tokio::test]
    async fn test_integration_blackboard_produces_entries() {
        let agents: Vec<Arc<dyn BoardAgent>> = vec![
            Arc::new(LlmBoardAgent::new(make_agent("agent-a"))),
            Arc::new(LlmBoardAgent::new(make_agent("agent-b"))),
        ];
        let providers = noop_providers();
        let config = BlackboardConfig {
            max_rounds: 2,
            ..Default::default()
        };
        let mut board = Board::new("test task".to_string(), config.token_budget);

        run_blackboard(&mut board, &agents, &providers, &config)
            .await
            .unwrap();

        // Board must be halted (max_rounds reached)
        assert!(board.is_halted() || board.round >= config.max_rounds);

        // Both agents should have contributed at least once
        let agent_a_entries = board
            .entries()
            .iter()
            .filter(|e| e.agent == "agent-a")
            .count();
        let agent_b_entries = board
            .entries()
            .iter()
            .filter(|e| e.agent == "agent-b")
            .count();
        assert!(agent_a_entries >= 1, "agent-a should have contributed");
        assert!(agent_b_entries >= 1, "agent-b should have contributed");

        // All entries should be Findings (our LlmBoardAgent always produces Findings)
        for entry in board.entries() {
            assert!(
                matches!(entry.kind, EntryKind::Finding),
                "expected Finding, got {:?}",
                entry.kind
            );
        }
    }

    // ── Integration: ring with LlmRingAgents ─────────────────────

    #[tokio::test]
    async fn test_integration_ring_produces_outcome() {
        let agents: Vec<Arc<dyn RingAgent>> = vec![
            Arc::new(LlmRingAgent::new(make_agent("agent-a"))),
            Arc::new(LlmRingAgent::new(make_agent("agent-b"))),
        ];
        let providers = noop_providers();
        let config = RingConfig {
            max_laps: 2,
            ..Default::default()
        };
        let order = vec!["agent-a".to_string(), "agent-b".to_string()];
        let mut token = RingToken::new("test task".to_string(), order, config.token_budget);

        run_ring(&mut token, &agents, &providers, &config)
            .await
            .unwrap();

        // Must be Done
        assert!(matches!(token.status(), TokenStatus::Done { .. }));

        // Both agents should have voted (NoopProvider returns "ok" for all)
        assert!(
            token.votes().contains_key("agent-a"),
            "agent-a should have voted"
        );
        assert!(
            token.votes().contains_key("agent-b"),
            "agent-b should have voted"
        );

        // Since both vote "ok" → same position → Consensus
        match token.status() {
            TokenStatus::Done {
                outcome: RingOutcome::Consensus { score, .. },
            } => {
                assert!((score - 1.0).abs() < f32::EPSILON);
            }
            other => panic!("Expected Consensus, got {other:?}"),
        }
    }
}
