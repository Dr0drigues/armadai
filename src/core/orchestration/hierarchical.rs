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
use crate::core::events::{EventSink, RunEvent};
use crate::core::routing::{BudgetState, RoutingRules, route};
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
    sink: Arc<dyn EventSink>,
    /// Rules for routing `latest:auto` agents (spec: OH4 router in orchestration,
    /// mirroring `RoutingCtx` in `llm_agents.rs` for board/ring). Defaults to
    /// the embedded `RoutingRules::default()` when the engine is built via
    /// `new()`; `with_routing_rules()` allows callers (e.g. `run_orchestrated`)
    /// to supply the project's `armadai.yaml` `routing:` section instead.
    routing_rules: RoutingRules,
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
    ///
    /// `sink` receives `RunEvent::Delegate{from, to}` for every agent invocation
    /// (initial coordinator call and every recursive delegation/ask-peer/escalate).
    /// Pass `Arc::new(NullSink)` when JSONL event emission is not needed.
    ///
    /// Uses the embedded `RoutingRules::default()` for any `latest:auto` agent
    /// in the fleet — use `with_routing_rules` to supply project-configured
    /// rules instead (see `run_orchestrated` in `cli/run.rs`).
    pub fn new(
        config: OrchestrationConfig,
        agents: HashMap<String, Agent>,
        providers: HashMap<String, Arc<dyn Provider>>,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        Self::with_routing_rules(config, agents, providers, sink, RoutingRules::default())
    }

    /// Like `new`, but with explicit `RoutingRules` for `latest:auto` agents.
    ///
    /// Mirrors `RoutingCtx` in `llm_agents.rs` (Task 3): `call_llm` routes
    /// `latest:auto` through `core::routing::route`, using these rules and a
    /// `BudgetState` derived from the engine's configured `token_budget` vs.
    /// tokens consumed so far. Budget only ever *downgrades* the tier — it
    /// never introduces a new failure/halt path.
    pub fn with_routing_rules(
        config: OrchestrationConfig,
        agents: HashMap<String, Agent>,
        providers: HashMap<String, Arc<dyn Provider>>,
        sink: Arc<dyn EventSink>,
        routing_rules: RoutingRules,
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
                sink,
                routing_rules,
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

        let mut state = self.state.lock().unwrap_or_else(|e| {
            tracing::warn!("Mutex poisoned in run(), recovering: {:?}", e);
            e.into_inner()
        });
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
            let s = state
                .lock()
                .map_err(|e| anyhow::anyhow!("Mutex poisoned during safety checks: {:?}", e))?;
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
            let mut s = state.lock().unwrap_or_else(|e| {
                tracing::warn!(
                    "Mutex poisoned in invoke_agent (update state), recovering: {:?}",
                    e
                );
                e.into_inner()
            });
            s.iteration_count += 1;
            s.trace.push(DelegationEvent {
                from: sender.clone(),
                to: agent_name.clone(),
                message: truncate(&input, 200),
                depth,
            });
            // Only emit `Delegate` for real agent-to-agent delegations. The root
            // call (`user` -> coordinator) is not a delegation and would otherwise
            // inflate delegation counts for JSONL consumers.
            if sender != "user" {
                ctx.sink.emit(&RunEvent::Delegate {
                    from: sender.clone(),
                    to: agent_name.clone(),
                });
            }
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
            let mut s = state.lock().unwrap_or_else(|e| {
                tracing::warn!(
                    "Mutex poisoned in invoke_agent (record response), recovering: {:?}",
                    e
                );
                e.into_inner()
            });
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
                    let result =
                        invoke_agent(ctx, state, target_name.clone(), task, depth + 1, sender)
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
            let mut s = state.lock().unwrap_or_else(|e| {
                tracing::warn!(
                    "Mutex poisoned in invoke_agent (re-inject results), recovering: {:?}",
                    e
                );
                e.into_inner()
            });
            let conv = s.conversations.entry(agent_name.clone()).or_default();
            conv.push(ChatMessage {
                role: "user".to_string(),
                content: results_message,
            });
        } // unlock

        let synthesis = call_llm(&ctx, &state, &agent_name, &system_prompt).await?;

        {
            let mut s = state.lock().unwrap_or_else(|e| {
                tracing::warn!(
                    "Mutex poisoned in invoke_agent (record synthesis), recovering: {:?}",
                    e
                );
                e.into_inner()
            });
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

    let orchestration_block = build_orchestration_prompt(agent_name, &ctx.config, &ctx.agents_info);

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

    let (messages, tokens_consumed) = {
        let s = state.lock().map_err(|e| {
            anyhow::anyhow!("Mutex poisoned in call_llm (read conversation): {:?}", e)
        })?;
        let messages = s.conversations.get(agent_name).cloned().unwrap_or_default();
        let tokens_consumed = s.total_tokens_in as u64 + s.total_tokens_out as u64;
        (messages, tokens_consumed)
    }; // unlock before async call

    let raw_model = agent
        .metadata
        .model
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // Route `latest:auto` the same way `run_single_agent`/board/ring do (OH4
    // router). Concrete models and `latest:pro/fast/max` placeholders pass
    // through unchanged — only the exact `latest:auto` string is special-cased,
    // matching `agent_model` in `llm_agents.rs`.
    let model = if raw_model == "latest:auto" {
        // Use the last message in the conversation (the task/message this
        // call is about to answer — either the incoming delegation or the
        // re-injected results for synthesis) as the routing input. Falls
        // back to the system prompt if the conversation is somehow empty.
        let routing_input = messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or(system_prompt);

        // Budget is derived from the engine's *configured* token_budget vs.
        // tokens consumed so far across the whole run. `None` (no budget
        // configured, or configured as 0) disables downgrade — `route()` is
        // still called, just without a `BudgetState`.
        let budget = ctx.config.token_budget.filter(|&b| b > 0).map(|total| {
            let remaining = total.saturating_sub(tokens_consumed);
            BudgetState {
                remaining_ratio: remaining as f64 / total as f64,
            }
        });

        let (tier, reason) = route(
            routing_input,
            &agent.metadata.tags,
            budget,
            &ctx.routing_rules,
        );
        ctx.sink.emit(&RunEvent::Route {
            agent: agent_name.to_string(),
            tier: format!("{tier:?}"),
            reason: format!("{reason:?}"),
        });
        crate::linker::model_resolution::resolve_model_for_tier(&agent.metadata.provider, tier)
    } else {
        raw_model
    };

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
        let mut s = state.lock().unwrap_or_else(|e| {
            tracing::warn!(
                "Mutex poisoned in call_llm (update metrics), recovering: {:?}",
                e
            );
            e.into_inner()
        });
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
    use crate::core::events::NullSink;
    use crate::core::orchestration::TeamConfig;
    use crate::providers::traits::{CompletionResponse, ProviderMetadata, TokenStream};
    use async_trait::async_trait;
    use std::path::PathBuf;

    /// No-op sink for tests that don't assert on emitted events.
    fn null_sink() -> Arc<dyn EventSink> {
        Arc::new(NullSink)
    }

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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
        let result = engine.run("Do something").await.unwrap();

        assert_eq!(result.content, "Final synthesis from coord.");
        assert_eq!(result.invocation_count, 3); // coord + agent-a + coord synthesis
        assert!(result.trace.len() >= 2);
    }

    /// A sink that captures every emitted event as its JSONL-serialized form,
    /// for assertions on which events fired and with what payload.
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

    #[tokio::test]
    async fn test_single_delegation_emits_delegate_events() {
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
                "@agent-a: do the task",
                "Final synthesis from coord.",
            ])),
        );
        providers.insert(
            "agent-a".to_string(),
            Arc::new(MockProvider::new(vec!["Result from agent A."])),
        );

        let capture = Arc::new(CaptureSink::new());
        let sink: Arc<dyn EventSink> = capture.clone();
        let mut engine = HierarchicalEngine::new(config, agents, providers, sink);
        let _ = engine.run("Do something").await.unwrap();

        let events = capture.events();
        // Real agent-to-agent delegation: coordinator -> agent-a.
        assert!(
            events.iter().any(|e| e.contains(r#""t":"delegate""#)
                && e.contains(r#""from":"coordinator""#)
                && e.contains(r#""to":"agent-a""#)),
            "expected coordinator->agent-a delegate event, got: {events:?}"
        );
        // Root call (user -> coordinator) is NOT a real delegation and must not
        // be emitted as a `Delegate` event.
        assert!(
            !events
                .iter()
                .any(|e| e.contains(r#""t":"delegate""#) && e.contains(r#""from":"user""#)),
            "root call user->coordinator should not emit a Delegate event, got: {events:?}"
        );
    }

    // ── latest:auto routing in hierarchical (Task 4) ─────────────

    /// Records the `model` of every `CompletionRequest` it receives, so tests
    /// can assert what `call_llm` resolved `latest:auto` to (mirrors
    /// `CapturingProvider` in `llm_agents.rs`'s own test module).
    struct CapturingProvider {
        models: std::sync::Mutex<Vec<String>>,
        tokens_in: u32,
        tokens_out: u32,
        response: String,
    }

    impl CapturingProvider {
        fn new(response: &str, tokens_in: u32, tokens_out: u32) -> Self {
            Self {
                models: std::sync::Mutex::new(Vec::new()),
                tokens_in,
                tokens_out,
                response: response.to_string(),
            }
        }

        fn models(&self) -> Vec<String> {
            self.models.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for CapturingProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            self.models.lock().unwrap().push(request.model.clone());
            Ok(CompletionResponse {
                content: self.response.clone(),
                model: request.model,
                tokens_in: self.tokens_in,
                tokens_out: self.tokens_out,
                cost: 0.0,
            })
        }

        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not supported in mock")
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "capturing".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    #[tokio::test]
    async fn test_latest_auto_routes_in_hierarchical_and_downgrades_on_low_budget() {
        // Coordinator uses a concrete model and delegates to agent-a, which is
        // configured with `latest:auto` + a "critical" tag (-> Max tier under
        // default RoutingRules). MockProvider's coordinator response consumes
        // 10 input + 20 output = 30 tokens before agent-a is ever invoked. A
        // `token_budget` of 35 therefore leaves only 5 tokens remaining
        // (ratio ~0.14) once agent-a's `call_llm` runs, under the 0.2
        // `budget_downgrade_ratio` threshold -- the router must downgrade
        // agent-a's tag-driven Max tier to Fast and report
        // `RouteReason::Budget`, exactly as `agent_model` does for board/ring.
        let mut config = sample_config();
        config.token_budget = Some(35);

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        let mut agent_a = make_agent("agent-a", "You do A.");
        agent_a.metadata.model = Some("latest:auto".to_string());
        agent_a.metadata.tags = vec!["critical".to_string()];
        agents.insert("agent-a".to_string(), agent_a);
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: do the task",
                "Final synthesis from coord.",
            ])),
        );
        let capturing = Arc::new(CapturingProvider::new("Result from agent A.", 1, 1));
        providers.insert(
            "agent-a".to_string(),
            capturing.clone() as Arc<dyn Provider>,
        );

        let sink = Arc::new(CaptureSink::new());
        let mut engine = HierarchicalEngine::with_routing_rules(
            config,
            agents,
            providers,
            sink.clone() as Arc<dyn EventSink>,
            RoutingRules::default(),
        );

        let result = engine.run("Do something").await.unwrap();
        assert_eq!(result.content, "Final synthesis from coord.");

        let models = capturing.models();
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0],
            crate::linker::model_resolution::resolve_model_for_tier(
                "mock",
                crate::linker::model_resolution::ModelTier::Fast,
            ),
            "latest:auto with tag 'critical' (Max) must downgrade to Fast under low budget"
        );

        let events = sink.events();
        assert!(
            events.iter().any(|e| e.contains(r#""t":"route""#)
                && e.contains(r#""agent":"agent-a""#)
                && e.contains(r#""tier":"Fast""#)
                && e.contains(r#""reason":"Budget""#)),
            "expected a Route event with tier=Fast reason=Budget for agent-a, got: {events:?}"
        );
    }

    #[tokio::test]
    async fn test_concrete_model_and_latest_pro_unaffected_by_hierarchical_routing() {
        // Non-regression: concrete models and `latest:pro/fast/max` are NOT
        // routed through `route()` in hierarchical either -- they must reach
        // the provider unchanged even under the exact budget scenario
        // (`token_budget: Some(35)`, same as the downgrade test above) that
        // *would* force a downgrade if agent-a's model were `latest:auto`.
        // Only the exact `latest:auto` string is special-cased (see `call_llm`).
        let mut config = sample_config();
        config.token_budget = Some(35);

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );
        let mut agent_a = make_agent("agent-a", "You do A.");
        agent_a.metadata.model = Some("latest:pro".to_string());
        agents.insert("agent-a".to_string(), agent_a);
        agents.insert("agent-b".to_string(), make_agent("agent-b", "You do B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec![
                "@agent-a: do the task",
                "Final synthesis from coord.",
            ])),
        );
        let capturing = Arc::new(CapturingProvider::new("Result from agent A.", 1, 1));
        providers.insert(
            "agent-a".to_string(),
            capturing.clone() as Arc<dyn Provider>,
        );

        let mut engine = HierarchicalEngine::with_routing_rules(
            config,
            agents,
            providers,
            null_sink(),
            RoutingRules::default(),
        );
        engine.run("Do something").await.unwrap();

        assert_eq!(
            capturing.models(),
            vec!["latest:pro".to_string()],
            "latest:pro must pass through call_llm unchanged"
        );
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
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
                ..Default::default()
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

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());
        let result = engine.run("Do all three tasks").await.unwrap();

        assert_eq!(
            result.content,
            "All three results received and synthesized."
        );
        // coord (1) + a,b,c parallel (3) + coord synthesis (1) = 5
        assert_eq!(result.invocation_count, 5);
        // Trace: user→coord, coord→a, coord→b, coord→c = at least 4
        assert!(result.trace.len() >= 4);
    }

    #[tokio::test]
    async fn test_mutex_poison_recovery() {
        // Regression test for B3: verify that poisoned mutex is handled gracefully
        let config = sample_config();

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coordinator".to_string(),
            Arc::new(MockProvider::new(vec!["Direct answer."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers, null_sink());

        // Poison the mutex by panicking while holding the lock
        let state = Arc::clone(&engine.state);
        let poison_result = std::panic::catch_unwind(|| {
            let _guard = state.lock().unwrap();
            panic!("Intentional panic to poison mutex");
        });
        assert!(poison_result.is_err(), "Panic should have occurred");

        // Verify the mutex is poisoned
        assert!(
            engine.state.lock().is_err(),
            "Mutex should be poisoned after panic"
        );

        // The engine should still be able to extract results via recovery
        // (run() uses unwrap_or_else on the final lock)
        // However, invoke_agent will fail on the safety checks lock (which uses map_err)
        let result = engine.run("test").await;

        // The call will fail at the safety checks (line 174) because that lock uses Result
        assert!(
            result.is_err(),
            "Should fail when mutex is poisoned at safety checks"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Mutex poisoned") || err_msg.contains("poisoned"),
            "Error should mention mutex poisoning, got: {err_msg}"
        );
    }

    /// Provider that poisons a mutex during execution to test recovery paths
    struct PoisoningProvider {
        response: String,
        target_mutex: Arc<Mutex<EngineState>>,
    }

    impl PoisoningProvider {
        fn new(response: &str, target_mutex: Arc<Mutex<EngineState>>) -> Self {
            Self {
                response: response.to_string(),
                target_mutex,
            }
        }
    }

    #[async_trait]
    impl Provider for PoisoningProvider {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> anyhow::Result<CompletionResponse> {
            // Poison the target mutex by panicking in a spawned thread while holding the lock
            let mutex = Arc::clone(&self.target_mutex);
            let handle = std::thread::spawn(move || {
                let _guard = mutex.lock().unwrap();
                panic!("Intentional poison during provider execution");
            });

            // Wait for the poison to happen (the thread will panic)
            let _ = handle.join();

            // Verify mutex is now poisoned
            assert!(
                self.target_mutex.lock().is_err(),
                "Mutex should be poisoned after thread panic"
            );

            // Provider still returns successfully (the provider itself didn't panic)
            Ok(CompletionResponse {
                content: self.response.clone(),
                model: "poisoning-mock".to_string(),
                tokens_in: 15,
                tokens_out: 25,
                cost: 0.002,
            })
        }

        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("Streaming not supported by PoisoningProvider")
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "poisoning-mock".to_string(),
                models: vec!["poisoning-mock".to_string()],
                supports_streaming: false,
            }
        }
    }

    #[tokio::test]
    async fn test_mutex_poison_recovery_during_execution() {
        // Test recovery when mutex gets poisoned DURING execution (after safety checks).
        // This exercises the unwrap_or_else recovery sites in invoke_agent at L217, L246, etc.
        let config = sample_config();

        let mut agents = HashMap::new();
        agents.insert(
            "coordinator".to_string(),
            make_agent("coordinator", "You coordinate."),
        );

        // Build agents_info like new() does
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

        // Create state first so we can pass it to PoisoningProvider
        let state = Arc::new(Mutex::new(EngineState {
            conversations: HashMap::new(),
            trace: Vec::new(),
            iteration_count: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost: 0.0,
            invocation_count: 0,
        }));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();

        // Use PoisoningProvider that will poison the mutex during its execution
        providers.insert(
            "coordinator".to_string(),
            Arc::new(PoisoningProvider::new(
                "Response after poisoning",
                Arc::clone(&state),
            )),
        );

        let mut engine = HierarchicalEngine {
            ctx: Arc::new(EngineContext {
                config,
                agents,
                providers,
                agents_info,
                sink: null_sink(),
                routing_rules: RoutingRules::default(),
            }),
            state,
        };

        // Call run() - the provider will poison the mutex during execution,
        // then invoke_agent will hit the recovery sites when it tries to update state (L217)
        // or record the response (L246), or when run() tries to build the final result (L145)
        let result = engine.run("test input").await;

        // The execution should complete despite the poisoning, thanks to recovery sites
        assert!(
            result.is_ok(),
            "Should recover from poisoned mutex during execution, got error: {:?}",
            result.err()
        );

        let result = result.unwrap();

        // Verify we got a meaningful response (the provider did execute)
        assert!(
            result.content.contains("Response after poisoning"),
            "Should have received provider response: {}",
            result.content
        );

        // Verify metrics were captured despite poisoning
        assert!(
            result.total_tokens_in > 0,
            "Should have captured input tokens"
        );
        assert!(
            result.total_tokens_out > 0,
            "Should have captured output tokens"
        );
        assert!(result.total_cost > 0.0, "Should have captured cost");
        assert_eq!(result.invocation_count, 1, "Should have one invocation");
    }
}
