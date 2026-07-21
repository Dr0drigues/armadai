use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use super::blackboard::{
    BoardAgent, BoardDelta, BoardEntry, BoardSnapshot, EntryKind, TokenCount, entry_kind_name,
};
use super::ring::{Contribution, ContributionAction, RingAgent, RingRole, TokenSnapshot, Vote};
use crate::core::agent::Agent;
use crate::core::events::{EventSink, NullSink, RunEvent};
use crate::core::routing::{BudgetState, RoutingRules, route};
use crate::providers::traits::{ChatMessage, CompletionRequest, Provider};

/// Routing context threaded through the orchestration engines so
/// `LlmBoardAgent`/`LlmRingAgent` can route `latest:auto` models the same way
/// `run_single_agent` does (spec: OH4 router in orchestration).
///
/// `rules` is shared via `Arc` since one context is cloned into every agent
/// built for a run. `total_budget` is the board/ring's *configured*
/// `token_budget` (constant for the run) — not the remaining amount, which
/// changes every round/lap and is read from the snapshot at route time.
/// `None` (or a configured budget of `0`) disables budget-aware downgrade
/// entirely: `route()` is still called, just with `budget: None`.
#[derive(Clone)]
pub struct RoutingCtx {
    rules: Arc<RoutingRules>,
    total_budget: Option<u64>,
}

impl RoutingCtx {
    /// `total_budget` is the board/ring's configured token budget (e.g.
    /// `BlackboardConfig::token_budget` / `RingConfig::token_budget`).
    pub fn new(rules: RoutingRules, total_budget: u64) -> Self {
        Self {
            rules: Arc::new(rules),
            total_budget: (total_budget > 0).then_some(total_budget),
        }
    }

    /// Derive the router's `BudgetState` from tokens remaining vs. the
    /// configured total. `None` when no budget is configured for this run.
    fn budget_state(&self, remaining: u64) -> Option<BudgetState> {
        self.total_budget.map(|total| BudgetState {
            remaining_ratio: remaining as f64 / total as f64,
        })
    }
}

impl Default for RoutingCtx {
    fn default() -> Self {
        Self {
            rules: Arc::new(RoutingRules::default()),
            total_budget: None,
        }
    }
}

/// Resolve the model string for a completion request.
///
/// Concrete models, `command`-based agents, and `latest:pro/fast/max`
/// placeholders pass through unchanged — those are resolved later by the
/// linker (`resolve_model_for_tier` is only reached here for `latest:auto`;
/// `parse_latest_placeholder` deliberately does not recognize it, see
/// `linker::model_resolution`). `latest:auto` is routed through the OH4
/// router (`core::routing::route`) using `input` (the prompt about to be
/// sent), the agent's tags, and the budget derived from `budget_remaining`
/// via `routing`. Emits `RunEvent::Route` on the routed path — budget only
/// ever *downgrades* the tier (see `route`), it never fails the run.
fn agent_model(
    agent: &Agent,
    input: &str,
    budget_remaining: u64,
    routing: &RoutingCtx,
    sink: &Arc<dyn EventSink>,
) -> String {
    let raw = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

    if raw != "latest:auto" {
        return raw;
    }

    let budget = routing.budget_state(budget_remaining);
    let (tier, reason) = route(input, &agent.metadata.tags, budget, &routing.rules);
    sink.emit(&RunEvent::Route {
        agent: agent.name.clone(),
        tier: format!("{tier:?}"),
        reason: format!("{reason:?}"),
    });
    crate::linker::model_resolution::resolve_model_for_tier(&agent.metadata.provider, tier)
}

// ── Structured-response parsers ─────────────────────────────────

/// Prompt suffix appended to board agent messages so the LLM returns a
/// structured action header we can parse.
///
/// `pub(crate)` (rather than private) so `es::blackboard::BlackboardEffectRunner`
/// (OH1 Lot 3, Task 4) can reuse the exact same instructions when assembling
/// its own board prompt, instead of duplicating this string.
pub(crate) const BOARD_ACTION_INSTRUCTIONS: &str = "\n\n\
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
    routing: RoutingCtx,
    sink: Arc<dyn EventSink>,
}

impl LlmBoardAgent {
    pub fn new(agent: Agent) -> Self {
        Self {
            agent,
            routing: RoutingCtx::default(),
            sink: Arc::new(NullSink),
        }
    }

