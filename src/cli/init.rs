use crate::core::config;

pub async fn execute(force: bool, project: bool) -> anyhow::Result<()> {
    if project {
        return init_project();
    }
    init_global(force)
}

/// Create the global config directory and default files.
fn init_global(force: bool) -> anyhow::Result<()> {
    let dir = config::config_dir();

    // Create directory tree
    config::ensure_config_dirs()?;
    println!("Created config directory: {}", dir.display());

    // Write config.yaml
    let config_path = config::config_file_path();
    write_if_missing_or_force(&config_path, config::DEFAULT_CONFIG_YAML, force)?;

    // Write providers.yaml
    let providers_path = config::providers_file_path();
    write_if_missing_or_force(&providers_path, config::DEFAULT_PROVIDERS_YAML, force)?;

    println!("\nArmadAI initialized at {}", dir.display());
    println!("  config:    {}", config_path.display());
    println!("  providers: {}", providers_path.display());
    println!("  agents:    {}", config::user_agents_dir().display());
    println!("  fleets:    {}", config::user_fleets_dir().display());
    println!("  prompts:   {}", config::user_prompts_dir().display());
    println!("  skills:    {}", config::user_skills_dir().display());
    println!("  registry:  {}", config::registry_cache_dir().display());

    Ok(())
}

/// Create a minimal armadai.yaml in the current directory.
fn init_project() -> anyhow::Result<()> {
    let path = std::path::Path::new("armadai.yaml");
    if path.exists() {
        anyhow::bail!("armadai.yaml already exists in current directory");
    }

    let content = "\
# ArmadAI project configuration
# See: https://github.com/Dr0drigues/swarm-festai

# Agents directory (relative to this file)
agents_dir: agents

# Templates directory
templates_dir: templates
";

    std::fs::write(path, content)?;
    println!("Created armadai.yaml in current directory");
    println!("  Edit it to configure project-local settings.");

    Ok(())
}

fn write_if_missing_or_force(
    path: &std::path::Path,
    content: &str,
    force: bool,
) -> anyhow::Result<()> {
    if path.exists() && !force {
        println!("  skip (exists): {}", path.display());
        return Ok(());
    }
    std::fs::write(path, content)?;
    if force && path.exists() {
        println!("  overwritten:   {}", path.display());
    } else {
        println!("  created:       {}", path.display());
    }
    Ok(())
}
