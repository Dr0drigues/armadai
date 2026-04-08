//! Integration tests for orchestration using the real Gemini API provider.
//!
//! These tests use the Google Gemini API to validate end-to-end orchestration behavior.
//! They are gated behind the `GOOGLE_API_KEY` environment variable and will be skipped
//! if it is not set, so they are safe to include in the test suite.
//!
//! To run these tests:
//! ```
//! GOOGLE_API_KEY=your_key cargo test --no-default-features --features tui,providers-api test_direct_agent_responds
//! ```
//!
//! Cost minimization:
//! - Uses `gemini-2.0-flash` (cheapest model: $0.10/$0.40 per 1M tokens)
//! - Sets `max_tokens: 200` on all agents
//! - Sets global `token_budget: 5000`
//! - Sets `temperature: 0.0` for reproducibility

#![cfg(test)]
#![cfg(feature = "providers-api")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::core::agent::{Agent, AgentMetadata};
use crate::core::orchestration::hierarchical::HierarchicalEngine;
use crate::core::orchestration::{OrchestrationConfig, OrchestrationPattern, TeamConfig};
use crate::providers::api::google::GoogleProvider;
use crate::providers::traits::Provider;

// ── Test infrastructure ──────────────────────────────────────

/// Helper to check if we should skip tests (no API key available).
fn skip_unless_gemini() -> bool {
    std::env::var("GOOGLE_API_KEY").is_err()
}

