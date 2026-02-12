use crate::core::config;
use crate::core::starter::{StarterPack, list_available_packs, starters_dir};

pub async fn execute(force: bool, project: bool, pack: Option<String>) -> anyhow::Result<()> {
    if let Some(ref pack_name) = pack
        && project
    {
        // Combined mode: install pack + create project config referencing it
        init_global(force)?;
        let installed_pack = install_pack(pack_name, force)?;
        init_project_with_pack(&installed_pack, pack_name)?;
        return Ok(());
    }

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

/// Install a starter pack by name. Returns the loaded pack definition.
fn install_pack(name: &str, force: bool) -> anyhow::Result<StarterPack> {
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

    Ok(pack)
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

/// Create an armadai.yaml pre-configured with the agents from a starter pack.
fn init_project_with_pack(pack: &StarterPack, pack_name: &str) -> anyhow::Result<()> {
    let path = std::path::Path::new("armadai.yaml");
    if path.exists() {
        anyhow::bail!("armadai.yaml already exists in current directory");
    }

    // Detect provider from the pack's agent files
    let link_target = detect_pack_provider(pack_name);

    let mut content = String::from(
        "# ArmadAI project configuration\n\
         # See: https://github.com/Dr0drigues/swarm-festai\n\n\
         # Agents used in this project\n\
         agents:\n",
    );

    for agent in &pack.agents {
        content.push_str(&format!("  - name: {agent}\n"));
    }

    // Prompts
    if !pack.prompts.is_empty() {
        content.push_str("\n# Composable prompt fragments\nprompts:\n");
        for prompt in &pack.prompts {
            content.push_str(&format!("  - name: {prompt}\n"));
        }
    } else {
        content.push_str("\nprompts: []\n");
    }

    content.push_str("\nskills: []\nsources: []\n");

    // Linker configuration
    if let Some(target) = link_target {
        content.push_str(&format!(
            "\n# Linker configuration\nlink:\n  target: {target}\n"
        ));
    }

    std::fs::write(path, &content)?;
    println!("\nCreated armadai.yaml with pack '{}' agents", pack.name);
    println!("  Run `armadai link` to generate target config files.");

    Ok(())
}

/// Try to detect the primary provider used by a pack's agents.
fn detect_pack_provider(pack_name: &str) -> Option<String> {
    let dir = starters_dir();
    let agents_dir = dir.join(pack_name).join("agents");
    if !agents_dir.is_dir() {
        return None;
    }

    let entries = std::fs::read_dir(&agents_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md")
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            for line in content.lines() {
                let trimmed = line.trim().trim_start_matches("- ");
                if let Some(provider) = trimmed.strip_prefix("provider:") {
                    let provider = provider.trim();
                    if !provider.is_empty() {
                        return Some(provider.to_string());
                    }
                }
            }
        }
    }

    None
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
