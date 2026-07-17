use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::core::agent::{Agent, AgentMode};
use crate::core::config::AppPaths;
use crate::core::events::{EventSink, RunEvent};
use crate::core::project::{self, AgentRef, ProjectConfig, ProjectDefaults};
use crate::providers::factory::create_provider;
use crate::providers::rate_limiter::RateLimiter;
use crate::providers::traits::{ChatMessage, CompletionRequest};

const GUIDED_MODE_INSTRUCTION: &str = "\
\n\n---\n\n\
**Important**: Before providing your full response, assess whether the request \
is clear and complete. If critical details are missing, ambiguous, or could \
significantly change your approach, ask 2-3 targeted clarifying questions first. \
Only proceed with your complete response once you have enough context to deliver \
accurate, relevant output.";

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
    orchestrate: Option<String>,
    headless: bool,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
) -> anyhow::Result<()> {
    // headless is implied by json (machine output cannot be interrupted by a prompt)
    let headless = headless || json;
    let sink = crate::core::events::make_sink(json);

    let result = run_inner(
        agent_name,
        input,
        pipe,
        orchestrate,
        headless,
        json,
        quiet,
        max_content,
        &sink,
    )
    .await;

    if let Err(e) = result {
        if headless {
            let code = exit_code_for(&e);
            sink.emit(&RunEvent::Error {
                code: match code {
                    3 => "budget_exceeded",
                    4 => "provider_unavailable",
                    _ => "agent_failed",
                }
                .into(),
                msg: e.to_string(),
            });
            std::process::exit(code);
        }
        return Err(e);
    }

    Ok(())
}

/// Map a run error to a CI-friendly exit code.
///
/// - `0`: success (handled by caller, never produced here)
/// - `1`: generic execution error
/// - `2`: usage error (reserved for CLI-level argument validation)
/// - `3`: budget/cost limit exceeded
/// - `4`: provider unavailable
fn exit_code_for(err: &anyhow::Error) -> i32 {
    let s = err.to_string().to_lowercase();
    if s.contains("budget") || s.contains("cost limit") {
        3
    } else if s.contains("not available") || s.contains("unavailable") {
        4
    } else {
        1
    }
}

/// Core run logic (sequential or orchestrated). Kept separate from [`execute`] so that
/// all error paths funnel through a single headless error-event + exit-code handler.
#[allow(clippy::too_many_arguments)]
async fn run_inner(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
    orchestrate: Option<String>,
    headless: bool,
    json: bool,
    quiet: bool,
    max_content: Option<usize>,
    sink: &Arc<dyn EventSink>,
) -> anyhow::Result<()> {
    let resolution = resolve_agents_dir(headless);

    // Build the execution chain: primary agent + piped agents
    let mut chain = vec![agent_name];
    if let Some(extra) = pipe {
        chain.extend(extra);
    }

    // Resolve input text
    let current_input = resolve_input(input).await?;

    // Orchestrated multi-agent execution (explicit --orchestrate flag)
    if let Some(pattern) = orchestrate {
        if chain.len() < 2 {
            anyhow::bail!("--orchestrate requires at least 2 agents (use --pipe to add more)");
        }
        return run_orchestrated(&resolution, &chain, &current_input, &pattern, sink, json).await;
    }

    // Auto-detect orchestration from project config (orchestration.enabled: true)
    if let AgentResolution::Project { ref config, .. } = resolution
        && let Some(ref orch) = config.orchestration
        && orch.enabled
    {
        let pattern = orch.pattern.to_string();
        // Collect all agents from orchestration config
        let mut orch_agents = Vec::new();
        if let Some(ref coord) = orch.coordinator {
            orch_agents.push(coord.clone());
        }
        for team in &orch.teams {
            if let Some(ref lead) = team.lead {
                orch_agents.push(lead.clone());
            }
            orch_agents.extend(team.agents.iter().cloned());
        }
        if !orch_agents.is_empty() {
            return run_orchestrated(
                &resolution,
                &orch_agents,
                &current_input,
                &pattern,
                sink,
                json,
            )
            .await;
        }
    }

    // Standard sequential execution (backward compatible)
    let mut current_input = current_input;
    let project_defaults = match &resolution {
        AgentResolution::Project { config, .. } => Some(&config.defaults),
        _ => None,
    };
    let routing_rules = match &resolution {
        AgentResolution::Project { config, .. } => config.routing.clone().unwrap_or_default(),
        _ => crate::core::routing::RoutingRules::default(),
    };

    sink.emit(&RunEvent::RunStart {
        v: 1,
        agents: chain.clone(),
        prov: String::new(), // filled per-agent in agent_start; kept minimal here
        model: String::new(),
        in_chars: current_input.chars().count(),
    });

    let mut agg_tin = 0u32;
    let mut agg_tout = 0u32;
    let mut agg_cost = 0.0f64;

    for (i, name) in chain.iter().enumerate() {
        if chain.len() > 1 && !json {
            eprintln!("--- [{}/{} {}] ---", i + 1, chain.len(), name);
        }

        let agent_path = resolve_agent_path(&resolution, name)?;
        let (output, metrics) = run_single_agent(
            &agent_path,
            name,
            &current_input,
            project_defaults,
            sink,
            quiet,
            max_content,
            &routing_rules,
        )
        .await?;
        agg_tin += metrics.tokens_in as u32;
        agg_tout += metrics.tokens_out as u32;
        agg_cost += metrics.cost;
        current_input = output;
    }

    sink.emit(&RunEvent::Result {
        content: current_input.clone(),
        tin: agg_tin,
        tout: agg_tout,
        cost: agg_cost,
        agents: chain.len(),
    });

    // Human/plain output only when not emitting JSON
    if !json {
        println!("{current_input}");
    }

    Ok(())
}