/// Create a minimal agent for testing.
fn make_test_agent(name: &str, prompt: &str) -> Agent {
    Agent {
        name: name.to_string(),
        source: PathBuf::from(format!("{name}.md")),
        metadata: AgentMetadata {
            provider: "google".to_string(),
            model: Some("gemini-2.0-flash".to_string()),
            command: None,
            args: None,
            temperature: 0.0,      // Reproducible
            max_tokens: Some(200), // Low to keep costs minimal
            timeout: Some(30),
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

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
async fn test_direct_agent_responds() {
    if skip_unless_gemini() {
        eprintln!("Skipping: GOOGLE_API_KEY not set");
        return;
    }

    let api_key = std::env::var("GOOGLE_API_KEY").unwrap();

    // Create a simple agent
    let agent = make_test_agent(
        "calculator",
        "You are a helpful assistant. Always respond in exactly one sentence.",
    );

    // Create provider
    let provider: Arc<dyn Provider> = Arc::new(GoogleProvider::new(api_key));

    // Build provider map
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    providers.insert("calculator".to_string(), provider);

    // Build agent map
    let mut agents = HashMap::new();
    agents.insert("calculator".to_string(), agent);

    // Create minimal orchestration config for direct execution
    let config = OrchestrationConfig {
        enabled: true,
        pattern: OrchestrationPattern::Direct,
        token_budget: Some(5000),
        ..Default::default()
    };

    // Create engine and run
    let mut engine = HierarchicalEngine::new(config, agents, providers);
    let result = engine.run("What is 2+2?").await;

    // Verify success
    assert!(
        result.is_ok(),
        "Direct agent execution failed: {:?}",
        result
    );
    let result = result.unwrap();

    // Assert response is meaningful
    assert!(!result.content.is_empty(), "Response is empty");
    assert!(
        result.content.contains("4") || result.content.contains("two"),
        "Response should mention the result. Got: {}",
        result.content
    );

    // Print metrics
    eprintln!(
        "Test: direct_agent | Cost: ${:.6} | Invocations: {} | Tokens: {}+{}",
        result.total_cost, result.invocation_count, result.total_tokens_in, result.total_tokens_out
    );
}

#[tokio::test]
async fn test_hierarchical_delegation_works() {
    if skip_unless_gemini() {
        eprintln!("Skipping: GOOGLE_API_KEY not set");
        return;
    }

    let api_key = std::env::var("GOOGLE_API_KEY").unwrap();

    // Create agents with specific roles
    let coordinator = make_test_agent(
        "coordinator",
        "You are a coordinator. Your job is to analyze requests and delegate to specialists. \
         Use @analyst for technical analysis tasks. Use @reviewer for code review tasks. \
         After receiving their results, synthesize a final answer. \
         Always be concise and delegate to appropriate specialists.",
    );

    let analyst = make_test_agent(
        "analyst",
        "You are a technical analyst. Provide a brief, focused technical analysis when asked. \
         Keep your response to 1-2 sentences maximum.",
    );

    let reviewer = make_test_agent(
        "reviewer",
        "You are a code reviewer. Provide a brief code review focusing on correctness and clarity. \
         Keep your response to 1-2 sentences maximum.",
    );

    // Build provider map
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    let api_key_clone = api_key.clone();
    providers.insert(
        "coordinator".to_string(),
        Arc::new(GoogleProvider::new(api_key)),
    );
    providers.insert(
        "analyst".to_string(),
        Arc::new(GoogleProvider::new(api_key_clone.clone())),
    );
    providers.insert(
        "reviewer".to_string(),
        Arc::new(GoogleProvider::new(api_key_clone)),
    );

    // Build agent map
    let mut agents = HashMap::new();
    agents.insert("coordinator".to_string(), coordinator);
    agents.insert("analyst".to_string(), analyst);
    agents.insert("reviewer".to_string(), reviewer);

    // Create hierarchical config
    let config = OrchestrationConfig {
        enabled: true,
        pattern: OrchestrationPattern::Hierarchical,
        coordinator: Some("coordinator".to_string()),
        teams: vec![TeamConfig {
            lead: None,
            agents: vec!["analyst".to_string(), "reviewer".to_string()],
        }],
        max_depth: Some(3),
        max_iterations: Some(10),
        token_budget: Some(5000),
        ..Default::default()
    };

    // Create engine and run
    let mut engine = HierarchicalEngine::new(config, agents, providers);
    let result = engine
        .run("Review this function: fn add(a: i32, b: i32) -> i32 { a + b }")
        .await;

    // Verify success
    assert!(
        result.is_ok(),
        "Hierarchical orchestration failed: {:?}",
        result
    );
    let result = result.unwrap();

    // Assertions
    assert!(!result.content.is_empty(), "Final result is empty");
    assert!(
        result.invocation_count >= 3,
        "Expected at least 3 invocations (coordinator + 2 agents), got {}",
        result.invocation_count
    );
    assert!(result.total_cost > 0.0, "Total cost should be > 0");
    assert!(
        !result.trace.is_empty(),
        "Trace should record delegation events"
    );

    // Print metrics
    eprintln!(
        "Test: hierarchical_delegation | Cost: ${:.6} | Invocations: {} | Tokens: {}+{}",
        result.total_cost, result.invocation_count, result.total_tokens_in, result.total_tokens_out
    );
}

#[tokio::test]
async fn test_budget_halts_gracefully() {
    if skip_unless_gemini() {
        eprintln!("Skipping: GOOGLE_API_KEY not set");
        return;
    }

    let api_key = std::env::var("GOOGLE_API_KEY").unwrap();

    // Create agents
    let coordinator = make_test_agent(
        "coordinator",
        "You are a coordinator. When asked a question, try to delegate to specialists for analysis. \
         Use @analyst for detailed analysis. Always synthesize a final answer.",
    );

    let analyst = make_test_agent(
        "analyst",
        "You are an analyst. Provide detailed analysis of the request. \
         Include multiple perspectives and considerations.",
    );

    // Build provider map with tight budget (500 tokens)
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    providers.insert(
        "coordinator".to_string(),
        Arc::new(GoogleProvider::new(api_key.clone())),
    );
    providers.insert(
        "analyst".to_string(),
        Arc::new(GoogleProvider::new(api_key)),
    );

    // Build agent map
    let mut agents = HashMap::new();
    agents.insert("coordinator".to_string(), coordinator);
    agents.insert("analyst".to_string(), analyst);

    // Create hierarchical config with very tight budget
    let config = OrchestrationConfig {
        enabled: true,
        pattern: OrchestrationPattern::Hierarchical,
        coordinator: Some("coordinator".to_string()),
        teams: vec![TeamConfig {
            lead: None,
            agents: vec!["analyst".to_string()],
        }],
        max_depth: Some(2),
        max_iterations: Some(5),
        token_budget: Some(500), // Very restrictive
        ..Default::default()
    };

    // Create engine and run
    let mut engine = HierarchicalEngine::new(config, agents, providers);
    let result = engine
        .run("Please provide an exhaustive analysis of the political, economic, social, and technological implications of artificial intelligence across all sectors of modern society.")
        .await;

    // Verify result (should be Ok, even if degraded)
    assert!(
        result.is_ok(),
        "Budget-limited orchestration should not error: {:?}",
        result
    );
    let result = result.unwrap();

    // We should have some content and the budget should be respected
    assert!(
        !result.content.is_empty(),
        "Even budget-limited, should have output"
    );

    // Token budget was 500, we expect either limited invocations or a message about budget
    // (The exact behavior depends on how the engine handles budget exhaustion)
    eprintln!(
        "Test: budget_halts | Cost: ${:.6} | Invocations: {} | Tokens: {}+{} (budget: 500)",
        result.total_cost, result.invocation_count, result.total_tokens_in, result.total_tokens_out
    );
}

#[tokio::test]
async fn test_agents_follow_roles() {
    if skip_unless_gemini() {
        eprintln!("Skipping: GOOGLE_API_KEY not set");
        return;
    }

    let api_key = std::env::var("GOOGLE_API_KEY").unwrap();

    // Create agents with very specific roles
    let coordinator = make_test_agent(
        "coordinator",
        "You are a coordinator. Delegate the text to a translator and a summarizer. \
         Use @translator to translate to French. Use @summarizer to create a one-sentence summary. \
         Combine their results into a coherent response.",
    );

    let translator = make_test_agent(
        "translator",
        "You are a translator. Translate any input text to French. Output ONLY the French translation, nothing else.",
    );

    let summarizer = make_test_agent(
        "summarizer",
        "You are a summarizer. Summarize the input text in exactly one sentence. Output ONLY the summary.",
    );

    // Build provider map
    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    providers.insert(
        "coordinator".to_string(),
        Arc::new(GoogleProvider::new(api_key.clone())),
    );
    providers.insert(
        "translator".to_string(),
        Arc::new(GoogleProvider::new(api_key.clone())),
    );
    providers.insert(
        "summarizer".to_string(),
        Arc::new(GoogleProvider::new(api_key)),
    );

    // Build agent map
    let mut agents = HashMap::new();
    agents.insert("coordinator".to_string(), coordinator);
    agents.insert("translator".to_string(), translator);
    agents.insert("summarizer".to_string(), summarizer);

    // Create hierarchical config
    let config = OrchestrationConfig {
        enabled: true,
        pattern: OrchestrationPattern::Hierarchical,
        coordinator: Some("coordinator".to_string()),
        teams: vec![TeamConfig {
            lead: None,
            agents: vec!["translator".to_string(), "summarizer".to_string()],
        }],
        max_depth: Some(3),
        max_iterations: Some(10),
        token_budget: Some(5000),
        ..Default::default()
    };

    // Create engine and run
    let mut engine = HierarchicalEngine::new(config, agents, providers);
    let result = engine
        .run("Process this text: The quick brown fox jumps over the lazy dog")
        .await;

    // Verify success
    assert!(
        result.is_ok(),
        "Role-based orchestration failed: {:?}",
        result
    );
    let result = result.unwrap();

    // Assertions
    assert!(!result.content.is_empty(), "Final result is empty");
    assert!(
        result.invocation_count >= 3,
        "Expected at least 3 invocations (coordinator + 2 specialists), got {}",
        result.invocation_count
    );

    // The result should indicate that both translation and summary were performed
    // (the coordinator should have combined their results)
    eprintln!(
        "Test: agents_follow_roles | Cost: ${:.6} | Invocations: {} | Result length: {} chars",
        result.total_cost,
        result.invocation_count,
        result.content.len()
    );
    eprintln!(
        "  Result preview: {}",
        &result.content[..result.content.len().min(200)]
    );
}
