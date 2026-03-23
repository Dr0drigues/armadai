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
}
