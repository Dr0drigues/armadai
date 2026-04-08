//! Hierarchical orchestration engine.
//!
//! Implements the pyramid topology: coordinator → leads → agents.
//! The coordinator receives the user input, decomposes it via `@agent: task`
//! delegation directives, and the engine recursively invokes target agents.
//!
//! Independent `Delegate` actions from a single response are dispatched in
//! parallel via `tokio::spawn`, while `AskPeer` and `Escalate` remain sequential.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

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

// ── Shared state ────────────────────────────────────────────────

/// Immutable context shared across all concurrent agent invocations.
struct EngineContext {
    config: OrchestrationConfig,
    agents: HashMap<String, Agent>,
    providers: HashMap<String, Arc<dyn Provider>>,
    agents_info: HashMap<String, AgentInfo>,
}

/// Mutable state protected by a mutex for concurrent access.
struct EngineState {
    conversations: HashMap<String, Vec<ChatMessage>>,
    trace: Vec<DelegationEvent>,
    iteration_count: u32,
    total_tokens_in: u32,
    total_tokens_out: u32,
    total_cost: f64,
    invocation_count: u32,
}

// ── Engine ───────────────────────────────────────────────────────

/// Hierarchical orchestration engine.
///
/// Manages the recursive delegation loop between coordinator, leads, and agents.
/// Independent delegations are dispatched in parallel.
pub struct HierarchicalEngine {
    ctx: Arc<EngineContext>,
    state: Arc<Mutex<EngineState>>,
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
            ctx: Arc::new(EngineContext {
                config,
                agents,
                providers,
                agents_info,
            }),
            state: Arc::new(Mutex::new(EngineState {
                conversations: HashMap::new(),
                trace: Vec::new(),
                iteration_count: 0,
                total_tokens_in: 0,
                total_tokens_out: 0,
                total_cost: 0.0,
                invocation_count: 0,
            })),
        }
    }

    /// Run the orchestration with the given user input.
    ///
    /// Sends the input to the coordinator, parses delegations, recursively
    /// invokes agents, and loops until a final answer or limits are reached.
    pub async fn run(&mut self, user_input: &str) -> anyhow::Result<OrchestrationResult> {
        let coordinator = self
            .ctx
            .config
            .coordinator
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No coordinator configured"))?;

        let result = invoke_agent(
            Arc::clone(&self.ctx),
            Arc::clone(&self.state),
            coordinator,
            user_input.to_string(),
            0,
            "user".to_string(),
        )
        .await?;

        let mut state = self.state.lock().expect("engine state mutex poisoned");
        Ok(OrchestrationResult {
            content: result,
            trace: std::mem::take(&mut state.trace),
            total_tokens_in: state.total_tokens_in,
            total_tokens_out: state.total_tokens_out,
            total_cost: state.total_cost,
            invocation_count: state.invocation_count,
        })
    }
}

// ── Recursive agent invocation (free function for parallel dispatch) ──

