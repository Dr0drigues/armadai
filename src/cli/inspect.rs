use crate::core::agent::Agent;
use crate::core::config::AppPaths;
use crate::core::project::{self, AgentRef};
use crate::parser::parse_agent_file;

pub async fn execute(agent_name: String) -> anyhow::Result<()> {
    let path = resolve_agent_file(&agent_name)?;
    let agent = parse_agent_file(&path)?;

    // Header
    println!("Agent: {}", agent.name);
    println!("Source: {}", agent.source.display());
    println!();

    // Metadata table
    println!("## Metadata");
    println!("  Provider:       {}", agent.metadata.provider);

    if let Some(ref model) = agent.metadata.model {
        println!("  Model:          {model}");
    }
    if let Some(ref command) = agent.metadata.command {
        println!("  Command:        {command}");
    }
    if let Some(ref args) = agent.metadata.args {
        println!("  Args:           [{}]", args.join(", "));
    }

    println!("  Temperature:    {}", agent.metadata.temperature);

    if let Some(max) = agent.metadata.max_tokens {
        println!("  Max tokens:     {max}");
    }
    if let Some(timeout) = agent.metadata.timeout {
        println!("  Timeout:        {timeout}s");
    }
    if !agent.metadata.tags.is_empty() {
        println!("  Tags:           [{}]", agent.metadata.tags.join(", "));
    }
    if !agent.metadata.stacks.is_empty() {
        println!("  Stacks:         [{}]", agent.metadata.stacks.join(", "));
    }
    if let Some(cost) = agent.metadata.cost_limit {
        println!("  Cost limit:     ${cost:.2}");
    }
    if let Some(ref rate) = agent.metadata.rate_limit {
        println!("  Rate limit:     {rate}");
    }
    if let Some(ctx) = agent.metadata.context_window {
        println!("  Context window: {ctx}");
    }

    // System prompt
    println!();
    println!("## System Prompt");
    for line in agent.system_prompt.lines() {
        println!("  {line}");
    }

    // Instructions
    if let Some(ref instructions) = agent.instructions {
        println!();
        println!("## Instructions");
        for line in instructions.lines() {
            println!("  {line}");
        }
    }

    // Output format
    if let Some(ref format) = agent.output_format {
        println!();
        println!("## Output Format");
        for line in format.lines() {
            println!("  {line}");
        }
    }

    // Pipeline
    if let Some(ref pipeline) = agent.pipeline {
        println!();
        println!("## Pipeline");
        for next in &pipeline.next {
            println!("  -> {next}");
        }
    }

    // Context
    if let Some(ref context) = agent.context {
        println!();
        println!("## Context");
        for line in context.lines() {
            println!("  {line}");
        }
    }

    Ok(())
}

/// Resolve an agent by name: check project config first, then default paths.
fn resolve_agent_file(agent_name: &str) -> anyhow::Result<std::path::PathBuf> {
    if let Some((root, config)) = project::find_project_config() {
        // Try to find the agent in the project config
        if let Some(agent_ref) = config.agents.iter().find(|r| match r {
            AgentRef::Named { name } => name == agent_name,
            AgentRef::Path { path } => path.file_stem().is_some_and(|s| s == agent_name),
            AgentRef::Registry { registry } => registry.ends_with(agent_name),
        }) {
            return project::resolve_agent(agent_ref, &root);
        }

        // Not in config, but try resolving as Named from project root
        let fallback = AgentRef::Named {
            name: agent_name.to_string(),
        };
        if let Ok(path) = project::resolve_agent(&fallback, &root) {
            return Ok(path);
        }
    }

    // Fallback to default paths
    let paths = AppPaths::resolve();
    Agent::find_file(&paths.agents_dir, agent_name).ok_or_else(|| {
        anyhow::anyhow!(
            "Agent '{agent_name}' not found in {}/ (looked for {agent_name}.md)",
            paths.agents_dir.display()
        )
    })
}