/// Result of resolving the agents directory / project config.
enum AgentResolution {
    /// New-format project config with walk-up root
    Project {
        root: PathBuf,
        config: Box<ProjectConfig>,
    },
    /// No project config found — use default paths
    Default(PathBuf),
}

/// Resolve a single agent name to a file path using the resolution context.
fn resolve_agent_path(resolution: &AgentResolution, agent_name: &str) -> anyhow::Result<PathBuf> {
    match resolution {
        AgentResolution::Project { root, config } => {
            // If the agent is declared in the project config, resolve it
            if let Some(agent_ref) = config.agents.iter().find(|r| match r {
                AgentRef::Named { name } => name == agent_name,
                AgentRef::Path { path } => path.file_stem().is_some_and(|s| s == agent_name),
                AgentRef::Registry { registry } => registry.ends_with(agent_name),
            }) {
                return project::resolve_agent(agent_ref, root);
            }

            // Not declared in config — try resolving as Named anyway
            let fallback_ref = AgentRef::Named {
                name: agent_name.to_string(),
            };
            project::resolve_agent(&fallback_ref, root)
        }
        AgentResolution::Default(agents_dir) => Agent::find_file(agents_dir, agent_name)
            .ok_or_else(|| {
                anyhow::anyhow!("Agent '{agent_name}' not found in {}", agents_dir.display())
            }),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_single_agent(
    agent_path: &Path,
    agent_name: &str,
    input: &str,
    project_defaults: Option<&ProjectDefaults>,
    sink: &Arc<dyn EventSink>,
    quiet: bool,
    max_content: Option<usize>,
    routing_rules: &crate::core::routing::RoutingRules,
) -> anyhow::Result<(String, RunMetrics)> {
    // 1. Load agent
    let mut agent = crate::parser::parse_agent_file(agent_path)?;

    // 1b. Resolve deprecated model aliases
    let model_before = agent.metadata.model.clone();
    crate::linker::model_aliases::resolve_model_deprecations(
        &mut agent.metadata.model,
        &mut agent.metadata.model_fallback,
    );
    if agent.metadata.model != model_before {
        sink.emit(&RunEvent::Warning {
            code: "deprecated_model".to_string(),
            from: model_before,
            to: agent.metadata.model.clone(),
        });
    }
    // 1c. Warn if model unknown in registry
    if let Some(ref model) = agent.metadata.model {
        crate::linker::model_resolution::warn_unknown_model(model, &agent.metadata.provider);
    }

    // 2. Create provider
    let provider = create_provider(&agent)?;

    // 3. Apply rate limiting if configured
    if let Some(ref rate_str) = agent.metadata.rate_limit
        && let Some(rpm) = RateLimiter::parse_rate(rate_str)
    {
        let limiter = RateLimiter::new(rpm);
        limiter.acquire().await;
    }

    // 4. Resolve effective mode and build system prompt
    let effective_mode = agent
        .metadata
        .mode
        .or(project_defaults.and_then(|d| d.mode))
        .unwrap_or_default();

    let system_prompt = if effective_mode == AgentMode::Guided {
        format!("{}{GUIDED_MODE_INSTRUCTION}", agent.system_prompt)
    } else {
        agent.system_prompt.clone()
    };

    // 5. Build request
    let raw_model = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

    let model = if raw_model == "latest:auto" {
        let (tier, reason) =
            crate::core::routing::route(input, &agent.metadata.tags, None, routing_rules);
        sink.emit(&RunEvent::Route {
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
        system_prompt,
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: input.to_string(),
        }],
        temperature: agent.metadata.temperature,
        max_tokens: agent.metadata.max_tokens,
    };

    sink.emit(&RunEvent::AgentStart {
        agent: agent_name.to_string(),
        prov: agent.metadata.provider.clone(),
        model: agent.metadata.model.clone().unwrap_or_default(),
    });

    // 6. Execute (with model fallback)
    let start = Instant::now();
    let response = match provider.complete(request.clone()).await {
        Ok(resp) => resp,
        Err(err) if is_model_not_found(&err) && !agent.metadata.model_fallback.is_empty() => {
            let mut last_err = err;
            let mut fallback_resp = None;
            for fallback_model in &agent.metadata.model_fallback {
                eprintln!("[{agent_name}] Model unavailable, falling back to {fallback_model}...");
                let mut retry_request = request.clone();
                retry_request.model = fallback_model.clone();
                match provider.complete(retry_request).await {
                    Ok(resp) => {
                        fallback_resp = Some(resp);
                        break;
                    }
                    Err(e) if is_model_not_found(&e) => {
                        last_err = e;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
            fallback_resp.ok_or(last_err)?
        }
        Err(err) => return Err(err),
    };
    let duration = start.elapsed();

    if !quiet {
        let content_out = match max_content {
            Some(n) => response.content.chars().take(n).collect::<String>(),
            None => response.content.clone(),
        };
        sink.emit(&RunEvent::AgentEnd {
            agent: agent_name.to_string(),
            tin: response.tokens_in,
            tout: response.tokens_out,
            cost: response.cost,
            content: content_out,
        });
    }

    // 7. Print summary to stderr (so stdout is clean for piping)
    let duration_ms = duration.as_millis() as i64;
    eprintln!(
        "\n[{}] model={} tokens={}/{} cost=${:.6} duration={}ms",
        agent_name,
        response.model,
        response.tokens_in,
        response.tokens_out,
        response.cost,
        duration_ms
    );

    let metrics = RunMetrics {
        agent: agent_name.to_string(),
        provider_name: agent.metadata.provider.clone(),
        model: response.model.clone(),
        tokens_in: response.tokens_in as i64,
        tokens_out: response.tokens_out as i64,
        cost: response.cost,
        duration_ms,
    };

    // 8. Record in storage (if available)
    #[cfg(feature = "storage")]
    record_run(&metrics, input, &response.content);

    Ok((response.content, metrics))
}

#[allow(dead_code)]
struct RunMetrics {
    agent: String,
    provider_name: String,
    model: String,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    duration_ms: i64,
}

#[cfg(feature = "storage")]
fn record_run(metrics: &RunMetrics, input: &str, output: &str) {
    use crate::storage::{init_db, queries};

    let db = match init_db() {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to init storage: {e}");
            return;
        }
    };

    let record = queries::RunRecord {
        agent: metrics.agent.clone(),
        input: input.to_string(),
        output: output.to_string(),
        provider: metrics.provider_name.clone(),
        model: metrics.model.clone(),
        tokens_in: metrics.tokens_in,
        tokens_out: metrics.tokens_out,
        cost: metrics.cost,
        duration_ms: metrics.duration_ms,
        status: "success".to_string(),
    };

    if let Err(e) = queries::insert_run(&db, record) {
        tracing::warn!("Failed to record run: {e}");
    }
}

async fn resolve_input(input: Option<String>) -> anyhow::Result<String> {
    match input {
        Some(text) if text.starts_with('@') => {
            let path = &text[1..];
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read input file '{path}': {e}"))
        }
        Some(text) => Ok(text),
        None => {
            // Try reading from stdin if piped
            if atty_is_pipe() {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
                if buf.is_empty() {
                    anyhow::bail!("No input provided. Usage: armadai run <agent> <input>");
                }
                Ok(buf)
            } else {
                anyhow::bail!("No input provided. Usage: armadai run <agent> \"<input>\"");
            }
        }
    }
}

/// Check if stdin is a pipe (not a terminal).
fn atty_is_pipe() -> bool {
    use std::io::IsTerminal;
    !std::io::stdin().is_terminal()
}

/// Resolve agent source: walk up for `armadai.yaml`, detect format,
/// and return the appropriate resolution strategy.
fn resolve_agents_dir(headless: bool) -> AgentResolution {
    // 1. Walk-up search for project config (new or legacy format)
    if let Some((root, config)) = project::find_project_config()
        && !config.agents.is_empty()
    {
        tracing::info!(
            "Using project config from {} ({} agent(s))",
            root.display(),
            config.agents.len()
        );
        if let Err(e) = crate::core::project_registry::register_project(&root) {
            tracing::warn!("Failed to register project in registry: {:?}", e);
        }
        let interactive = !headless && !atty_is_pipe();
        crate::core::model_updater::auto_check_and_prompt(&root, interactive);
        return AgentResolution::Project {
            root,
            config: Box::new(config),
        };
    }

    // 2. Default fallback
    AgentResolution::Default(AppPaths::resolve().agents_dir)
}

/// Run orchestrated multi-agent execution (blackboard or ring).
async fn run_orchestrated(
    resolution: &AgentResolution,
    agent_names: &[String],
    input: &str,
    pattern: &str,
    sink: &std::sync::Arc<dyn crate::core::events::EventSink>,
    json: bool,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use crate::core::orchestration::blackboard::{
        BlackboardConfig, Board, BoardAgent, run_blackboard,
    };
    use crate::core::orchestration::llm_agents::{LlmBoardAgent, LlmRingAgent};
    use crate::core::orchestration::ring::{
        RingAgent, RingConfig, RingOutcome, RingToken, TokenStatus, run_ring,
    };
    use crate::core::project::OrchestrationDefaults;
    use crate::providers::traits::Provider;

    // Load all agents and create providers
    let mut agents = Vec::new();
    let mut providers: Vec<Arc<dyn Provider>> = Vec::new();

    // Read project-level orchestration overrides (if any).
    let orch_defaults = match resolution {
        AgentResolution::Project { config, .. } => {
            config.defaults.orchestration.clone().unwrap_or_default()
        }
        _ => OrchestrationDefaults::default(),
    };

    sink.emit(&RunEvent::RunStart {
        v: 1,
        agents: agent_names.to_vec(),
        prov: String::new(),
        model: pattern.to_string(),
        in_chars: input.chars().count(),
    });

    for name in agent_names {
        let agent_path = resolve_agent_path(resolution, name)?;
        let mut agent = crate::parser::parse_agent_file(&agent_path)?;

        let model_before = agent.metadata.model.clone();
        crate::linker::model_aliases::resolve_model_deprecations(
            &mut agent.metadata.model,
            &mut agent.metadata.model_fallback,
        );
        if agent.metadata.model != model_before {
            sink.emit(&RunEvent::Warning {
                code: "deprecated_model".to_string(),
                from: model_before,
                to: agent.metadata.model.clone(),
            });
        }

        sink.emit(&RunEvent::AgentStart {
            agent: name.clone(),
            prov: agent.metadata.provider.clone(),
            model: agent.metadata.model.clone().unwrap_or_default(),
        });
        let provider = create_provider(&agent)?;
        providers.push(Arc::from(provider));
        agents.push(agent);
    }

    match pattern {
        "blackboard" => {
            let board_agents: Vec<Arc<dyn BoardAgent>> = agents
                .into_iter()
                .map(|a| Arc::new(LlmBoardAgent::new(a)) as Arc<dyn BoardAgent>)
                .collect();

            let config = apply_blackboard_overrides(BlackboardConfig::default(), &orch_defaults);
            let mut board = Board::new(input.to_string(), config.token_budget);

            eprintln!(
                "[blackboard] Starting with {} agent(s), max {} rounds",
                board_agents.len(),
                config.max_rounds
            );

            run_blackboard(&mut board, &board_agents, &providers, &config).await?;

            eprintln!("[blackboard] Halted: {:?}", board.state());

            #[cfg(feature = "storage")]
            record_orchestration_blackboard(&board, &config, input);

            let outcome_text = board
                .entries()
                .iter()
                .map(|entry| format!("[{}] {}", entry.agent, entry.content))
                .collect::<Vec<_>>()
                .join("\n");

            if !json {
                println!("{outcome_text}");
            }

            // NOTE: token/cost aggregation for orchestration requires engine-level
            // instrumentation (out of scope for beta.3).
            emit_agent_ends(sink, agent_names);
            sink.emit(&RunEvent::Result {
                content: outcome_text,
                tin: 0,
                tout: 0,
                cost: 0.0,
                agents: agent_names.len(),
            });
        }
        "ring" => {
            let ring_agents: Vec<Arc<dyn RingAgent>> = agents
                .into_iter()
                .map(|a| Arc::new(LlmRingAgent::new(a)) as Arc<dyn RingAgent>)
                .collect();

            let agent_order: Vec<String> =
                ring_agents.iter().map(|a| a.name().to_string()).collect();

            let config = apply_ring_overrides(RingConfig::default(), &orch_defaults);
            let mut token = RingToken::new(input.to_string(), agent_order, config.token_budget);

            eprintln!(
                "[ring] Starting with {} agent(s), max {} laps",
                ring_agents.len(),
                config.max_laps
            );

            run_ring(&mut token, &ring_agents, &providers, &config).await?;

            #[cfg(feature = "storage")]
            record_orchestration_ring(&token, &config, input);

            let outcome_text = match token.status() {
                TokenStatus::Done { outcome } => match outcome {
                    RingOutcome::Consensus {
                        resolution, score, ..
                    } => {
                        eprintln!("[ring] Consensus ({:.0}%)", score * 100.0);
                        if !json {
                            println!("{resolution}");
                        }
                        resolution.clone()
                    }
                    RingOutcome::Majority {
                        resolution,
                        score,
                        dissents,
                    } => {
                        eprintln!(
                            "[ring] Majority ({:.0}%, {} dissent(s))",
                            score * 100.0,
                            dissents.len()
                        );
                        if !json {
                            println!("{resolution}");
                        }
                        resolution.clone()
                    }
                    RingOutcome::NoConsensus { summary, .. } => {
                        eprintln!("[ring] No consensus");
                        if !json {
                            println!("{summary}");
                        }
                        summary.clone()
                    }
                    RingOutcome::BudgetExhausted { partial_summary } => {
                        eprintln!("[ring] Budget exhausted");
                        if !json {
                            println!("{partial_summary}");
                        }
                        partial_summary.clone()
                    }
                    RingOutcome::CostLimitExceeded {
                        partial_summary,
                        spent,
                        limit,
                    } => {
                        eprintln!("[ring] Cost limit exceeded: ${:.4}/${:.4}", spent, limit);
                        if !json {
                            println!("{partial_summary}");
                        }
                        partial_summary.clone()
                    }
                    RingOutcome::Cancelled => {
                        eprintln!("[ring] Cancelled");
                        String::new()
                    }
                },
                other => {
                    eprintln!("[ring] Unexpected status: {other:?}");
                    String::new()
                }
            };

            // NOTE: token/cost aggregation for orchestration requires engine-level
            // instrumentation (out of scope for beta.3).
            emit_agent_ends(sink, agent_names);
            sink.emit(&RunEvent::Result {
                content: outcome_text,
                tin: 0,
                tout: 0,
                cost: 0.0,
                agents: agent_names.len(),
            });
        }
        "hierarchical" => {
            use std::collections::HashMap;

            use crate::core::orchestration::OrchestrationConfig;
            use crate::core::orchestration::hierarchical::HierarchicalEngine;

            // Build orchestration config from project or defaults
            let orch_config = match resolution {
                AgentResolution::Project { config, .. } => {
                    config.orchestration.as_deref().cloned().unwrap_or_default()
                }
                _ => OrchestrationConfig::default(),
            };

            // Validate the config
            if let Err(errors) = crate::core::orchestration::validate_config(&orch_config) {
                let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                anyhow::bail!("Orchestration config errors:\n  - {}", msgs.join("\n  - "));
            }

            let coordinator_name = orch_config
                .coordinator
                .as_deref()
                .unwrap_or(agent_names.first().map(|s| s.as_str()).unwrap_or(""));

            // Build agent map and provider map
            let mut agent_map: HashMap<String, crate::core::agent::Agent> = HashMap::new();
            let mut provider_map: HashMap<String, Arc<dyn Provider>> = HashMap::new();

            for (agent, provider) in agents.into_iter().zip(providers) {
                provider_map.insert(agent.name.clone(), provider);
                agent_map.insert(agent.name.clone(), agent);
            }

            eprintln!(
                "[hierarchical] Starting with coordinator '{}', {} agent(s)",
                coordinator_name,
                agent_map.len()
            );

            let mut engine = HierarchicalEngine::new(orch_config, agent_map, provider_map);
            let result = engine.run(input).await?;

            eprintln!(
                "[hierarchical] Done: {} invocations, {} tokens in, {} tokens out",
                result.invocation_count, result.total_tokens_in, result.total_tokens_out
            );

            if !json {
                println!("{}", result.content);
            }

            emit_agent_ends(sink, agent_names);
            sink.emit(&RunEvent::Result {
                content: result.content,
                tin: result.total_tokens_in,
                tout: result.total_tokens_out,
                cost: result.total_cost,
                agents: agent_names.len(),
            });
        }
        other => {
            anyhow::bail!(
                "Unknown orchestration pattern: '{other}'. Use 'blackboard', 'ring', or 'hierarchical'"
            );
        }
    }

    Ok(())
}

/// Emit one `AgentEnd` event per agent, in order, restoring the JSONL contract's
/// start/end symmetry for orchestrated runs (spec §3).
///
/// Per-agent completion metrics (tokens, cost) are not available from the
/// orchestration engines (blackboard/ring/hierarchical aggregate at the run
/// level only), so each event carries zeroed metrics and empty content — this
/// is documented out-of-scope, not a bug. Call immediately before emitting the
/// terminal `Result` event.
fn emit_agent_ends(
    sink: &std::sync::Arc<dyn crate::core::events::EventSink>,
    agent_names: &[String],
) {
    for name in agent_names {
        sink.emit(&RunEvent::AgentEnd {
            agent: name.clone(),
            tin: 0,
            tout: 0,
            cost: 0.0,
            content: String::new(),
        });
    }
}

/// Apply project-level orchestration overrides to a BlackboardConfig.
fn apply_blackboard_overrides(
    mut config: crate::core::orchestration::blackboard::BlackboardConfig,
    overrides: &crate::core::project::OrchestrationDefaults,
) -> crate::core::orchestration::blackboard::BlackboardConfig {
    if let Some(v) = overrides.max_rounds {
        config.max_rounds = v;
    }
    if let Some(v) = overrides.consensus_threshold {
        config.consensus_threshold = v;
    }
    if let Some(v) = overrides.divergence_threshold {
        config.divergence_threshold = v;
    }
    if let Some(v) = overrides.token_budget {
        config.token_budget = v;
    }
    if let Some(v) = overrides.agent_timeout_secs {
        config.agent_timeout_secs = v;
    }
    if let Some(v) = overrides.convergence_rounds {
        config.convergence_rounds = v;
    }
    config
}

/// Apply project-level orchestration overrides to a RingConfig.
fn apply_ring_overrides(
    mut config: crate::core::orchestration::ring::RingConfig,
    overrides: &crate::core::project::OrchestrationDefaults,
) -> crate::core::orchestration::ring::RingConfig {
    if let Some(v) = overrides.max_laps {
        config.max_laps = v;
    }
    if let Some(v) = overrides.consensus_threshold {
        config.consensus_threshold = v;
    }
    if let Some(v) = overrides.majority_threshold {
        config.majority_threshold = v;
    }
    if let Some(v) = overrides.similarity_threshold {
        config.similarity_threshold = v;
    }
    if let Some(v) = overrides.token_budget {
        config.token_budget = v;
    }
    if let Some(v) = overrides.agent_timeout_secs {
        config.agent_timeout_secs = v;
    }
    config
}

#[cfg(feature = "storage")]
fn record_orchestration_blackboard(
    board: &crate::core::orchestration::blackboard::Board,
    config: &crate::core::orchestration::blackboard::BlackboardConfig,
    input: &str,
) {
    use crate::core::orchestration::blackboard::BoardState;
    use crate::storage::{init_db, queries};

    let db = match init_db() {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to init storage: {e}");
            return;
        }
    };

    let run_id = uuid::Uuid::new_v4().to_string();

    // 1. Parent run record
    let parent = queries::RunRecord {
        agent: "orchestration:blackboard".to_string(),
        input: input.to_string(),
        output: format!("{:?}", board.state()),
        provider: "orchestration".to_string(),
        model: String::new(),
        tokens_in: board.budget().used as i64,
        tokens_out: 0,
        cost: 0.0,
        duration_ms: 0,
        status: if board.is_halted() {
            "halted"
        } else {
            "success"
        }
        .to_string(),
    };
    if let Err(e) = queries::insert_run_with_id(&db, &run_id, parent) {
        tracing::warn!("Failed to record orchestration parent run: {e}");
        return;
    }

    // 2. Orchestration metadata
    let halt_reason = match board.state() {
        BoardState::Halted { reason } => Some(format!("{reason:?}")),
        _ => None,
    };
    let orch = queries::OrchestrationRunRecord {
        run_id: run_id.clone(),
        pattern: "blackboard".to_string(),
        config_json: serde_json::to_string(config).unwrap_or_default(),
        outcome_json: serde_json::to_string(board.state()).ok(),
        rounds: board.round as i64,
        halt_reason,
    };
    if let Err(e) = queries::insert_orchestration_run(&db, orch) {
        tracing::warn!("Failed to record orchestration metadata: {e}");
        return;
    }

    // 3. Board entries
    for entry in board.entries() {
        let kind_str = match &entry.kind {
            crate::core::orchestration::blackboard::EntryKind::Finding => "finding",
            crate::core::orchestration::blackboard::EntryKind::Challenge { .. } => "challenge",
            crate::core::orchestration::blackboard::EntryKind::Confirmation { .. } => {
                "confirmation"
            }
            crate::core::orchestration::blackboard::EntryKind::Synthesis { .. } => "synthesis",
            crate::core::orchestration::blackboard::EntryKind::Question => "question",
            crate::core::orchestration::blackboard::EntryKind::Answer { .. } => "answer",
        };
        let record = queries::BoardEntryRecord {
            run_id: run_id.clone(),
            agent: entry.agent.clone(),
            round: entry.round as i64,
            kind: kind_str.to_string(),
            content: entry.content.clone(),
            refs_json: serde_json::to_string(&entry.references).unwrap_or_default(),
            confidence: entry.confidence as f64,
            tokens_in: entry.tokens_used.input as i64,
            tokens_out: entry.tokens_used.output as i64,
        };
        if let Err(e) = queries::insert_board_entry(&db, record) {
            tracing::warn!("Failed to record board entry: {e}");
        }
    }
}

#[cfg(feature = "storage")]
fn record_orchestration_ring(
    token: &crate::core::orchestration::ring::RingToken,
    config: &crate::core::orchestration::ring::RingConfig,
    input: &str,
) {
    use crate::core::orchestration::ring::TokenStatus;
    use crate::storage::{init_db, queries};

    let db = match init_db() {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to init storage: {e}");
            return;
        }
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    let outcome_str = match token.status() {
        TokenStatus::Done { outcome } => serde_json::to_string(outcome).ok(),
        _ => None,
    };

    // 1. Parent run record
    let parent = queries::RunRecord {
        agent: "orchestration:ring".to_string(),
        input: input.to_string(),
        output: format!("{:?}", token.status()),
        provider: "orchestration".to_string(),
        model: String::new(),
        tokens_in: token.budget.used as i64,
        tokens_out: 0,
        cost: 0.0,
        duration_ms: 0,
        status: match token.status() {
            TokenStatus::Done { .. } => "done",
            _ => "incomplete",
        }
        .to_string(),
    };
    if let Err(e) = queries::insert_run_with_id(&db, &run_id, parent) {
        tracing::warn!("Failed to record orchestration parent run: {e}");
        return;
    }

    // 2. Orchestration metadata
    let orch = queries::OrchestrationRunRecord {
        run_id: run_id.clone(),
        pattern: "ring".to_string(),
        config_json: serde_json::to_string(config).unwrap_or_default(),
        outcome_json: outcome_str,
        rounds: token.lap as i64,
        halt_reason: None,
    };
    if let Err(e) = queries::insert_orchestration_run(&db, orch) {
        tracing::warn!("Failed to record orchestration metadata: {e}");
        return;
    }

    // 3. Contributions
    for c in token.contributions.iter() {
        let action_str = match &c.action {
            crate::core::orchestration::ring::ContributionAction::Propose => "propose",
            crate::core::orchestration::ring::ContributionAction::Enrich { .. } => "enrich",
            crate::core::orchestration::ring::ContributionAction::Contest { .. } => "contest",
            crate::core::orchestration::ring::ContributionAction::Endorse { .. } => "endorse",
            crate::core::orchestration::ring::ContributionAction::Synthesize => "synthesize",
            crate::core::orchestration::ring::ContributionAction::Pass { .. } => "pass",
        };
        let record = queries::RingContributionRecord {
            run_id: run_id.clone(),
            agent: c.agent.clone(),
            lap: c.lap as i64,
            position_in_lap: c.position_in_lap as i64,
            action: action_str.to_string(),
            content: c.content.clone(),
            reactions_json: serde_json::to_string(&c.reactions).unwrap_or_default(),
            tokens_in: c.tokens_used.input as i64,
            tokens_out: c.tokens_used.output as i64,
        };
        if let Err(e) = queries::insert_ring_contribution(&db, record) {
            tracing::warn!("Failed to record ring contribution: {e}");
        }
    }

    // 4. Votes
    for (agent, vote) in token.votes() {
        let record = queries::RingVoteRecord {
            run_id: run_id.clone(),
            agent: agent.clone(),
            position: vote.position.clone(),
            confidence: vote.confidence as f64,
            supports: serde_json::to_string(&vote.supporting_contributions).unwrap_or_default(),
            concerns: serde_json::to_string(&vote.unresolved_concerns).unwrap_or_default(),
        };
        if let Err(e) = queries::insert_ring_vote(&db, record) {
            tracing::warn!("Failed to record ring vote: {e}");
        }
    }
}

/// Check if an error indicates the model was not found (HTTP 404 or model-related 400).
fn is_model_not_found(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();

    // Google-style: HTTP 404 with "not found"
    if msg.contains("404") && msg.contains("not found") {
        return true;
    }

    // Anthropic-style: "model" + "not_found" or "invalid"
    if msg.contains("model") && (msg.contains("not_found") || msg.contains("invalid")) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_model_not_found_google_404() {
        let err = anyhow::anyhow!("HTTP 404: model gemini-3.0-pro not found");
        assert!(is_model_not_found(&err));
    }

    #[test]
    fn test_is_model_not_found_anthropic_400() {
        let err = anyhow::anyhow!("400 Bad Request: model not_found: claude-opus-next");
        assert!(is_model_not_found(&err));
    }

    #[test]
    fn test_is_model_not_found_auth_401_false() {
        let err = anyhow::anyhow!("401 Unauthorized: invalid API key");
        assert!(!is_model_not_found(&err));
    }

    #[test]
    fn test_is_model_not_found_rate_limit_429_false() {
        let err = anyhow::anyhow!("429 Too Many Requests: rate limit exceeded");
        assert!(!is_model_not_found(&err));
    }

    #[test]
    fn exit_code_mapping() {
        assert_eq!(exit_code_for(&anyhow::anyhow!("token budget exceeded")), 3);
        assert_eq!(
            exit_code_for(&anyhow::anyhow!("provider 'x' not available")),
            4
        );
        assert_eq!(exit_code_for(&anyhow::anyhow!("boom")), 1);
    }

    #[test]
    fn latest_auto_is_the_only_routed_value() {
        // concrete + latest:pro must NOT be treated as auto
        assert_ne!("claude-3", "latest:auto");
        assert_ne!("latest:pro", "latest:auto");
        // routing only triggers on the exact "latest:auto" string (guard documented)
    }

    #[test]
    fn test_resolve_agents_dir_returns_valid_resolution() {
        // resolve_agents_dir should not panic regardless of cwd state
        let resolution = resolve_agents_dir(false);
        match resolution {
            AgentResolution::Project { root, config } => {
                assert!(!root.to_string_lossy().is_empty());
                assert!(!config.agents.is_empty());
            }
            AgentResolution::Default(dir) => {
                assert!(!dir.to_string_lossy().is_empty());
            }
        }
    }
}
