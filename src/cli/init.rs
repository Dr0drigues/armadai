use crate::core::config;
use crate::core::starter::{StarterPack, list_available_packs, starters_dir};

pub async fn execute(force: bool, project: bool, pack: Option<String>) -> anyhow::Result<()> {
    if project {
        return init_project();
    }

    // Always init global config first
    init_global(force)?;

    // Install starter pack if requested
    if let Some(pack_name) = pack {
        install_pack(&pack_name, force)?;
    }

    Ok(())
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

/// Install a starter pack by name.
fn install_pack(name: &str, force: bool) -> anyhow::Result<()> {
    let dir = starters_dir();
    let pack_dir = dir.join(name);

    if !pack_dir.is_dir() {
        let available = list_available_packs();
        if available.is_empty() {
            anyhow::bail!(
                "Starter pack '{name}' not found. No packs available in {}",
                dir.display()
            );
        } else {
            anyhow::bail!(
                "Starter pack '{name}' not found. Available packs: {}",
                available.join(", ")
            );
        }
    }

    let pack = StarterPack::load(&pack_dir)?;
    println!(
        "\nInstalling starter pack: {} — {}",
        pack.name, pack.description
    );

    let (agents, prompts) = pack.install(&pack_dir, force)?;
    println!(
        "\nPack '{}' installed: {} agent(s), {} prompt(s)",
        pack.name, agents, prompts
    );

    Ok(())
}

/// Create an armadai.yaml in the current directory using the new project format.
fn init_project() -> anyhow::Result<()> {
    let path = std::path::Path::new("armadai.yaml");
    if path.exists() {
        anyhow::bail!("armadai.yaml already exists in current directory");
    }

    let content = "\
# ArmadAI project configuration
# See: https://github.com/Dr0drigues/swarm-festai

# Agents used in this project
agents:
  # - name: code-reviewer           # Named agent from user library
  # - registry: official/security   # Agent from the registry
  # - path: .armadai/agents/team.md # Local agent file

# Composable prompt fragments
prompts: []
  # - name: rust-conventions
  # - path: .armadai/prompts/style.md

# Skills (Agent Skills open standard)
skills: []
  # - name: docker-compose
  # - path: .armadai/skills/deploy/

# Context files injected into agent runs
sources: []
  # - docs/architecture.md
  # - CONTRIBUTING.md

# Linker configuration
# link:
#   target: claude
#   overrides:
#     claude:
#       output: .claude/
#     copilot:
#       output: .github/agents/
";

    std::fs::write(path, content)?;
    println!("Created armadai.yaml in current directory");
    println!("  Edit it to declare agents, prompts, skills and link targets.");

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