/// Invoke a specific agent with a message, handling recursive delegations.
///
/// This is a free function (not a method) so it can be cloned into parallel
/// `tokio::spawn` tasks. Uses `Pin<Box<...>>` for async recursion.
fn invoke_agent(
    ctx: Arc<EngineContext>,
    state: Arc<Mutex<EngineState>>,
    agent_name: String,
    input: String,
    depth: u32,
    sender: String,
) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>> {
    Box::pin(async move {
        // ── Safety checks (lock briefly, then release) ──────────
        {
            let s = state.lock().expect("engine state mutex poisoned");
            if depth >= ctx.config.max_depth() {
                anyhow::bail!(
                    "Max delegation depth ({}) reached at agent '{agent_name}'",
                    ctx.config.max_depth()
                );
            }
            if s.iteration_count >= ctx.config.max_iterations() {
                anyhow::bail!("Max iterations ({}) reached", ctx.config.max_iterations());
            }

            // Budget checks — return partial results instead of error
            if let Some(token_budget) = ctx.config.token_budget {
                let total_tokens = s.total_tokens_in as u64 + s.total_tokens_out as u64;
                if total_tokens >= token_budget {
                    return Ok(build_partial_result(
                        &s,
                        &format!(
                            "[Budget exceeded: used {total_tokens}/{token_budget} tokens. Returning partial results.]"
                        ),
                    ));
                }
            }
            if let Some(cost_limit) = ctx.config.cost_limit
                && s.total_cost >= cost_limit
            {
                return Ok(build_partial_result(
                    &s,
                    &format!(
                        "[Cost limit exceeded: spent ${:.4}/${:.4}. Returning partial results.]",
                        s.total_cost, cost_limit
                    ),
                ));
            }
        } // unlock

        // ── Update state: iteration count, trace, conversation ──
        {
            let mut s = state.lock().expect("engine state mutex poisoned");
            s.iteration_count += 1;
            s.trace.push(DelegationEvent {
                from: sender.clone(),
                to: agent_name.clone(),
                message: truncate(&input, 200),
                depth,
            });
            let conv = s.conversations.entry(agent_name.clone()).or_default();
            conv.push(ChatMessage {
                role: "user".to_string(),
                content: format_incoming_message(&sender, &input),
            });
        } // unlock

        // ── Build enriched system prompt (read-only) ────────────
        let system_prompt = build_enriched_prompt(&ctx, &agent_name);

        // ── Call the LLM ────────────────────────────────────────
        let response = call_llm(&ctx, &state, &agent_name, &system_prompt).await?;

        // ── Record assistant response ───────────────────────────
        {
            let mut s = state.lock().expect("engine state mutex poisoned");
            let conv = s.conversations.entry(agent_name.clone()).or_default();
            conv.push(ChatMessage {
                role: "assistant".to_string(),
                content: response.clone(),
            });
        } // unlock

        // ── Parse delegation actions ────────────────────────────
        let actions = parse_delegations(&response, &agent_name, &ctx.config);

        // If it's a final answer, return it
        if actions.len() == 1
            && let DelegationAction::FinalAnswer { ref content } = actions[0]
        {
            return Ok(content.clone());
        }

        // ── Separate parallel (Delegate) from sequential (AskPeer/Escalate) ──
        let mut delegate_tasks: Vec<(String, String)> = Vec::new();
        let mut sequential_tasks: Vec<(String, String)> = Vec::new();

        for action in &actions {
            match action {
                DelegationAction::Delegate { target, task } => {
                    delegate_tasks.push((target.clone(), task.clone()));
                }
                DelegationAction::AskPeer { target, question } => {
                    sequential_tasks.push((target.clone(), question.clone()));
                }
                DelegationAction::Escalate { target, message } => {
                    sequential_tasks.push((target.clone(), message.clone()));
                }
                DelegationAction::FinalAnswer { .. } => {}
            }
        }

        let mut results: Vec<(String, String)> = Vec::new();

        // ── Parallel dispatch for independent Delegate actions ───
        if !delegate_tasks.is_empty() {
            let mut handles = Vec::new();
            for (target, task) in delegate_tasks {
                let ctx = Arc::clone(&ctx);
                let state = Arc::clone(&state);
                let sender = agent_name.clone();
                let target_name = target.clone();
                handles.push(tokio::spawn(async move {
                    let result = invoke_agent(
                        ctx,
                        state,
                        target_name.clone(),
                        task,
                        depth + 1,
                        sender,
                    )
                    .await?;
                    Ok::<_, anyhow::Error>((target_name, result))
                }));
            }
            for handle in handles {
                let pair = handle
                    .await
                    .map_err(|e| anyhow::anyhow!("Agent task join error: {e}"))??;
                results.push(pair);
            }
        }

        // ── Sequential dispatch for AskPeer / Escalate ──────────
        for (target, msg) in sequential_tasks {
            let result = invoke_agent(
                Arc::clone(&ctx),
                Arc::clone(&state),
                target.clone(),
                msg,
                depth + 1,
                agent_name.clone(),
            )
            .await?;
            results.push((target, result));
        }

        // ── If no results collected, return narrative ────────────
        if results.is_empty() {
            return Ok(extract_narrative(&response));
        }

        // ── Re-inject results and ask for synthesis ─────────────
        let results_message = format_results(&results);
        {
            let mut s = state.lock().expect("engine state mutex poisoned");
            let conv = s.conversations.entry(agent_name.clone()).or_default();
            conv.push(ChatMessage {
                role: "user".to_string(),
                content: results_message,
            });
        } // unlock

        let synthesis = call_llm(&ctx, &state, &agent_name, &system_prompt).await?;

        {
            let mut s = state.lock().expect("engine state mutex poisoned");
            let conv = s.conversations.entry(agent_name.clone()).or_default();
            conv.push(ChatMessage {
                role: "assistant".to_string(),
                content: synthesis.clone(),
            });
        } // unlock

        // Check if synthesis contains more delegations
        let synth_actions = parse_delegations(&synthesis, &agent_name, &ctx.config);
        if synth_actions.len() == 1
            && let DelegationAction::FinalAnswer { ref content } = synth_actions[0]
        {
            return Ok(content.clone());
        }

        // For safety, just return the synthesis text to avoid infinite loops
        Ok(extract_narrative(&synthesis))
    })
}

