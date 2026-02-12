use crate::core::agent::Agent;
use crate::core::config::AppPaths;
use crate::core::project;
use crate::parser;

pub async fn execute(tags: Option<Vec<String>>, stack: Option<String>) -> anyhow::Result<()> {
    let mut agents = load_agents()?;

    if agents.is_empty() {
        println!("No agents found.");
        println!("Create one with: armadai new --template basic <name>");
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

/// Load agents: if a project config is found, resolve only declared agents.
/// Otherwise, load all agents from the default directory.
fn load_agents() -> anyhow::Result<Vec<Agent>> {
    if let Some((root, config)) = project::find_project_config()
        && !config.agents.is_empty()
    {
        let (paths, errors) = project::resolve_all_agents(&config, &root);
        for err in &errors {
            eprintln!("  warn: {err}");
        }
        let mut agents = Vec::new();
        for path in &paths {
            match parser::parse_agent_file(path) {
                Ok(agent) => agents.push(agent),
                Err(e) => eprintln!("  warn: failed to parse {}: {e}", path.display()),
            }
        }
        return Ok(agents);
    }

    let paths = AppPaths::resolve();
    Agent::load_all(&paths.agents_dir)
}
