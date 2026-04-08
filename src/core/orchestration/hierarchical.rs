//! Hierarchical orchestration engine.
//!
//! Implements the pyramid topology: coordinator → leads → agents.
//! The coordinator receives the user input, decomposes it via `@agent: task`
//! delegation directives, and the engine recursively invokes target agents.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::agent::Agent;
use crate::providers::traits::{ChatMessage, CompletionRequest, CompletionResponse, Provider};

use super::OrchestrationConfig;
use super::context_injection::{AgentInfo, build_orchestration_prompt};
use super::protocol::{DelegationAction, extract_narrative, parse_delegations};

// ── Result types ─────────────────────────────────────────────────

/// Result of a hierarchical orchestration run.
#[derive(Debug)]
pub struct OrchestrationResult {
    /// Final synthesized answer from the coordinator.
    pub content: String,
    /// All delegation events that occurred during the run.
    pub trace: Vec<DelegationEvent>,
    /// Aggregated metrics.
    pub total_tokens_in: u32,
    pub total_tokens_out: u32,
    pub total_cost: f64,
    pub invocation_count: u32,
}

/// A single delegation event in the trace.
#[derive(Debug, Clone)]
pub struct DelegationEvent {
    pub from: String,
    pub to: String,
    pub message: String,
    pub depth: u32,
}

// ── Engine ───────────────────────────────────────────────────────

/// Hierarchical orchestration engine.
///
/// Manages the recursive delegation loop between coordinator, leads, and agents.
pub struct HierarchicalEngine {
    config: OrchestrationConfig,
    agents: HashMap<String, Agent>,
    providers: HashMap<String, Arc<dyn Provider>>,
    agents_info: HashMap<String, AgentInfo>,
    /// Per-agent conversation history.
    conversations: HashMap<String, Vec<ChatMessage>>,
    /// Delegation trace for observability.
    trace: Vec<DelegationEvent>,
    /// Global iteration counter (safety).
    iteration_count: u32,
    /// Aggregated metrics.
    total_tokens_in: u32,
    total_tokens_out: u32,
    total_cost: f64,
    invocation_count: u32,
}

impl HierarchicalEngine {
    /// Create a new engine from config, agents, and their providers.
    pub fn new(
        config: OrchestrationConfig,
        agents: HashMap<String, Agent>,
        providers: HashMap<String, Arc<dyn Provider>>,
    ) -> Self {
        let agents_info = agents
            .iter()
            .map(|(name, agent)| {
                let description = agent
                    .system_prompt
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.trim().to_string());
                (
                    name.clone(),
                    AgentInfo {
                        name: name.clone(),
                        description,
                    },
                )
            })
            .collect();

