use armadai_core::agent::Agent;
use armadai_core::config::AppPaths;
use armadai_core::parser::parse_agent_file;
use armadai_core::project;

pub async fn execute(agent_name: String) -> anyhow::Result<()> {
    let agent = load_named_agent(&agent_name)?;

    // Header
    let h = crate::cli::style::header();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!("{h}Agent:{h:#} {a}{}{a:#}", agent.name);
    anstream::println!("{m}Source: {}{m:#}", agent.source.display());
    println!();

    // Metadata table
    let h = crate::cli::style::header();
    anstream::println!("{h}## Metadata{h:#}");
    let m = crate::cli::style::muted();
    anstream::println!("{m}  Provider:      {m:#} {}", agent.metadata.provider);

    if let Some(ref model) = agent.metadata.model {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Model:         {m:#} {model}");
    }
    if let Some(ref command) = agent.metadata.command {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Command:       {m:#} {command}");
    }
    if !agent.metadata.model_fallback.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}  Fallbacks:     {m:#} [{}]",
            agent.metadata.model_fallback.join(", ")
        );
    }
    if let Some(ref args) = agent.metadata.args {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Args:          {m:#} [{}]", args.join(", "));
    }

    let m = crate::cli::style::muted();
    anstream::println!("{m}  Temperature:   {m:#} {}", agent.metadata.temperature);

    if let Some(max) = agent.metadata.max_tokens {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Max tokens:    {m:#} {max}");
    }
    if let Some(timeout) = agent.metadata.timeout {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Timeout:       {m:#} {timeout}s");
    }
    if !agent.metadata.tags.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}  Tags:          {m:#} [{}]",
            agent.metadata.tags.join(", ")
        );
    }
    if !agent.metadata.stacks.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}  Stacks:        {m:#} [{}]",
            agent.metadata.stacks.join(", ")
        );
    }
    if !agent.metadata.scope.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}  Scope:         {m:#} [{}]",
            agent.metadata.scope.join(", ")
        );
    }
    if let Some(cost) = agent.metadata.cost_limit {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Cost limit:    {m:#} ${cost:.2}");
    }
    if let Some(ref rate) = agent.metadata.rate_limit {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Rate limit:    {m:#} {rate}");
    }
    if let Some(ctx) = agent.metadata.context_window {
        let m = crate::cli::style::muted();
        anstream::println!("{m}  Context window:{m:#} {ctx}");
    }

    // System prompt
    println!();
    let h = crate::cli::style::header();
    anstream::println!("{h}## System Prompt{h:#}");
    for line in agent.system_prompt.lines() {
        println!("  {line}");
    }

    // Instructions
    if let Some(ref instructions) = agent.instructions {
        println!();
        let h = crate::cli::style::header();
        anstream::println!("{h}## Instructions{h:#}");
        for line in instructions.lines() {
            println!("  {line}");
        }
    }

    // Output format
    if let Some(ref format) = agent.output_format {
        println!();
        let h = crate::cli::style::header();
        anstream::println!("{h}## Output Format{h:#}");
        for line in format.lines() {
            println!("  {line}");
        }
    }

    // Pipeline
    if let Some(ref pipeline) = agent.pipeline {
        println!();
        let h = crate::cli::style::header();
        anstream::println!("{h}## Pipeline{h:#}");
        for next in &pipeline.next {
            let a = crate::cli::style::accent();
            let m = crate::cli::style::muted();
            anstream::println!("{m}  ->{m:#} {a}{next}{a:#}");
        }
    }

    // Context
    if let Some(ref context) = agent.context {
        println!();
        let h = crate::cli::style::header();
        anstream::println!("{h}## Context{h:#}");
        for line in context.lines() {
            println!("  {line}");
        }
    }

    Ok(())
}

/// Resolve an agent by name: check the project first (file-backed or
/// declared in `.armadai/agents.yaml`, via `agent_source::load_agent_by_name`),
/// then fall back to the default global agents directory — mirroring the
/// same two-tier fallback the previous path-only version used, just with a
/// richer project-side lookup that also covers declared agents.
fn load_named_agent(agent_name: &str) -> anyhow::Result<Agent> {
    if let Some((root, config)) = project::find_project_config() {
        let fragments = armadai_core::agent_source::project_fragments(&root);
        if let Ok(agent) =
            armadai_core::agent_source::load_agent_by_name(agent_name, &config, &root, &fragments)
        {
            return Ok(agent);
        }
    }

    // Fallback to default paths
    let paths = AppPaths::resolve();
    let path = Agent::find_file(&paths.agents_dir, agent_name).ok_or_else(|| {
        anyhow::anyhow!(
            "Agent '{agent_name}' not found in {}/ (looked for {agent_name}.md)",
            paths.agents_dir.display()
        )
    })?;
    parse_agent_file(&path)
}
