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

// ── Structured-response parsers ─────────────────────────────────

/// Prompt suffix appended to board agent messages so the LLM returns a
/// structured action header we can parse.
const BOARD_ACTION_INSTRUCTIONS: &str = "\n\n\
Respond with the following structured header, then your content:\n\
ACTION: <type> (one of: FINDING, CHALLENGE, CONFIRMATION, SYNTHESIS, QUESTION, ANSWER)\n\
TARGET: <index> (required for CHALLENGE, CONFIRMATION, ANSWER; comma-separated for SYNTHESIS)\n\
CONFIDENCE: <0.0-1.0>\n\
CONTENT: <your actual response>\n";

/// Parse a board agent's structured response into (EntryKind, confidence, content).
///
/// Falls back to `EntryKind::Finding` with confidence 0.8 if the header cannot
/// be parsed (e.g. the LLM ignores the instructions).
pub(crate) fn parse_board_action(response: &str) -> (EntryKind, f32, String) {
    let mut action_str = None;
    let mut target_str = None;
    let mut confidence: f32 = 0.8;
    let mut content_start = None;

    for (i, line) in response.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("ACTION:") {
            action_str = Some(rest.trim().to_uppercase());
        } else if let Some(rest) = trimmed.strip_prefix("TARGET:") {
            target_str = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("CONFIDENCE:") {
            if let Ok(c) = rest.trim().parse::<f32>() {
                confidence = c.clamp(0.0, 1.0);
            }
        } else if let Some(rest) = trimmed.strip_prefix("CONTENT:") {
            // Everything from here onward is the content body.
            let remainder: String = std::iter::once(rest.trim().to_string())
                .chain(response.lines().skip(i + 1).map(|l| l.to_string()))
                .collect::<Vec<_>>()
                .join("\n");
            content_start = Some(remainder);
            break;
        }
    }

    let content = content_start.unwrap_or_else(|| response.to_string());

    // Actions that require a TARGET fall back to Finding when the index is
    // absent — this avoids silently pointing at entry 0.
    let kind = match action_str.as_deref() {
        Some(a) if a.starts_with("CHALLENGE") => match parse_single_index(&target_str) {
            Some(target) => EntryKind::Challenge { target },
            None => EntryKind::Finding,
        },
        Some(a) if a.starts_with("CONFIRMATION") => match parse_single_index(&target_str) {
            Some(target) => EntryKind::Confirmation { target },
            None => EntryKind::Finding,
        },
        Some(a) if a.starts_with("SYNTHESIS") => {
            let sources = parse_index_list(&target_str);
            EntryKind::Synthesis { sources }
        }
        Some(a) if a.starts_with("QUESTION") => EntryKind::Question,
        Some(a) if a.starts_with("ANSWER") => match parse_single_index(&target_str) {
            Some(question) => EntryKind::Answer { question },
            None => EntryKind::Finding,
        },
        // FINDING or anything unrecognised → default
        _ => EntryKind::Finding,
    };

    (kind, confidence, content)
}

/// Prompt suffix for ring agent process messages.
const RING_ACTION_INSTRUCTIONS: &str = "\n\n\
Respond with the following structured header, then your content:\n\
ACTION: <type> (one of: PROPOSE, ENRICH, CONTEST, ENDORSE, SYNTHESIZE, PASS)\n\
TARGET: <index> (required for ENRICH, CONTEST, ENDORSE)\n\
CONTENT: <your actual response>\n";

/// Parse a ring agent's structured response into (ContributionAction, content).
///
/// Falls back to `ContributionAction::Propose` if parsing fails.
pub(crate) fn parse_ring_action(response: &str) -> (ContributionAction, String) {
    let mut action_str = None;
    let mut target_str = None;
    let mut content_start = None;

    for (i, line) in response.lines().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("ACTION:") {
            action_str = Some(rest.trim().to_uppercase());
        } else if let Some(rest) = trimmed.strip_prefix("TARGET:") {
            target_str = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("CONTENT:") {
            let remainder: String = std::iter::once(rest.trim().to_string())
                .chain(response.lines().skip(i + 1).map(|l| l.to_string()))
                .collect::<Vec<_>>()
                .join("\n");
            content_start = Some(remainder);
            break;
        }
    }

    let content = content_start.unwrap_or_else(|| response.to_string());

    // Actions that require a TARGET fall back to Propose when the index is absent.
    let action = match action_str.as_deref() {
        Some(a) if a.starts_with("ENRICH") => match parse_single_index(&target_str) {
            Some(target) => ContributionAction::Enrich { target },
            None => ContributionAction::Propose,
        },
        Some(a) if a.starts_with("CONTEST") => match parse_single_index(&target_str) {
            Some(target) => ContributionAction::Contest {
                target,
                counter_argument: String::new(),
            },
            None => ContributionAction::Propose,
        },
        Some(a) if a.starts_with("ENDORSE") => match parse_single_index(&target_str) {
            Some(target) => ContributionAction::Endorse { target },
            None => ContributionAction::Propose,
        },
        Some(a) if a.starts_with("SYNTHESIZE") => ContributionAction::Synthesize,
        Some(a) if a.starts_with("PASS") => ContributionAction::Pass {
            reason: content.clone(),
        },
        // PROPOSE or anything unrecognised → default
        _ => ContributionAction::Propose,
    };

    (action, content)
}