        Self {
            config,
            agents,
            providers,
            agents_info,
            conversations: HashMap::new(),
            trace: Vec::new(),
            iteration_count: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost: 0.0,
            invocation_count: 0,
        }
    }

    /// Run the orchestration with the given user input.
    ///
    /// Sends the input to the coordinator, parses delegations, recursively
    /// invokes agents, and loops until a final answer or limits are reached.
    pub async fn run(&mut self, user_input: &str) -> anyhow::Result<OrchestrationResult> {
        let coordinator = self
            .config
            .coordinator
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No coordinator configured"))?;

        let result = self
            .invoke_agent(&coordinator, user_input, 0, "user")
            .await?;

        Ok(OrchestrationResult {
            content: result,
            trace: std::mem::take(&mut self.trace),
            total_tokens_in: self.total_tokens_in,
            total_tokens_out: self.total_tokens_out,
            total_cost: self.total_cost,
            invocation_count: self.invocation_count,
        })
    }

    /// Invoke a specific agent with a message, handling recursive delegations.
    ///
    /// Uses `Pin<Box<...>>` because the method is recursive and async.
    fn invoke_agent<'a>(
        &'a mut self,
        agent_name: &'a str,
        input: &'a str,
        depth: u32,
        sender: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + 'a>> {
        Box::pin(async move {
            // Safety checks
            if depth >= self.config.max_depth() {
                anyhow::bail!(
                    "Max delegation depth ({}) reached at agent '{agent_name}'",
                    self.config.max_depth()
                );
            }
            if self.iteration_count >= self.config.max_iterations() {
                anyhow::bail!("Max iterations ({}) reached", self.config.max_iterations());
            }

            // Budget checks - return partial results instead of error
            if let Some(token_budget) = self.config.token_budget {
                let total_tokens = self.total_tokens_in as u64 + self.total_tokens_out as u64;
                if total_tokens >= token_budget {
                    let partial = self.build_partial_result(
                        &format!(
                            "[Budget exceeded: used {}/{} tokens. Returning partial results.]",
                            total_tokens, token_budget
                        )
                    );
                    return Ok(partial);
                }
            }
            if let Some(cost_limit) = self.config.cost_limit
                && self.total_cost >= cost_limit
            {
                let partial = self.build_partial_result(&format!(
                    "[Cost limit exceeded: spent ${:.4}/${:.4}. Returning partial results.]",
                    self.total_cost, cost_limit
                ));
                return Ok(partial);
            }

            self.iteration_count += 1;

            // Record delegation event
            self.trace.push(DelegationEvent {
                from: sender.to_string(),
                to: agent_name.to_string(),
                message: truncate(input, 200),
                depth,
            });

            // Build enriched system prompt
            let system_prompt = self.build_enriched_prompt(agent_name);

            // Add user message to this agent's conversation
            let conversation = self
                .conversations
                .entry(agent_name.to_string())
                .or_default();
            conversation.push(ChatMessage {
                role: "user".to_string(),
                content: format_incoming_message(sender, input),
            });

            // Call the LLM
            let response = self.call_llm(agent_name, &system_prompt).await?;

            // Add assistant response to conversation
            let conversation = self
                .conversations
                .entry(agent_name.to_string())
                .or_default();
            conversation.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });

            // Parse delegation actions
            let actions = parse_delegations(&response, agent_name, &self.config);

            // If it's a final answer, return it
            if actions.len() == 1
                && let DelegationAction::FinalAnswer { ref content } = actions[0]
            {
                return Ok(content.clone());
            }

            // Process delegations
            let mut results = Vec::new();
            for action in &actions {
                match action {
                    DelegationAction::Delegate { target, task } => {
                        let result = self
                            .invoke_agent(target, task, depth + 1, agent_name)
                            .await?;
                        results.push((target.clone(), result));
                    }
                    DelegationAction::AskPeer { target, question } => {
                        let result = self
                            .invoke_agent(target, question, depth + 1, agent_name)
                            .await?;
                        results.push((target.clone(), result));
                    }
                    DelegationAction::Escalate { target, message } => {
                        let result = self
                            .invoke_agent(target, message, depth + 1, agent_name)
                            .await?;
                        results.push((target.clone(), result));
                    }
                    DelegationAction::FinalAnswer { .. } => {
                        // Should not happen in a mixed list, but handle gracefully
                    }
                }
            }

            // If no results collected (all FinalAnswer), return narrative
            if results.is_empty() {
                return Ok(extract_narrative(&response));
            }

            // Re-inject results into the agent's conversation and ask for synthesis
            let results_message = format_results(&results);
            let conversation = self
                .conversations
                .entry(agent_name.to_string())
                .or_default();
            conversation.push(ChatMessage {
                role: "user".to_string(),
                content: results_message,
            });

            // Call the agent again for synthesis
            let synthesis = self.call_llm(agent_name, &system_prompt).await?;

            // Add synthesis to conversation
            let conversation = self
                .conversations
                .entry(agent_name.to_string())
                .or_default();
            conversation.push(ChatMessage {
                role: "assistant".to_string(),
                content: synthesis.clone(),
            });

            // Check if synthesis contains more delegations (recursive)
            let synth_actions = parse_delegations(&synthesis, agent_name, &self.config);
            if synth_actions.len() == 1
                && let DelegationAction::FinalAnswer { ref content } = synth_actions[0]
            {
                return Ok(content.clone());
            }

            // For safety, just return the synthesis text to avoid infinite loops
            Ok(extract_narrative(&synthesis))
        })
    }

    /// Build the enriched system prompt for an agent (original + orchestration context).
    fn build_enriched_prompt(&self, agent_name: &str) -> String {
        let base_prompt = self
            .agents
            .get(agent_name)
            .map(|a| a.system_prompt.as_str())
            .unwrap_or("You are a helpful assistant.");

        let orchestration_block =
            build_orchestration_prompt(agent_name, &self.config, &self.agents_info);

        match orchestration_block {
            Some(block) => format!("{base_prompt}{block}"),
            None => base_prompt.to_string(),
        }
    }

    /// Build a partial result when budget is exceeded.
    /// Collects the last assistant message from each agent's conversation.
    fn build_partial_result(&self, budget_message: &str) -> String {
        let mut result = String::from(budget_message);
        result.push_str("\n\n");

        for (agent_name, conversation) in &self.conversations {
            if let Some(last_msg) = conversation.iter().rev().find(|m| m.role == "assistant") {
                result.push_str(&format!(
                    "[Partial from @{agent_name}]\n{}\n\n",
                    truncate(&last_msg.content, 500)
                ));
            }
        }

        if result.len() <= budget_message.len() + 2 {
            result.push_str("[No partial results available yet.]");
        }

        result
    }

    /// Call the LLM for a specific agent using its conversation history.
    async fn call_llm(&mut self, agent_name: &str, system_prompt: &str) -> anyhow::Result<String> {
        let provider = self
            .providers
            .get(agent_name)
            .ok_or_else(|| anyhow::anyhow!("No provider found for agent '{agent_name}'"))?;

        let agent = self
            .agents
            .get(agent_name)
            .ok_or_else(|| anyhow::anyhow!("Agent '{agent_name}' not found"))?;

        let messages = self
            .conversations
            .get(agent_name)
            .cloned()
            .unwrap_or_default();

        let model = agent
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "default".to_string());

        let request = CompletionRequest {
            model,
            system_prompt: system_prompt.to_string(),
            messages,
            temperature: agent.metadata.temperature,
            max_tokens: agent.metadata.max_tokens,
        };

        let response: CompletionResponse = provider.complete(request).await?;

        // Aggregate metrics
        self.total_tokens_in += response.tokens_in;
        self.total_tokens_out += response.tokens_out;
        self.total_cost += response.cost;
        self.invocation_count += 1;

        Ok(response.content)
    }
}

