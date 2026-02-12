use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::core::agent::Agent;
use crate::core::config::AppPaths;
use crate::core::fleet::FleetDefinition;
use crate::core::project::{self, AgentRef, ProjectConfig};
use crate::providers::factory::create_provider;
use crate::providers::rate_limiter::RateLimiter;
use crate::providers::traits::{ChatMessage, CompletionRequest};

pub async fn execute(
    agent_name: String,
    input: Option<String>,
    pipe: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let resolution = resolve_agents_dir();

    // Build the execution chain: primary agent + piped agents
    let mut chain = vec![agent_name];
    if let Some(extra) = pipe {
        chain.extend(extra);
    }

    // Resolve input text
    let mut current_input = resolve_input(input).await?;

    for (i, name) in chain.iter().enumerate() {
        if chain.len() > 1 {
            eprintln!("--- [{}/{} {}] ---", i + 1, chain.len(), name);
        }

        let agent_path = resolve_agent_path(&resolution, name)?;
        let (output, _) = run_single_agent(&agent_path, name, &current_input).await?;
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
) -> anyhow::Result<(String, RunMetrics)> {
    // 1. Load agent
    let agent = crate::parser::parse_agent_file(agent_path)?;

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
        return AgentResolution::Project { root, config };
    }

    // 2. Check for legacy fleet file in cwd
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

#[cfg(test)]
mod tests {
    use super::*;

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