    /// Construct with an explicit routing context and event sink. Used by
    /// `run_orchestrated` so `latest:auto` agents route through the
    /// project's `RoutingRules` and the board's configured budget, emitting
    /// `RunEvent::Route` via `sink`.
    pub fn with_routing(agent: Agent, routing: RoutingCtx, sink: Arc<dyn EventSink>) -> Self {
        Self {
            agent,
            routing,
            sink,
        }
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
            model: agent_model(
                &self.agent,
                &user_msg,
                board.budget_remaining,
                &self.routing,
                &self.sink,
            ),
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
    routing: RoutingCtx,
    sink: Arc<dyn EventSink>,
}

impl LlmRingAgent {
    pub fn new(agent: Agent) -> Self {
        Self {
            agent,
            routing: RoutingCtx::default(),
            sink: Arc::new(NullSink),
        }
    }

    /// Construct with an explicit routing context and event sink. Used by
    /// `run_orchestrated` so `latest:auto` agents route through the
    /// project's `RoutingRules` and the ring's configured budget, emitting
    /// `RunEvent::Route` via `sink`.
    pub fn with_routing(agent: Agent, routing: RoutingCtx, sink: Arc<dyn EventSink>) -> Self {
        Self {
            agent,
            routing,
            sink,
        }
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
            model: agent_model(
                &self.agent,
                &user_msg,
                token.budget_remaining,
                &self.routing,
                &self.sink,
            ),
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
            model: agent_model(
                &self.agent,
                &user_msg,
                token.budget_remaining,
                &self.routing,
                &self.sink,
            ),
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
    use crate::core::events::{EventSink, NullSink};
    use crate::core::orchestration::blackboard::{
        BlackboardConfig, Board, BoardState, run_blackboard,
    };
    use crate::core::orchestration::ring::{
        RingConfig, RingOutcome, RingToken, TokenStatus, run_ring,
    };
    use crate::core::orchestration::test_helpers::noop_providers;
    use crate::core::orchestration::{AgentRingConfig, TriggerConfig};

    /// No-op sink for tests that don't assert on emitted events.
    fn null_sink() -> Arc<dyn EventSink> {
        Arc::new(NullSink)
    }

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

        run_blackboard(&mut board, &agents, &providers, &config, &null_sink())
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

        run_ring(&mut token, &agents, &providers, &config, &null_sink())
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

    // ── latest:auto routing in orchestration (RoutingCtx) ────────────

    /// Records the `model` of every `CompletionRequest` it receives, so tests
    /// can assert what `agent_model` resolved `latest:auto` to.
    struct CapturingProvider(std::sync::Mutex<Vec<String>>);

    impl CapturingProvider {
        fn new() -> Self {
            Self(std::sync::Mutex::new(Vec::new()))
        }

        fn models(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for CapturingProvider {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> anyhow::Result<crate::providers::traits::CompletionResponse> {
            self.0.lock().unwrap().push(request.model.clone());
            Ok(crate::providers::traits::CompletionResponse {
                content: "ACTION: FINDING\nCONFIDENCE: 0.8\nCONTENT: ok".to_string(),
                model: request.model,
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
            })
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> anyhow::Result<crate::providers::traits::TokenStream> {
            unimplemented!("not exercised by these tests")
        }

        fn metadata(&self) -> crate::providers::traits::ProviderMetadata {
            crate::providers::traits::ProviderMetadata {
                name: "capturing".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    /// Capture-only sink so tests can assert on emitted `RunEvent`s (mirrors
    /// the `CaptureSink` used in `ring.rs`'s own test module).
    struct CaptureSink(std::sync::Mutex<Vec<String>>);

    impl CaptureSink {
        fn new() -> Self {
            Self(std::sync::Mutex::new(Vec::new()))
        }

        fn events(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    impl EventSink for CaptureSink {
        fn emit(&self, ev: &RunEvent) {
            if let Ok(s) = serde_json::to_string(ev) {
                self.0.lock().unwrap().push(s);
            }
        }
    }

    fn make_agent_latest_auto(name: &str, tags: Vec<String>) -> Agent {
        let mut agent = make_agent(name);
        agent.metadata.model = Some("latest:auto".to_string());
        agent.metadata.tags = tags;
        agent
    }

    /// RAII guard that points `ARMADAI_CONFIG_DIR` at a private, empty
    /// tempdir for the lifetime of the guard, so `resolve_model_for_tier`
    /// (called both by the production code path under test and by the
    /// test's own expected-value computation) can never observe the
    /// ambient/machine-local models.dev cache.
    ///
    /// Without this, two calls to `resolve_model_for_tier` in the same test
    /// could race against a *different* test (in another thread) that is
    /// concurrently fetching/writing the real shared cache file, or could
    /// simply see different content across machines/CI — exactly the
    /// "present/absent/partial depending on machine + parallel runs"
    /// flakiness this guard eliminates. Serialised via `ENV_MUTEX` since
    /// mutating `ARMADAI_CONFIG_DIR` is process-global state.
    struct IsolatedModelCache {
        _guard: std::sync::MutexGuard<'static, ()>,
        _tmp: tempfile::TempDir,
        orig: Option<String>,
    }

    impl IsolatedModelCache {
        fn new() -> Self {
            let guard = crate::core::config::ENV_MUTEX.lock().unwrap();
            let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
            let tmp = tempfile::tempdir().expect("tempdir");
            // SAFETY: env mutation is serialised via ENV_MUTEX for the
            // lifetime of this guard, and the original value is restored on
            // drop.
            unsafe {
                std::env::set_var("ARMADAI_CONFIG_DIR", tmp.path());
            }
            Self {
                _guard: guard,
                _tmp: tmp,
                orig,
            }
        }
    }

    impl Drop for IsolatedModelCache {
        fn drop(&mut self) {
            // SAFETY: still holding `_guard` (ENV_MUTEX) at this point.
            match self.orig.take() {
                Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
                None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
            }
        }
    }

    #[tokio::test]
    async fn test_board_agent_latest_auto_downgrades_on_low_budget() {
        // The "critical" tag maps to Max under default RoutingRules, but the
        // board's remaining budget is far below `budget_downgrade_ratio`
        // (default 0.2) — the router must downgrade to Fast and report
        // RouteReason::Budget, exactly as it would for run_single_agent.
        let _isolated = IsolatedModelCache::new();

        let agent = make_agent_latest_auto("agent-a", vec!["critical".to_string()]);
        let routing = RoutingCtx::new(RoutingRules::default(), 1_000);
        let sink = Arc::new(CaptureSink::new());
        let board_agent =
            LlmBoardAgent::with_routing(agent, routing, sink.clone() as Arc<dyn EventSink>);

        let snapshot = BoardSnapshot {
            task: "test task".to_string(),
            entries: Arc::new(vec![]),
            round: 0,
            state: BoardState::Open,
            context: Default::default(),
            budget_remaining: 50, // 50 / 1_000 = 0.05 <= 0.2 downgrade threshold
        };

        let provider = CapturingProvider::new();
        board_agent.contribute(&snapshot, &provider).await.unwrap();

        let models = provider.models();
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0],
            crate::linker::model_resolution::resolve_model_for_tier(
                "anthropic",
                crate::linker::model_resolution::ModelTier::Fast,
            ),
            "latest:auto should resolve to the Fast tier's model when budget is low"
        );

        let events = sink.events();
        assert!(
            events.iter().any(|e| e.contains(r#""t":"route""#)
                && e.contains(r#""agent":"agent-a""#)
                && e.contains(r#""tier":"Fast""#)
                && e.contains(r#""reason":"Budget""#)),
            "expected a Route event with tier=Fast reason=Budget, got: {events:?}"
        );
    }

    #[tokio::test]
    async fn test_board_agent_latest_auto_no_downgrade_when_budget_healthy() {
        // Same "critical" tag (→ Max) but a healthy budget: no downgrade,
        // reason stays Tag — proves the budget check is not unconditional.
        let _isolated = IsolatedModelCache::new();

        let agent = make_agent_latest_auto("agent-a", vec!["critical".to_string()]);
        let routing = RoutingCtx::new(RoutingRules::default(), 1_000);
        let sink = Arc::new(CaptureSink::new());
        let board_agent =
            LlmBoardAgent::with_routing(agent, routing, sink.clone() as Arc<dyn EventSink>);

        let snapshot = BoardSnapshot {
            task: "test task".to_string(),
            entries: Arc::new(vec![]),
            round: 0,
            state: BoardState::Open,
            context: Default::default(),
            budget_remaining: 900, // 900 / 1_000 = 0.9, well above threshold
        };

        let provider = CapturingProvider::new();
        board_agent.contribute(&snapshot, &provider).await.unwrap();

        let models = provider.models();
        assert_eq!(
            models[0],
            crate::linker::model_resolution::resolve_model_for_tier(
                "anthropic",
                crate::linker::model_resolution::ModelTier::Max,
            )
        );

        let events = sink.events();
        assert!(
            events.iter().any(|e| e.contains(r#""t":"route""#)
                && e.contains(r#""tier":"Max""#)
                && e.contains(r#""reason":"Tag""#)),
            "expected a Route event with tier=Max reason=Tag, got: {events:?}"
        );
    }

    #[tokio::test]
    async fn test_ring_agent_latest_auto_downgrades_on_low_budget() {
        let _isolated = IsolatedModelCache::new();

        let agent = make_agent_latest_auto("agent-a", vec!["critical".to_string()]);
        let routing = RoutingCtx::new(RoutingRules::default(), 1_000);
        let sink = Arc::new(CaptureSink::new());
        let ring_agent =
            LlmRingAgent::with_routing(agent, routing, sink.clone() as Arc<dyn EventSink>);

        let snapshot = TokenSnapshot {
            task: "test task".to_string(),
            contributions: Arc::new(vec![]),
            lap: 0,
            status: TokenStatus::Circulating,
            ring_order: vec!["agent-a".to_string()],
            current_position: 0,
            budget_remaining: 30, // 30 / 1_000 = 0.03, well under threshold
        };

        let provider = CapturingProvider::new();
        ring_agent.process(&snapshot, &provider).await.unwrap();

        let models = provider.models();
        assert_eq!(
            models[0],
            crate::linker::model_resolution::resolve_model_for_tier(
                "anthropic",
                crate::linker::model_resolution::ModelTier::Fast,
            )
        );

        let events = sink.events();
        assert!(
            events
                .iter()
                .any(|e| e.contains(r#""t":"route""#) && e.contains(r#""reason":"Budget""#)),
            "expected a Route event with reason=Budget for the ring agent, got: {events:?}"
        );
    }

    #[test]
    fn test_agent_model_concrete_and_latest_pro_unaffected_by_routing() {
        // Non-regression: concrete models and latest:pro/fast/max are NOT
        // routed through `route()` — they pass through unchanged regardless
        // of budget/RoutingCtx. Only `latest:auto` is special-cased.
        let routing = RoutingCtx::new(RoutingRules::default(), 1_000);
        let sink: Arc<dyn EventSink> = Arc::new(NullSink);

        let mut concrete = make_agent("agent-concrete");
        concrete.metadata.model = Some("claude-sonnet-4-5-20250929".to_string());
        assert_eq!(
            agent_model(&concrete, "any input", 10, &routing, &sink),
            "claude-sonnet-4-5-20250929"
        );

        let mut latest_pro = make_agent("agent-pro");
        latest_pro.metadata.model = Some("latest:pro".to_string());
        assert_eq!(
            agent_model(&latest_pro, "any input", 10, &routing, &sink),
            "latest:pro",
            "latest:pro must NOT be routed here — only latest:auto is special-cased"
        );
    }

    #[test]
    fn test_routing_ctx_default_disables_budget_downgrade() {
        // A `RoutingCtx::default()` (used by `LlmBoardAgent::new`/`LlmRingAgent::new`
        // when no explicit routing context is supplied) must carry no budget,
        // so `route()` never downgrades regardless of remaining tokens.
        let _isolated = IsolatedModelCache::new();
        let routing = RoutingCtx::default();
        let sink: Arc<dyn EventSink> = Arc::new(NullSink);

        let agent = make_agent_latest_auto("agent-a", vec!["critical".to_string()]);
        // budget_remaining is irrelevant here since RoutingCtx::default() has
        // no total_budget — the router never receives a `BudgetState`.
        let model = agent_model(&agent, "hi", 0, &routing, &sink);
        assert_eq!(
            model,
            crate::linker::model_resolution::resolve_model_for_tier(
                "anthropic",
                crate::linker::model_resolution::ModelTier::Max,
            ),
            "no budget configured → tag-driven Max tier must not be downgraded"
        );
    }
}
