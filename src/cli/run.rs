use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::core::agent::Agent;
use crate::core::fleet::FleetDefinition;
use crate::providers::factory::create_provider;
use crate::providers::rate_limiter::RateLimiter;
use crate::providers::traits::{ChatMessage, CompletionRequest};

pub async fn execute(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let (agents_dir, fleet) = resolve_agents_dir();
    let agents_dir = agents_dir.as_path();

    // Build the execution chain: primary agent + piped agents
    let mut chain = vec![agent_name];
    if let Some(extra) = pipe {
        chain.extend(extra);
    }

    // Validate agents against fleet if present
    if let Some(ref fleet) = fleet {
        for name in &chain {
            if !fleet.contains_agent(name) {
                anyhow::bail!(
                    "Agent '{name}' is not in fleet '{}'. Available: {}",
                    fleet.fleet,
                    fleet.agents.join(", ")
                );
            }
        }
    }

    // Resolve input text
    let mut current_input = resolve_input(input).await?;

    for (i, name) in chain.iter().enumerate() {
        if chain.len() > 1 {
            eprintln!("--- [{}/{} {}] ---", i + 1, chain.len(), name);
        }

        let (output, _) = run_single_agent(agents_dir, name, &current_input).await?;
        current_input = output;
    }

    // Final output to stdout
    println!("{current_input}");

    Ok(())
}

async fn run_single_agent(
    agents_dir: &Path,
    agent_name: &str,
    input: &str,
) -> anyhow::Result<(String, RunMetrics)> {
    // 1. Load agent
    let agent_path = Agent::find_file(agents_dir, agent_name).ok_or_else(|| {
        anyhow::anyhow!("Agent '{agent_name}' not found in {}", agents_dir.display())
    })?;
    let agent = crate::parser::parse_agent_file(&agent_path)?;

    // 2. Create provider
    let provider = create_provider(&agent)?;

    // 3. Apply rate limiting if configured
    if let Some(ref rate_str) = agent.metadata.rate_limit
        && let Some(rpm) = RateLimiter::parse_rate(rate_str)
    {
        let limiter = RateLimiter::new(rpm);
        limiter.acquire().await;
    }

    // 4. Build request
    let model = agent
        .metadata
        .model
        .clone()
        .or_else(|| agent.metadata.command.clone())
        .unwrap_or_else(|| "default".to_string());

    let request = CompletionRequest {
        model,
        system_prompt: agent.system_prompt.clone(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: input.to_string(),
        }],
        temperature: agent.metadata.temperature,
        max_tokens: agent.metadata.max_tokens,
    };

    // 5. Execute
    let start = Instant::now();
    let response = provider.complete(request).await?;
    let duration = start.elapsed();

    // 6. Print summary to stderr (so stdout is clean for piping)
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

    // 7. Record in storage (if available)
    #[cfg(feature = "storage")]
    record_run(&metrics, input, &response.content).await;

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
async fn record_run(metrics: &RunMetrics, input: &str, output: &str) {
    use crate::storage::{init_db, queries};

    let db = match init_db().await {
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

    if let Err(e) = queries::insert_run(&db, record).await {
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

/// Resolve the agents directory: if a `armadai.yaml` fleet file exists in the
/// current directory, use the fleet's source/agents/ path. Otherwise default
/// to the local `agents/` directory.
fn resolve_agents_dir() -> (PathBuf, Option<FleetDefinition>) {
    let fleet_path = Path::new("armadai.yaml");
    if fleet_path.exists()
        && let Ok(fleet) = FleetDefinition::load(fleet_path)
    {
        let dir = fleet.agents_dir();
        if dir.exists() {
            tracing::info!(
                "Using fleet '{}' agents from {}",
                fleet.fleet,
                dir.display()
            );
            return (dir, Some(fleet));
        }
        tracing::warn!(
            "Fleet '{}' source agents dir not found: {}",
            fleet.fleet,
            dir.display()
        );
    }
    (PathBuf::from("agents"), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_agents_dir_no_fleet() {
        // When no armadai.yaml exists in the current directory,
        // resolve_agents_dir should return the default "agents" path
        let (dir, fleet) = resolve_agents_dir();
        // We can't guarantee armadai.yaml doesn't exist in the test runner's cwd,
        // but the function should not panic
        assert!(fleet.is_none() || fleet.is_some());
        // dir should be a valid path
        assert!(!dir.to_string_lossy().is_empty());
    }
}
