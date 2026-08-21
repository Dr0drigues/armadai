use armadai_core::agent::Agent;
use armadai_core::config::AppPaths;
use armadai_core::project;

pub async fn execute(tags: Option<Vec<String>>, stack: Option<String>) -> anyhow::Result<()> {
    let mut agents = load_agents()?;

    if agents.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No agents found.{m:#}");
        anstream::println!("{m}Create one with: armadai new --template basic <name>{m:#}");
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
        let m = crate::cli::style::muted();
        anstream::println!("{m}No agents match the given filters.{m:#}");
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
    let h = crate::cli::style::header();
    anstream::println!(
        "{h}  {:<name_w$}  {:<provider_w$}  {:<model_w$}  TAGS  STACKS{h:#}",
        "NAME",
        "PROVIDER",
        "MODEL",
    );
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  {:<name_w$}  {:<provider_w$}  {:<model_w$}  ----  ------{m:#}",
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

        let a = crate::cli::style::accent();
        anstream::println!(
            "  {a}{:<name_w$}{a:#}  {:<provider_w$}  {:<model_w$}  {}  {}",
            agent.name,
            agent.metadata.provider,
            agent.model_display(),
            tags_str,
            stacks_str,
        );
    }

    let m = crate::cli::style::muted();
    anstream::println!("\n{m}  {} agent(s) found.{m:#}", agents.len());
    Ok(())
}

/// Load agents: if a project config is found, resolve project agents
/// (file-backed and declared alike). Otherwise, load all agents from the
/// default directory. When `--global` is active, always load from the
/// global library.
///
/// A project counts as having agents when `agents:` lists any, OR
/// `.armadai/agents.yaml` declares any — every declared agent is included
/// automatically, so a project that only uses that format (an empty/absent
/// `agents:` list) must still take the project branch instead of falling
/// through to the global library.
fn load_agents() -> anyhow::Result<Vec<Agent>> {
    if !armadai_core::config::is_force_global()
        && let Some((root, config)) = project::find_project_config()
        && (!config.agents.is_empty()
            || armadai_core::agent_source::declarations_path(&root).is_file())
    {
        let fragments = armadai_core::agent_source::project_fragments(&root);
        let (agents, errors) =
            armadai_core::agent_source::load_all_agents(&config, &root, &fragments);
        for err in &errors {
            let w = crate::cli::style::warn();
            anstream::eprintln!("{w}  warn: {err}{w:#}");
        }
        return Ok(agents);
    }

    let paths = AppPaths::resolve();
    Agent::load_all(&paths.agents_dir)
}
