use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::core::agent::{Agent, AgentMode};
use crate::core::config::AppPaths;
use crate::core::fleet::FleetDefinition;
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

pub async fn execute(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
    orchestrate: Option<String>,
) -> anyhow::Result<()> {
    let resolution = resolve_agents_dir();

    // Build the execution chain: primary agent + piped agents
    let mut chain = vec![agent_name];
    if let Some(extra) = pipe {
        chain.extend(extra);
    }

    // Resolve input text
    let current_input = resolve_input(input).await?;

    // Orchestrated multi-agent execution
    if let Some(pattern) = orchestrate {
        if chain.len() < 2 {
            anyhow::bail!("--orchestrate requires at least 2 agents (use --pipe to add more)");
        }
        return run_orchestrated(&resolution, &chain, &current_input, &pattern).await;
    }

    // Standard sequential execution (backward compatible)
    let mut current_input = current_input;
    let project_defaults = match &resolution {
        AgentResolution::Project { config, .. } => Some(&config.defaults),
        _ => None,
    };

    for (i, name) in chain.iter().enumerate() {
        if chain.len() > 1 {
            eprintln!("--- [{}/{} {}] ---", i + 1, chain.len(), name);
        }

        let agent_path = resolve_agent_path(&resolution, name)?;
        let (output, _) =
            run_single_agent(&agent_path, name, &current_input, project_defaults).await?;
        current_input = output;
    }

    // Final output to stdout
    println!("{current_input}");

    Ok(())
}

/// Result of resolving the agents directory / project config.
enum AgentResolution {
    /// New-format project config with walk-up root
    Project {
        root: PathBuf,
        config: ProjectConfig,
    },
    /// Legacy fleet format
    Fleet(FleetDefinition),
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
        AgentResolution::Fleet(fleet) => {
            if !fleet.contains_agent(agent_name) {
                anyhow::bail!(
                    "Agent '{agent_name}' is not in fleet '{}'. Available: {}",
                    fleet.fleet,
                    fleet.agents.join(", ")
                );
            }
            let agents_dir = fleet.agents_dir();
            Agent::find_file(&agents_dir, agent_name).ok_or_else(|| {
                anyhow::anyhow!("Agent '{agent_name}' not found in {}", agents_dir.display())
            })
        }
        AgentResolution::Default(agents_dir) => Agent::find_file(agents_dir, agent_name)
            .ok_or_else(|| {
                anyhow::anyhow!("Agent '{agent_name}' not found in {}", agents_dir.display())
            }),
    }
}