// ── Helpers ──────────────────────────────────────────────────────

/// Format an incoming message with sender attribution.
fn format_incoming_message(sender: &str, content: &str) -> String {
    if sender == "user" {
        content.to_string()
    } else {
        format!("[Message from @{sender}]\n{content}")
    }
}

/// Format collected results for re-injection into an agent's conversation.
fn format_results(results: &[(String, String)]) -> String {
    let mut out = String::new();
    for (agent_name, result) in results {
        out.push_str(&format!(
            "[Result from @{agent_name}]\n{result}\n[End result from @{agent_name}]\n\n"
        ));
    }
    out
}

/// Truncate a string for trace display.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::orchestration::TeamConfig;
    use crate::providers::traits::{CompletionResponse, ProviderMetadata, TokenStream};
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// A mock provider that returns scripted responses in order.
    struct MockProvider {
        responses: Mutex<Vec<String>>,
    }

    impl MockProvider {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(|s| s.to_string()).collect()),
            }
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> anyhow::Result<CompletionResponse> {
            let mut responses = self.responses.lock().unwrap();
            let content = if responses.is_empty() {
                "No more scripted responses.".to_string()
            } else {
                responses.remove(0)
            };
            Ok(CompletionResponse {
                content,
                model: "mock".to_string(),
                tokens_in: 10,
                tokens_out: 20,
                cost: 0.001,
            })
        }

        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not supported in mock")
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "mock".to_string(),
                models: vec!["mock".to_string()],
                supports_streaming: false,
            }
        }
    }

    fn make_agent(name: &str, prompt: &str) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: crate::core::agent::AgentMetadata {
                provider: "mock".to_string(),
                model: Some("mock".to_string()),
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
            system_prompt: prompt.to_string(),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    fn sample_config() -> OrchestrationConfig {
        OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["agent-a".to_string(), "agent-b".to_string()],
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_direct_answer_no_delegation() {
        let config = sample_config();

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec!["The answer is 42."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("What is the answer?").await.unwrap();

        assert_eq!(result.content, "The answer is 42.");
        assert_eq!(result.invocation_count, 1);
        assert_eq!(result.trace.len(), 1);
    }

    #[tokio::test]
    async fn test_single_delegation() {
        let config = sample_config();

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        // Coordinator delegates to agent-a, then synthesizes
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: do the task",       // first call: delegate
                "Final synthesis from coord.", // second call: synthesize after results
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Result from agent A."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do something").await.unwrap();

        assert_eq!(result.content, "Final synthesis from coord.");
        assert_eq!(result.invocation_count, 3); // coord + agent-a + coord synthesis
        assert!(result.trace.len() >= 2);
    }

    #[tokio::test]
    async fn test_multiple_delegations() {
        let config = sample_config();

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: do task A\n@agent-b: do task B",
                "Combined result from both agents.",
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["A done."])),
        );
        providers.insert(
            "agent-b".to_string(),
            Arc::new(MockProvider::new(vec!["B done."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do both tasks").await.unwrap();

        assert_eq!(result.content, "Combined result from both agents.");
        assert_eq!(result.invocation_count, 4); // coord + a + b + coord synthesis
    }

    #[tokio::test]
    async fn test_max_depth_protection() {
        // Create a config with max_depth = 2
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: Some("lead".to_string()),
                agents: vec!["worker".to_string()],
            }],
            max_depth: Some(2),
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("lead".to_string(), make_agent("lead", "You lead."));
        agents.insert("worker".to_string(), make_agent("worker", "You work."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        // Each agent delegates deeper — should hit max_depth
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec!["@lead: do it"])),
        );
        providers.insert(
            "lead".to_string(),
            Arc::new(MockProvider::new(vec!["@worker: do it"])),
        );
        providers.insert(
            "worker".to_string(),
            Arc::new(MockProvider::new(vec!["done"])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let err = engine.run("deep task").await.unwrap_err();
        assert!(err.to_string().contains("Max delegation depth"));
    }

    #[tokio::test]
    async fn test_max_iterations_protection() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["agent-a".to_string()],
            }],
            max_iterations: Some(1),
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        // Coordinator delegates — second invoke_agent call should hit max_iterations=1
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec!["@agent-a: task 1"])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["done 1"])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let err = engine.run("keep going").await.unwrap_err();
        assert!(err.to_string().contains("Max iterations"));
    }

    #[tokio::test]
    async fn test_metrics_aggregation() {
        let config = sample_config();

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec!["Direct answer."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Simple question").await.unwrap();

        assert_eq!(result.total_tokens_in, 10);
        assert_eq!(result.total_tokens_out, 20);
        assert!((result.total_cost - 0.001).abs() < f64::EPSILON);
    }

    #[test]
    fn test_format_results() {
        let results = vec![
            ("agent-a".to_string(), "result A".to_string()),
            ("agent-b".to_string(), "result B".to_string()),
        ];
        let formatted = format_results(&results);
        assert!(formatted.contains("[Result from @agent-a]"));
        assert!(formatted.contains("result A"));
        assert!(formatted.contains("[End result from @agent-a]"));
        assert!(formatted.contains("[Result from @agent-b]"));
    }

    #[test]
    fn test_format_incoming_message_user() {
        assert_eq!(format_incoming_message("user", "hello"), "hello");
    }

    #[test]
    fn test_format_incoming_message_agent() {
        let msg = format_incoming_message("lead", "do this");
        assert!(msg.contains("[Message from @lead]"));
        assert!(msg.contains("do this"));
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a long string here", 10), "a long str...");
    }

    #[tokio::test]
    async fn test_token_budget_enforcement() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["agent-a".to_string(), "agent-b".to_string()],
            }],
            token_budget: Some(55), // Budget: will exceed when trying to call second agent
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        // Each call consumes 10 (in) + 20 (out) = 30 tokens
        // Coordinator: 30, agent-a: 30 (total 60, exceeds 55)
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: task A\n@agent-b: task B",  // First call: 30 tokens
                "Final synthesis.",                     // Won't reach here
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Result A."])),
        );
        providers.insert(
            "agent-b".to_string(),
            Arc::new(MockProvider::new(vec!["Result B."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do both tasks").await.unwrap();

        // Budget should have stopped execution early
        // We expect: coordinator (30) + agent-a (30) + maybe agent-b budget check returns partial
        // Total should be around 60 tokens (first agent finished, second denied due to budget)
        eprintln!("Result content: {}", result.content);
        let total_tokens = result.total_tokens_in as u64 + result.total_tokens_out as u64;
        eprintln!("Total tokens: {}", total_tokens);

        // Should have stopped before processing all agents due to budget
        // The total should be close to the budget limit (55) but might slightly exceed
        // due to the last call that triggered the limit
        assert!((55..=100).contains(&total_tokens), "Expected tokens to be around budget limit");

        // At least one invocation should have been prevented by budget
        // (would be 4 calls without budget: coord + agent-a + agent-b + coord synthesis)
        assert!(result.invocation_count < 4, "Budget should have prevented all delegations");
    }

    #[tokio::test]
    async fn test_cost_limit_enforcement() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["agent-a".to_string(), "agent-b".to_string()],
            }],
            cost_limit: Some(0.0015), // Tiny cost limit: will exceed after 2 calls (each 0.001)
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        // Each call costs 0.001, limit is 0.0015
        // Coordinator: 0.001, agent-a: 0.001 (total 0.002, exceeds 0.0015)
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: task A\n@agent-b: task B",
                "Final synthesis.",
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Result A."])),
        );
        providers.insert(
            "agent-b".to_string(),
            Arc::new(MockProvider::new(vec!["Result B."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do something").await.unwrap();

        // Cost limit should have stopped execution early
        assert!(result.total_cost >= 0.0015, "Should have spent at least the limit");
        assert!(
            result.invocation_count < 4,
            "Cost limit should have prevented all delegations"
        );
    }

    #[tokio::test]
    async fn test_no_budget_limit() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["agent-a".to_string()],
            }],
            token_budget: None, // No limit
            cost_limit: None,   // No limit
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: task 1",
                "Final synthesis.",
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Result A."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do something").await.unwrap();

        // Should complete normally without budget warnings
        assert!(!result.content.contains("Budget exceeded"));
        assert!(!result.content.contains("Cost limit exceeded"));
        assert_eq!(result.content, "Final synthesis.");
    }

    #[tokio::test]
    async fn test_budget_returns_partial_not_error() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["agent-a".to_string(), "agent-b".to_string()],
            }],
            token_budget: Some(50), // Will exceed mid-execution
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: task A\n@agent-b: task B",
                "Combined result.",
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Done A."])),
        );
        providers.insert(
            "agent-b".to_string(),
            Arc::new(MockProvider::new(vec!["Done B."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do both tasks").await;

        // Should return Ok with partial results, NOT an error
        assert!(result.is_ok(), "Budget limit should return Ok, not Err");
        let result = result.unwrap();

        // The key is that it returns Ok (graceful degradation) even when budget is hit
        // We don't care about the exact content, just that it didn't error
        assert!(result.invocation_count > 0, "Should have made at least one call");
    }
}