/// Parse a confidence value from the first line of a vote response.
///
/// Falls back to 0.8 if the header is absent or malformed.
fn parse_vote_confidence(response: &str) -> (f32, String) {
    if let Some(first_line) = response.lines().next() {
        let trimmed = first_line.trim();
        if let Some(rest) = trimmed.strip_prefix("CONFIDENCE:")
            && let Ok(c) = rest.trim().parse::<f32>()
        {
            let body = response.lines().skip(1).collect::<Vec<_>>().join("\n");
            return (c.clamp(0.0, 1.0), body);
        }
    }
    (0.8, response.to_string())
}

fn parse_single_index(s: &Option<String>) -> Option<usize> {
    s.as_deref()
        .and_then(|v| v.trim().split(',').next())
        .and_then(|v| v.trim().parse::<usize>().ok())
}

fn parse_index_list(s: &Option<String>) -> Vec<usize> {
    s.as_deref()
        .map(|v| {
            v.split(',')
                .filter_map(|p| p.trim().parse::<usize>().ok())
                .collect()
        })
        .unwrap_or_default()
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
                    "- [{}#{} {}] {}\n",
                    entry.agent,
                    entry.index,
                    entry_kind_name(&entry.kind),
                    entry.content
                ));
            }
        }

        user_msg.push_str(BOARD_ACTION_INSTRUCTIONS);

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

        let (kind, confidence, content) = parse_board_action(&response.content);

        let entry = BoardEntry {
            index: 0, // assigned by Board::apply_deltas
            agent: self.agent.name.clone(),
            round: board.round,
            kind,
            content,
            references: vec![],
            confidence,
            tokens_used: TokenCount {
                input: response.tokens_in,
                output: response.tokens_out,
                cost: response.cost,
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
            for (i, c) in token.contributions.iter().enumerate() {
                user_msg.push_str(&format!(
                    "- [#{} Lap {} / {}] {}: {}\n",
                    i, c.lap, c.position_in_lap, c.agent, c.content
                ));
            }
        }

        user_msg.push_str(RING_ACTION_INSTRUCTIONS);

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

        let (action, content) = parse_ring_action(&response.content);

        Ok(Contribution {
            agent: self.agent.name.clone(),
            lap: token.lap,
            position_in_lap: token.current_position,
            action,
            content,
            reactions: vec![],
            tokens_used: TokenCount {
                input: response.tokens_in,
                output: response.tokens_out,
                cost: response.cost,
            },
            created_at: Utc::now(),
        })
    }

    fn vote_weight(&self) -> f32 {
        self.agent
            .metadata
            .ring_config
            .as_ref()
            .map(|c| c.vote_weight)
            .unwrap_or(1.0)
    }

    async fn vote(&self, token: &TokenSnapshot, provider: &dyn Provider) -> anyhow::Result<Vote> {
        let mut user_msg = format!("Task: {}\n\nAll contributions:\n", token.task);

        for c in token.contributions.iter() {
            user_msg.push_str(&format!(
                "- [Lap {} / {}] {}: {}\n",
                c.lap, c.position_in_lap, c.agent, c.content
            ));
        }

        user_msg.push_str(
            "\nSynthesize the contributions above. Identify areas of agreement, \
             unresolved disagreements, and any gaps. Then state your final \
             position in one or two sentences.\n\n\
             Format your response as:\n\
             CONFIDENCE: <0.0-1.0>\n\
             <your synthesized position>",
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

        let (confidence, position) = parse_vote_confidence(&response.content);

        Ok(Vote {
            position,
            confidence,
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

        // NoopProvider returns "ok" which has no structured header, so
        // parse_board_action falls back to Finding for every entry.
        for entry in board.entries() {
            assert!(
                matches!(entry.kind, EntryKind::Finding),
                "expected Finding (fallback), got {:?}",
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

    // ── Parser unit tests ────────────────────────────────────────

    // -- parse_board_action --

    #[test]
    fn test_parse_board_action_complete_header() {
        let response = "ACTION: CHALLENGE\nTARGET: 3\nCONFIDENCE: 0.9\nCONTENT: I disagree";
        let (kind, conf, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Challenge { target: 3 }));
        assert!((conf - 0.9).abs() < f32::EPSILON);
        assert_eq!(content, "I disagree");
    }

    #[test]
    fn test_parse_board_action_confirmation() {
        let response = "ACTION: CONFIRMATION\nTARGET: 0\nCONFIDENCE: 0.95\nCONTENT: Agreed";
        let (kind, conf, _) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Confirmation { target: 0 }));
        assert!((conf - 0.95).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_board_action_synthesis_multi_target() {
        let response = "ACTION: SYNTHESIS\nTARGET: 0, 2, 5\nCONTENT: Combined view";
        let (kind, _, content) = parse_board_action(response);
        match kind {
            EntryKind::Synthesis { sources } => assert_eq!(sources, vec![0, 2, 5]),
            other => panic!("Expected Synthesis, got {other:?}"),
        }
        assert_eq!(content, "Combined view");
    }

    #[test]
    fn test_parse_board_action_question() {
        let response = "ACTION: QUESTION\nCONTENT: What about edge cases?";
        let (kind, _, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Question));
        assert_eq!(content, "What about edge cases?");
    }

    #[test]
    fn test_parse_board_action_answer() {
        let response = "ACTION: ANSWER\nTARGET: 4\nCONTENT: Here is the answer";
        let (kind, _, _) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Answer { question: 4 }));
    }

    #[test]
    fn test_parse_board_action_no_header_fallback() {
        let response = "Just some plain text without any structured header";
        let (kind, conf, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Finding));
        assert!((conf - 0.8).abs() < f32::EPSILON);
        assert_eq!(content, response);
    }

    #[test]
    fn test_parse_board_action_challenge_no_target_fallback() {
        // CHALLENGE without TARGET should fallback to Finding
        let response = "ACTION: CHALLENGE\nCONFIDENCE: 0.7\nCONTENT: I disagree";
        let (kind, _, _) = parse_board_action(response);
        assert!(
            matches!(kind, EntryKind::Finding),
            "CHALLENGE without TARGET should fallback to Finding, got {kind:?}"
        );
    }

    #[test]
    fn test_parse_board_action_confirmation_no_target_fallback() {
        let response = "ACTION: CONFIRMATION\nCONTENT: Looks good";
        let (kind, _, _) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Finding));
    }

    #[test]
    fn test_parse_board_action_multiline_content() {
        let response = "ACTION: FINDING\nCONFIDENCE: 0.6\nCONTENT: Line one\nLine two\nLine three";
        let (kind, conf, content) = parse_board_action(response);
        assert!(matches!(kind, EntryKind::Finding));
        assert!((conf - 0.6).abs() < f32::EPSILON);
        assert_eq!(content, "Line one\nLine two\nLine three");
    }

    #[test]
    fn test_parse_board_action_confidence_clamped() {
        let response = "ACTION: FINDING\nCONFIDENCE: 5.0\nCONTENT: high";
        let (_, conf, _) = parse_board_action(response);
        assert!((conf - 1.0).abs() < f32::EPSILON);

        let response = "ACTION: FINDING\nCONFIDENCE: -2.0\nCONTENT: low";
        let (_, conf, _) = parse_board_action(response);
        assert!((conf - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_board_action_invalid_confidence_keeps_default() {
        let response = "ACTION: FINDING\nCONFIDENCE: not_a_number\nCONTENT: text";
        let (_, conf, _) = parse_board_action(response);
        assert!((conf - 0.8).abs() < f32::EPSILON);
    }

    // -- parse_ring_action --

    #[test]
    fn test_parse_ring_action_propose() {
        let response = "ACTION: PROPOSE\nCONTENT: Use Rust for this";
        let (action, content) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Propose));
        assert_eq!(content, "Use Rust for this");
    }

    #[test]
    fn test_parse_ring_action_enrich() {
        let response = "ACTION: ENRICH\nTARGET: 2\nCONTENT: Adding error handling";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Enrich { target: 2 }));
    }

    #[test]
    fn test_parse_ring_action_contest() {
        let response = "ACTION: CONTEST\nTARGET: 1\nCONTENT: Performance concern";
        let (action, content) = parse_ring_action(response);
        match action {
            ContributionAction::Contest {
                target,
                counter_argument,
            } => {
                assert_eq!(target, 1);
                // counter_argument is empty (content is in the Contribution.content field)
                assert!(counter_argument.is_empty());
            }
            other => panic!("Expected Contest, got {other:?}"),
        }
        assert_eq!(content, "Performance concern");
    }

    #[test]
    fn test_parse_ring_action_endorse() {
        let response = "ACTION: ENDORSE\nTARGET: 0\nCONTENT: Fully agree";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Endorse { target: 0 }));
    }

    #[test]
    fn test_parse_ring_action_synthesize() {
        let response = "ACTION: SYNTHESIZE\nCONTENT: Combining all views";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Synthesize));
    }

    #[test]
    fn test_parse_ring_action_pass() {
        let response = "ACTION: PASS\nCONTENT: Nothing to add";
        let (action, _) = parse_ring_action(response);
        match action {
            ContributionAction::Pass { reason } => assert_eq!(reason, "Nothing to add"),
            other => panic!("Expected Pass, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_ring_action_no_header_fallback() {
        let response = "Just a plain response";
        let (action, content) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Propose));
        assert_eq!(content, response);
    }

    #[test]
    fn test_parse_ring_action_enrich_no_target_fallback() {
        let response = "ACTION: ENRICH\nCONTENT: More detail";
        let (action, _) = parse_ring_action(response);
        assert!(
            matches!(action, ContributionAction::Propose),
            "ENRICH without TARGET should fallback to Propose"
        );
    }

    #[test]
    fn test_parse_ring_action_contest_no_target_fallback() {
        let response = "ACTION: CONTEST\nCONTENT: I disagree";
        let (action, _) = parse_ring_action(response);
        assert!(matches!(action, ContributionAction::Propose));
    }

    // -- parse_vote_confidence --

    #[test]
    fn test_parse_vote_confidence_valid() {
        let response = "CONFIDENCE: 0.75\nI agree with the proposal";
        let (conf, body) = parse_vote_confidence(response);
        assert!((conf - 0.75).abs() < f32::EPSILON);
        assert_eq!(body, "I agree with the proposal");
    }

    #[test]
    fn test_parse_vote_confidence_clamped() {
        let response = "CONFIDENCE: 99.0\nOverconfident";
        let (conf, _) = parse_vote_confidence(response);
        assert!((conf - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parse_vote_confidence_missing() {
        let response = "I just have an opinion";
        let (conf, body) = parse_vote_confidence(response);
        assert!((conf - 0.8).abs() < f32::EPSILON);
        assert_eq!(body, response);
    }

    #[test]
    fn test_parse_vote_confidence_malformed() {
        let response = "CONFIDENCE: high\nMy position";
        let (conf, body) = parse_vote_confidence(response);
        assert!((conf - 0.8).abs() < f32::EPSILON);
        assert_eq!(body, response); // entire response since parse failed
    }

    // -- parse_single_index / parse_index_list --

    #[test]
    fn test_parse_single_index_valid() {
        assert_eq!(parse_single_index(&Some("5".to_string())), Some(5));
        assert_eq!(parse_single_index(&Some(" 3 ".to_string())), Some(3));
    }

    #[test]
    fn test_parse_single_index_from_list() {
        // Takes first index from comma-separated
        assert_eq!(parse_single_index(&Some("2, 5, 7".to_string())), Some(2));
    }

    #[test]
    fn test_parse_single_index_none() {
        assert_eq!(parse_single_index(&None), None);
    }

    #[test]
    fn test_parse_single_index_invalid() {
        assert_eq!(parse_single_index(&Some("abc".to_string())), None);
    }

    #[test]
    fn test_parse_index_list_valid() {
        assert_eq!(
            parse_index_list(&Some("0, 2, 5".to_string())),
            vec![0, 2, 5]
        );
    }

    #[test]
    fn test_parse_index_list_none() {
        assert!(parse_index_list(&None).is_empty());
    }

    #[test]
    fn test_parse_index_list_mixed_invalid() {
        // Skips invalid entries
        assert_eq!(parse_index_list(&Some("1, abc, 3".to_string())), vec![1, 3]);
    }
}