async fn run_single_agent(
    agent_path: &Path,
    agent_name: &str,
    input: &str,
    project_defaults: Option<&ProjectDefaults>,
) -> anyhow::Result<(String, RunMetrics)> {
    // 1. Load agent
    let mut agent = crate::parser::parse_agent_file(agent_path)?;

    // 1b. Resolve deprecated model aliases
    crate::linker::model_aliases::resolve_model_deprecations(
        &mut agent.metadata.model,
        &mut agent.metadata.model_fallback,
    );
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
    let model = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

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
fn resolve_agents_dir() -> AgentResolution {
    // 1. Walk-up search for project config (new or legacy format)
    if let Some((root, config)) = project::find_project_config()
        && !config.agents.is_empty()
    {
        tracing::info!(
            "Using project config from {} ({} agent(s))",
            root.display(),
            config.agents.len()
        );
        let _ = crate::core::project_registry::register_project(&root);
        crate::core::model_updater::auto_check_and_prompt(&root, !atty_is_pipe());
        return AgentResolution::Project { root, config };
    }

    // 2. Check for legacy fleet file in cwd (deprecated)
    let fleet_path = Path::new("armadai.yaml");
    if fleet_path.exists()
        && let Ok(fleet) = FleetDefinition::load(fleet_path)
    {
        let dir = fleet.agents_dir();
        if dir.exists() {
            tracing::warn!(
                "Using deprecated fleet format from '{}'. \
                 Migrate to the modern armadai.yaml format (see `armadai init --project`).",
                fleet.fleet
            );
            return AgentResolution::Fleet(fleet);
        }
        tracing::warn!(
            "Fleet '{}' source agents dir not found: {}",
            fleet.fleet,
            dir.display()
        );
    }

    // 3. Default fallback
    AgentResolution::Default(AppPaths::resolve().agents_dir)
}

/// Run orchestrated multi-agent execution (blackboard or ring).
async fn run_orchestrated(
    resolution: &AgentResolution,
    agent_names: &[String],
    input: &str,
    pattern: &str,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use crate::core::orchestration::blackboard::{
        BlackboardConfig, Board, BoardAgent, run_blackboard,
    };
    use crate::core::orchestration::llm_agents::{LlmBoardAgent, LlmRingAgent};
    use crate::core::orchestration::ring::{
        RingAgent, RingConfig, RingOutcome, RingToken, TokenStatus, run_ring,
    };
    use crate::providers::traits::Provider;

    // Load all agents and create providers
    let mut agents = Vec::new();
    let mut providers: Vec<Arc<dyn Provider>> = Vec::new();

    for name in agent_names {
        let agent_path = resolve_agent_path(resolution, name)?;
        let mut agent = crate::parser::parse_agent_file(&agent_path)?;
        crate::linker::model_aliases::resolve_model_deprecations(
            &mut agent.metadata.model,
            &mut agent.metadata.model_fallback,
        );
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

            let config = BlackboardConfig::default();
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

            for entry in board.entries() {
                println!("[{}] {}", entry.agent, entry.content);
            }
        }
        "ring" => {
            let ring_agents: Vec<Arc<dyn RingAgent>> = agents
                .into_iter()
                .map(|a| Arc::new(LlmRingAgent::new(a)) as Arc<dyn RingAgent>)
                .collect();

            let agent_order: Vec<String> =
                ring_agents.iter().map(|a| a.name().to_string()).collect();

            let config = RingConfig::default();
            let mut token = RingToken::new(input.to_string(), agent_order, config.token_budget);

            eprintln!(
                "[ring] Starting with {} agent(s), max {} laps",
                ring_agents.len(),
                config.max_laps
            );

            run_ring(&mut token, &ring_agents, &providers, &config).await?;

            #[cfg(feature = "storage")]
            record_orchestration_ring(&token, &config, input);

            match token.status() {
                TokenStatus::Done { outcome } => match outcome {
                    RingOutcome::Consensus {
                        resolution, score, ..
                    } => {
                        eprintln!("[ring] Consensus ({:.0}%)", score * 100.0);
                        println!("{resolution}");
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
                        println!("{resolution}");
                    }
                    RingOutcome::NoConsensus { summary, .. } => {
                        eprintln!("[ring] No consensus");
                        println!("{summary}");
                    }
                    RingOutcome::BudgetExhausted { partial_summary } => {
                        eprintln!("[ring] Budget exhausted");
                        println!("{partial_summary}");
                    }
                    RingOutcome::Cancelled => {
                        eprintln!("[ring] Cancelled");
                    }
                },
                other => {
                    eprintln!("[ring] Unexpected status: {other:?}");
                }
            }
        }
        other => {
            anyhow::bail!("Unknown orchestration pattern: '{other}'. Use 'blackboard' or 'ring'");
        }
    }

    Ok(())
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
    fn test_resolve_agents_dir_returns_valid_resolution() {
        // resolve_agents_dir should not panic regardless of cwd state
        let resolution = resolve_agents_dir();
        match resolution {
            AgentResolution::Project { root, config } => {
                assert!(!root.to_string_lossy().is_empty());
                assert!(!config.agents.is_empty());
            }
            AgentResolution::Fleet(fleet) => {
                assert!(!fleet.fleet.is_empty());
            }
            AgentResolution::Default(dir) => {
                assert!(!dir.to_string_lossy().is_empty());
            }
        }
    }
}
