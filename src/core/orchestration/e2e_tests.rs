//! End-to-end orchestration tests.
//!
//! These tests exercise the full pipeline:
//! YAML config → parse → validate → classify → engine → result.
//! They use a `ScriptedProvider` to simulate multi-turn LLM conversations.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use crate::core::agent::{Agent, AgentMetadata};
    use crate::core::orchestration::classifier::classify_with_config;
    use crate::core::orchestration::context_injection::{AgentInfo, build_orchestration_prompt};
    use crate::core::orchestration::hierarchical::HierarchicalEngine;
    use crate::core::orchestration::{
        OrchestrationConfig, OrchestrationPattern, TeamConfig, validate_config,
    };
    use crate::providers::traits::{
        CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
    };

    // ── Test infrastructure ──────────────────────────────────────

    /// A provider that returns scripted responses in order, recording requests.
    struct ScriptedProvider {
        name: String,
        responses: Mutex<Vec<String>>,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    impl ScriptedProvider {
        fn new(name: &str, responses: Vec<&str>) -> Self {
            Self {
                name: name.to_string(),
                responses: Mutex::new(responses.iter().map(|s| s.to_string()).collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.lock().unwrap().len()
        }

        fn last_system_prompt(&self) -> Option<String> {
            self.requests
                .lock()
                .unwrap()
                .last()
                .map(|r| r.system_prompt.clone())
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            self.requests.lock().unwrap().push(request);
            let content = {
                let mut responses = self.responses.lock().unwrap();
                if responses.is_empty() {
                    "No more scripted responses.".to_string()
                } else {
                    responses.remove(0)
                }
            };
            Ok(CompletionResponse {
                content,
                model: format!("mock-{}", self.name),
                tokens_in: 15,
                tokens_out: 25,
                cost: 0.002,
            })
        }

        async fn stream(&self, _: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not supported in e2e tests")
        }

        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: format!("mock-{}", self.name),
                models: vec!["mock".to_string()],
                supports_streaming: false,
            }
        }
    }

    fn make_agent(name: &str, prompt: &str) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
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

    // ── E2E: YAML → parse → validate → engine ───────────────────

    #[tokio::test]
    async fn e2e_yaml_to_hierarchical_execution() {
        // Step 1: Parse config from YAML
        let yaml = r#"
enabled: true
pattern: hierarchical
coordinator: architect
teams:
  - lead: backend-lead
    agents:
      - api-dev
      - db-dev
  - agents:
      - devops
max_depth: 4
max_iterations: 20
timeout: 120
"#;
        let config: OrchestrationConfig = serde_yaml_ng::from_str(yaml).unwrap();

        // Step 2: Validate
        assert!(validate_config(&config).is_ok());
        assert_eq!(config.pattern, OrchestrationPattern::Hierarchical);
        assert_eq!(config.coordinator.as_deref(), Some("architect"));
        assert_eq!(config.teams.len(), 2);
        assert_eq!(config.max_depth(), 4);

        // Step 3: Classify
        let classification = classify_with_config("build an API", &[], &config);
        assert_eq!(classification.pattern, OrchestrationPattern::Hierarchical);
        assert!(classification.agents.contains(&"architect".to_string()));
        assert!(classification.agents.contains(&"backend-lead".to_string()));

        // Step 4: Build agents and providers
        let mut agents = HashMap::new();
        agents.insert(
            "architect".to_string(),
            make_agent("architect", "You are the chief architect."),
        );
        agents.insert(
            "backend-lead".to_string(),
            make_agent("backend-lead", "You lead the backend team."),
        );
        agents.insert(
            "api-dev".to_string(),
            make_agent("api-dev", "You develop APIs."),
        );
        agents.insert(
            "db-dev".to_string(),
            make_agent("db-dev", "You handle databases."),
        );
        agents.insert(
            "devops".to_string(),
            make_agent("devops", "You manage infrastructure."),
        );

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "architect".to_string(),
            Arc::new(ScriptedProvider::new(
                "architect",
                vec![
                    "@backend-lead: Design the REST API for user management\n@devops: Set up CI/CD pipeline",
                    "The system is designed with a REST API backed by PostgreSQL, deployed via CI/CD.",
                ],
            )),
        );
        providers.insert(
            "backend-lead".to_string(),
            Arc::new(ScriptedProvider::new(
                "backend-lead",
                vec![
                    "@api-dev: Implement CRUD endpoints for users\n@db-dev: Design the users table schema",
                    "Backend design complete: CRUD endpoints + normalized schema.",
                ],
            )),
        );
        providers.insert(
            "api-dev".to_string(),
            Arc::new(ScriptedProvider::new(
                "api-dev",
                vec!["GET /users, POST /users, PUT /users/:id, DELETE /users/:id"],
            )),
        );
        providers.insert(
            "db-dev".to_string(),
            Arc::new(ScriptedProvider::new(
                "db-dev",
                vec!["CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT, email TEXT UNIQUE);"],
            )),
        );
        providers.insert(
            "devops".to_string(),
            Arc::new(ScriptedProvider::new(
                "devops",
                vec!["GitHub Actions workflow with Docker build, test, and deploy stages."],
            )),
        );

        // Step 5: Run the engine
        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Build a user management system").await.unwrap();

        // Step 6: Verify results
        assert!(!result.content.is_empty());
        assert!(result.trace.len() >= 5); // architect + backend-lead + api-dev + db-dev + devops
        assert!(result.invocation_count >= 5);
        assert!(result.total_tokens_in > 0);
        assert!(result.total_tokens_out > 0);
        assert!(result.total_cost > 0.0);
    }

    #[tokio::test]
    async fn e2e_context_injection_reaches_agents() {
        // Verify that orchestration context is injected into system prompts
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["worker".to_string()],
            }],
            ..Default::default()
        };

        let mut agents_info = HashMap::new();
        agents_info.insert(
            "coord".to_string(),
            AgentInfo {
                name: "coord".to_string(),
                description: Some("The coordinator".to_string()),
            },
        );
        agents_info.insert(
            "worker".to_string(),
            AgentInfo {
                name: "worker".to_string(),
                description: Some("A worker agent".to_string()),
            },
        );

        // Coordinator should get orchestration instructions
        let coord_prompt = build_orchestration_prompt("coord", &config, &agents_info);
        assert!(coord_prompt.is_some());
        let coord_prompt = coord_prompt.unwrap();
        assert!(coord_prompt.contains("Orchestration Protocol"));
        assert!(coord_prompt.contains("worker"));

        // Worker should also get orchestration instructions
        let worker_prompt = build_orchestration_prompt("worker", &config, &agents_info);
        assert!(worker_prompt.is_some());
        let worker_prompt = worker_prompt.unwrap();
        assert!(worker_prompt.contains("Orchestration Protocol"));
    }

    #[tokio::test]
    async fn e2e_context_injection_in_engine_calls() {
        // Verify the engine actually injects orchestration context into LLM calls
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["worker".to_string()],
            }],
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert(
            "coord".to_string(),
            make_agent("coord", "You coordinate tasks."),
        );
        agents.insert(
            "worker".to_string(),
            make_agent("worker", "You do the work."),
        );

        let coord_provider = Arc::new(ScriptedProvider::new("coord", vec!["Direct answer."]));
        let worker_provider = Arc::new(ScriptedProvider::new("worker", vec!["Done."]));

        let coord_ref = Arc::clone(&coord_provider);

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert("coord".to_string(), coord_provider);
        providers.insert("worker".to_string(), worker_provider);

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let _ = engine.run("Do something").await.unwrap();

        // The coordinator's system prompt should contain orchestration context
        let system_prompt = coord_ref.last_system_prompt().unwrap();
        assert!(
            system_prompt.contains("Orchestration Protocol"),
            "System prompt should contain orchestration context, got: {}",
            &system_prompt[..system_prompt.len().min(200)]
        );
    }

    #[tokio::test]
    async fn e2e_auto_pattern_with_hierarchical_config() {
        // Auto pattern should detect hierarchical when coordinator + teams are set
        let yaml = r#"
enabled: true
pattern: auto
coordinator: boss
teams:
  - agents:
      - worker-1
      - worker-2
"#;
        let config: OrchestrationConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.pattern, OrchestrationPattern::Auto);

        let classification = classify_with_config("any task", &[], &config);
        assert_eq!(classification.pattern, OrchestrationPattern::Hierarchical);

        // Verify it can actually run
        let mut agents = HashMap::new();
        agents.insert("boss".to_string(), make_agent("boss", "You are the boss."));
        agents.insert(
            "worker-1".to_string(),
            make_agent("worker-1", "Worker one."),
        );
        agents.insert(
            "worker-2".to_string(),
            make_agent("worker-2", "Worker two."),
        );

        // Need to run with the explicit Hierarchical pattern for the engine
        let mut run_config = config.clone();
        run_config.pattern = classification.pattern;

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "boss".to_string(),
            Arc::new(ScriptedProvider::new("boss", vec!["I've decided: 42."])),
        );

        let mut engine = HierarchicalEngine::new(run_config, agents, providers);
        let result = engine.run("What is the answer?").await.unwrap();
        assert_eq!(result.content, "I've decided: 42.");
    }

    #[tokio::test]
    async fn e2e_validation_rejects_bad_config() {
        let yaml = r#"
enabled: true
pattern: hierarchical
teams:
  - agents:
      - worker-1
"#;
        let config: OrchestrationConfig = serde_yaml_ng::from_str(yaml).unwrap();

        // Should fail validation: no coordinator
        let errors = validate_config(&config).unwrap_err();
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.to_string().contains("coordinator")));
    }

    #[tokio::test]
    async fn e2e_validation_rejects_duplicate_agents() {
        let yaml = r#"
enabled: true
pattern: hierarchical
coordinator: boss
teams:
  - agents:
      - agent-x
  - agents:
      - agent-x
"#;
        let config: OrchestrationConfig = serde_yaml_ng::from_str(yaml).unwrap();

        let errors = validate_config(&config).unwrap_err();
        assert!(errors.iter().any(|e| e.to_string().contains("agent-x")));
    }

    #[tokio::test]
    async fn e2e_deep_delegation_chain() {
        // coordinator → lead → worker, with depth limit respected
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coordinator".to_string()),
            teams: vec![TeamConfig {
                lead: Some("lead".to_string()),
                agents: vec!["worker".to_string()],
            }],
            max_depth: Some(5),
            ..Default::default()
        };

        assert!(validate_config(&config).is_ok());

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
            Arc::new(ScriptedProvider::new(
                "coordinator",
                vec![
                    "@lead: handle this task",
                    "Final answer after delegation chain.",
                ],
            )),
        );
        providers.insert(
            "lead".to_string(),
            Arc::new(ScriptedProvider::new(
                "lead",
                vec![
                    "@worker: implement the details",
                    "Lead synthesis: implementation done.",
                ],
            )),
        );
        providers.insert(
            "worker".to_string(),
            Arc::new(ScriptedProvider::new(
                "worker",
                vec!["Implementation complete."],
            )),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Build the feature").await.unwrap();

        assert!(!result.content.is_empty());
        // Trace should show: user→coordinator, coordinator→lead, lead→worker
        assert!(result.trace.len() >= 3);

        // Verify depth levels in trace
        let depths: Vec<u32> = result.trace.iter().map(|e| e.depth).collect();
        assert!(depths.contains(&0)); // coordinator
        assert!(depths.contains(&1)); // lead
        assert!(depths.contains(&2)); // worker
    }

    #[tokio::test]
    async fn e2e_metrics_aggregate_across_chain() {
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["a".to_string(), "b".to_string()],
            }],
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert("coord".to_string(), make_agent("coord", "Coordinator."));
        agents.insert("a".to_string(), make_agent("a", "Agent A."));
        agents.insert("b".to_string(), make_agent("b", "Agent B."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coord".to_string(),
            Arc::new(ScriptedProvider::new(
                "coord",
                vec!["@a: task A\n@b: task B", "Combined results."],
            )),
        );
        providers.insert(
            "a".to_string(),
            Arc::new(ScriptedProvider::new("a", vec!["A done."])),
        );
        providers.insert(
            "b".to_string(),
            Arc::new(ScriptedProvider::new("b", vec!["B done."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Do both").await.unwrap();

        // 4 LLM calls: coord(delegate) + a + b + coord(synthesize)
        assert_eq!(result.invocation_count, 4);
        // Each call: 15 tokens in, 25 tokens out, $0.002
        assert_eq!(result.total_tokens_in, 60);
        assert_eq!(result.total_tokens_out, 100);
        assert!((result.total_cost - 0.008).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn e2e_direct_pattern_single_agent() {
        // Even with orchestration config, Direct pattern should work
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Direct,
            ..Default::default()
        };

        let agents = vec![make_agent("solo", "I do general tasks.")];
        let classification = classify_with_config("do something", &agents, &config);
        assert_eq!(classification.pattern, OrchestrationPattern::Direct);
    }

    #[tokio::test]
    async fn e2e_conversation_history_preserved() {
        // Verify that when an agent is called multiple times, conversation history is maintained
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["worker".to_string()],
            }],
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert("coord".to_string(), make_agent("coord", "You coordinate."));
        agents.insert("worker".to_string(), make_agent("worker", "You work."));

        let coord_provider = Arc::new(ScriptedProvider::new(
            "coord",
            vec!["@worker: first task", "All done."],
        ));
        let coord_ref = Arc::clone(&coord_provider);

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert("coord".to_string(), coord_provider);
        providers.insert(
            "worker".to_string(),
            Arc::new(ScriptedProvider::new("worker", vec!["First result."])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let _ = engine.run("Do the work").await.unwrap();

        // Coordinator was called twice (delegate then synthesize)
        assert_eq!(coord_ref.request_count(), 2);
    }

    #[tokio::test]
    async fn e2e_unknown_agent_in_delegation_is_error() {
        // If the LLM delegates to an agent that doesn't exist in the provider map
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec!["worker".to_string()],
            }],
            ..Default::default()
        };

        let mut agents = HashMap::new();
        agents.insert("coord".to_string(), make_agent("coord", "You coordinate."));
        // "worker" agent exists in config but no provider for "ghost-agent"
        agents.insert("worker".to_string(), make_agent("worker", "You work."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coord".to_string(),
            Arc::new(ScriptedProvider::new(
                "coord",
                vec!["@ghost-agent: do something impossible"],
            )),
        );
        providers.insert(
            "worker".to_string(),
            Arc::new(ScriptedProvider::new("worker", vec!["ok"])),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Test unknown agent").await;

        // Should error because ghost-agent has no provider
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ghost-agent"));
    }

    #[tokio::test]
    async fn e2e_parallel_delegations_results_collected() {
        // Coordinator delegates to 3 agents at once
        let config = OrchestrationConfig {
            enabled: true,
            pattern: OrchestrationPattern::Hierarchical,
            coordinator: Some("coord".to_string()),
            teams: vec![TeamConfig {
                lead: None,
                agents: vec![
                    "security".to_string(),
                    "performance".to_string(),
                    "ux".to_string(),
                ],
            }],
            ..Default::default()
        };

        assert!(validate_config(&config).is_ok());

        let mut agents = HashMap::new();
        agents.insert("coord".to_string(), make_agent("coord", "Coordinator."));
        agents.insert(
            "security".to_string(),
            make_agent("security", "Security expert."),
        );
        agents.insert(
            "performance".to_string(),
            make_agent("performance", "Performance expert."),
        );
        agents.insert("ux".to_string(), make_agent("ux", "UX expert."));

        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        providers.insert(
            "coord".to_string(),
            Arc::new(ScriptedProvider::new(
                "coord",
                vec![
                    "@security: audit the auth flow\n@performance: profile the API\n@ux: review the login page",
                    "Combined review: auth is solid, API needs caching, login page is clean.",
                ],
            )),
        );
        providers.insert(
            "security".to_string(),
            Arc::new(ScriptedProvider::new(
                "security",
                vec!["Auth flow uses proper PKCE and token rotation."],
            )),
        );
        providers.insert(
            "performance".to_string(),
            Arc::new(ScriptedProvider::new(
                "performance",
                vec!["API p99 is 450ms, recommend adding Redis cache."],
            )),
        );
        providers.insert(
            "ux".to_string(),
            Arc::new(ScriptedProvider::new(
                "ux",
                vec!["Login page follows WCAG 2.1 AA guidelines."],
            )),
        );

        let mut engine = HierarchicalEngine::new(config, agents, providers);
        let result = engine.run("Review the application").await.unwrap();

        assert!(!result.content.is_empty());
        // 5 calls: coord(delegate) + security + performance + ux + coord(synthesize)
        assert_eq!(result.invocation_count, 5);
        // 4 agents in trace (coord + 3 specialists)
        assert!(result.trace.len() >= 4);
    }
}
