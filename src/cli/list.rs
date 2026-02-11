use std::path::Path;

use crate::core::agent::Agent;

pub async fn execute(tags: Option<Vec<String>>, stack: Option<String>) -> anyhow::Result<()> {
    let agents_dir = Path::new("agents");
    let mut agents = Agent::load_all(agents_dir)?;

    if agents.is_empty() {
        println!("No agents found in {}/", agents_dir.display());
        println!("Create one with: swarm new --template basic <name>");
        return Ok(());
    }

    // Apply filters
    if let Some(ref tags) = tags {
        agents.retain(|a| a.matches_tags(tags));
    }
    if let Some(ref stack) = stack {
        agents.retain(|a| a.matches_stack(stack));
    }

    if agents.is_empty() {
        println!("No agents match the given filters.");
        return Ok(());
    }

    // Compute column widths
    let name_w = agents
        .iter()
        .map(|a| a.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let provider_w = agents
        .iter()
        .map(|a| a.metadata.provider.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let model_w = agents
        .iter()
        .map(|a| a.model_display().len())
        .max()
        .unwrap_or(5)
        .max(5);

    // Header
    println!(
        "  {:<name_w$}  {:<provider_w$}  {:<model_w$}  TAGS  STACKS",
        "NAME", "PROVIDER", "MODEL",
    );
    println!(
        "  {:<name_w$}  {:<provider_w$}  {:<model_w$}  ----  ------",
        "-".repeat(name_w),
        "-".repeat(provider_w),
        "-".repeat(model_w),
    );

    // Rows
    for agent in &agents {
        let tags_str = if agent.metadata.tags.is_empty() {
            "-".to_string()
        } else {
            agent.metadata.tags.join(", ")
        };
        let stacks_str = if agent.metadata.stacks.is_empty() {
            "-".to_string()
        } else {
            agent.metadata.stacks.join(", ")
        };

        println!(
            "  {:<name_w$}  {:<provider_w$}  {:<model_w$}  {}  {}",
            agent.name,
            agent.metadata.provider,
            agent.model_display(),
            tags_str,
            stacks_str,
        );
    }

    println!("\n  {} agent(s) found.", agents.len());
    Ok(())
}