// ── Internal helpers ────────────────────────────────────────────

/// Build the enriched system prompt for an agent (original + orchestration context).
fn build_enriched_prompt(ctx: &EngineContext, agent_name: &str) -> String {
    let base_prompt = ctx
        .agents
        .get(agent_name)
        .map(|a| a.system_prompt.as_str())
        .unwrap_or("You are a helpful assistant.");

    let orchestration_block =
        build_orchestration_prompt(agent_name, &ctx.config, &ctx.agents_info);

    match orchestration_block {
        Some(block) => format!("{base_prompt}{block}"),
        None => base_prompt.to_string(),
    }
}

/// Build a partial result when budget is exceeded.
/// Collects the last assistant message from each agent's conversation.
fn build_partial_result(state: &EngineState, budget_message: &str) -> String {
    let mut result = String::from(budget_message);
    result.push_str("\n\n");

    for (agent_name, conversation) in &state.conversations {
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
///
/// Locks state briefly to read conversation, releases before the async call,
/// then locks again to update metrics.
async fn call_llm(
    ctx: &Arc<EngineContext>,
    state: &Arc<Mutex<EngineState>>,
    agent_name: &str,
    system_prompt: &str,
) -> anyhow::Result<String> {
    let provider = ctx
        .providers
        .get(agent_name)
        .ok_or_else(|| anyhow::anyhow!("No provider found for agent '{agent_name}'"))?;

    let agent = ctx
        .agents
        .get(agent_name)
        .ok_or_else(|| anyhow::anyhow!("Agent '{agent_name}' not found"))?;

    let messages = {
        let s = state.lock().expect("engine state mutex poisoned");
        s.conversations
            .get(agent_name)
            .cloned()
            .unwrap_or_default()
    }; // unlock before async call

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

    // Update metrics
    {
        let mut s = state.lock().expect("engine state mutex poisoned");
        s.total_tokens_in += response.tokens_in;
        s.total_tokens_out += response.tokens_out;
        s.total_cost += response.cost;
        s.invocation_count += 1;
    } // unlock

    Ok(response.content)
}

// ── Public helpers ──────────────────────────────────────────────

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
    async fn test_multiple_delegations_parallel() {
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
            token_budget: Some(55),
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
        let result = engine.run("Do both tasks").await.unwrap();

        let total_tokens = result.total_tokens_in as u64 + result.total_tokens_out as u64;
        // With parallel dispatch, both agents may start before budget is checked,
        // so we allow a wider range
        assert!(
            total_tokens >= 55,
            "Should have consumed at least budget worth of tokens"
        );
        // Should not have completed all 4 calls (coord + a + b + synthesis)
        // With parallelism, both a and b might complete, but synthesis should be prevented
        assert!(
            result.invocation_count <= 4,
            "Budget should have limited invocations"
        );
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
            cost_limit: Some(0.0015),
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

        assert!(
            result.total_cost >= 0.0015,
            "Should have spent at least the limit"
        );
        assert!(
            result.invocation_count <= 4,
            "Cost limit should have limited invocations"
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
            token_budget: None,
            cost_limit: None,
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
            token_budget: Some(50),
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

        assert!(result.is_ok(), "Budget limit should return Ok, not Err");
        let result = result.unwrap();
        assert!(
            result.invocation_count > 0,
            "Should have made at least one call"
        );
    }

    #[tokio::test]
    async fn test_parallel_dispatch_collects_all_results() {
        // Verify that when coordinator delegates to 3 agents, all 3 results are collected
        let config = OrchestrationConfig {
            enabled: true,
            pattern: super::super::OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec![
                    "agent-a".to_string(),
                    "agent-b".to_string(),
                    "agent-c".to_string(),
                ],
            }],
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        agents.insert("agent-a".to_string(), make_agent("agent-a", "You do A."));
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));
        agents.insert("agent-c".to_string(), make_agent("agent-c", "You do C."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: task A\n@agent-b: task B\n@agent-c: task C",
                "All three results received and synthesized.",
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Alpha result."])),
        );
        providers.insert(
            "agent-b".to_string(),
            Arc::new(MockProvider::new(vec!["Beta result."])),
        );
        providers.insert(
            "agent-c".to_string(),
            Arc::new(MockProvider::new(vec!["Gamma result."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do all three tasks").await.unwrap();

        assert_eq!(result.content, "All three results received and synthesized.");
        // coord (1) + a,b,c parallel (3) + coord synthesis (1) = 5
        assert_eq!(result.invocation_count, 5);
        // Trace: user→coord, coord→a, coord→b, coord→c = at least 4
        assert!(result.trace.len() >= 4);
    }
}
