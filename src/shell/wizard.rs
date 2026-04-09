//! Project setup wizard for the shell.
//!
//! Guides the user through project initialization and linking before entering the shell.

use anyhow::Result;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::core::project;
use crate::core::starter::{find_pack_dir, list_available_packs};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WizardResult {
    pub provider_command: String,
    pub provider_args: Vec<String>,
    pub model_name: String,
    pub project_name: String,
}

/// Check project readiness and run wizard if needed.
/// Returns the provider configuration to use, or an error if setup was cancelled.
pub fn ensure_project_ready() -> Result<WizardResult> {
    // Step 1: Check project state
    let project_state = detect_project();

    // Step 2: Initialize project if needed
    match project_state {
        ProjectState::NoProject => {
            if !prompt_init()? {
                return Err(anyhow::anyhow!("Project setup cancelled by user"));
            }
        }
        ProjectState::GitRepoNoConfig => {
            if !prompt_init()? {
                return Err(anyhow::anyhow!("Project setup cancelled by user"));
            }
        }
        ProjectState::Configured => {
            // Project already configured, continue
        }
    }

    // Step 3: Check for existing link
    if let Some(linked) = detect_link() {
        // Link exists, use it
        return build_wizard_result(&linked.name);
    }

    // Step 4: Prompt for link if needed
    let provider = prompt_link()?;

    // Check auth
    if !check_auth(&provider) {
        eprintln!("\nWarning: '{}' command not found in PATH.", provider);
        eprintln!("Make sure it is installed and available before using the shell.");
    }

    build_wizard_result(&provider)
}

// ---------------------------------------------------------------------------
// Project detection
// ---------------------------------------------------------------------------

enum ProjectState {
    Configured,
    GitRepoNoConfig,
    NoProject,
}

fn detect_project() -> ProjectState {
    if project::find_project_config().is_some() {
        return ProjectState::Configured;
    }

    if Path::new(".git").exists() {
        return ProjectState::GitRepoNoConfig;
    }

    ProjectState::NoProject
}

// ---------------------------------------------------------------------------
// Link detection
// ---------------------------------------------------------------------------

struct LinkedProvider {
    name: String,
    #[allow(dead_code)]
    path: String,
}

fn detect_link() -> Option<LinkedProvider> {
    let checks = [
        (".gemini", "gemini"),
        (".claude", "claude"),
        (".github/copilot-instructions.md", "copilot"),
        (".codex", "codex"),
    ];

    for (path, provider) in checks {
        if Path::new(path).exists() {
            return Some(LinkedProvider {
                name: provider.to_string(),
                path: path.to_string(),
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Auth check
// ---------------------------------------------------------------------------

fn check_auth(provider: &str) -> bool {
    is_command_available(provider)
}

fn is_command_available(command: &str) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("which")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("where")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = command;
        false
    }
}

// ---------------------------------------------------------------------------
// Interactive prompts
// ---------------------------------------------------------------------------

fn prompt_init() -> Result<bool> {
    println!("\nArmadAI Shell — Project Setup\n");
    println!("No ArmadAI config found in this directory.\n");
    println!("Would you like to initialize a project?");
    println!("  1) Quick setup with a starter pack");
    println!("  2) Skip (use system-wide agents only)");

    let choice = read_choice(1, 2)?;

    if choice == 2 {
        return Ok(false);
    }

    // Choice 1: starter pack
    let packs = list_available_packs();
    if packs.is_empty() {
        eprintln!("\nNo starter packs available.");
        return Ok(false);
    }

    println!("\nAvailable starter packs:");
    for (i, pack) in packs.iter().enumerate() {
        println!("  {}) {}", i + 1, pack);
    }

    let pack_choice = read_choice(1, packs.len())?;
    let pack_name = &packs[pack_choice - 1];

    // Run init with pack
    run_init_with_pack(pack_name)?;

    Ok(true)
}

fn prompt_link() -> Result<String> {
    println!("\nNo link found. Which AI assistant do you use?");
    println!("  1) Gemini CLI");
    println!("  2) Claude Code");
    println!("  3) GitHub Copilot");
    println!("  4) Codex");
    println!("  5) Skip");

    let choice = read_choice(1, 5)?;

    if choice == 5 {
        return Err(anyhow::anyhow!("Link setup skipped by user"));
    }

    let target = match choice {
        1 => "gemini",
        2 => "claude",
        3 => "copilot",
        4 => "codex",
        _ => unreachable!(),
    };

    // Run link
    run_link(target)?;

    Ok(target.to_string())
}

fn read_choice(min: usize, max: usize) -> Result<usize> {
    print!("\nChoice [1]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let input = input.trim();
    if input.is_empty() {
        return Ok(1);
    }

    let choice: usize = input.parse().map_err(|_| {
        anyhow::anyhow!(
            "Invalid input: expected a number between {} and {}",
            min,
            max
        )
    })?;

    if choice < min || choice > max {
        return Err(anyhow::anyhow!(
            "Choice out of range: expected {} to {}",
            min,
            max
        ));
    }

    Ok(choice)
}

// ---------------------------------------------------------------------------
// Init/Link execution
// ---------------------------------------------------------------------------

fn run_init_with_pack(pack_name: &str) -> Result<()> {
    use crate::core::config;
    use crate::core::starter::StarterPack;

    // Init global config
    config::ensure_config_dirs()?;

    // Install pack
    let pack_dir = find_pack_dir(pack_name)
        .ok_or_else(|| anyhow::anyhow!("Starter pack '{}' not found", pack_name))?;

    let pack = StarterPack::load(&pack_dir)?;
    println!(
        "\nInstalling starter pack: {} — {}",
        pack.name, pack.description
    );

    let (agents, prompts, skills) = pack.install(&pack_dir, false)?;
    println!(
        "Pack '{}' installed: {} agent(s), {} prompt(s), {} skill(s)",
        pack.name, agents, prompts, skills
    );

    // Create project config
    let dotarmadai = Path::new(".armadai");
    let dotarmadai_config = dotarmadai.join("config.yaml");

    if dotarmadai_config.exists() {
        println!("\n.armadai/config.yaml already exists, skipping project init");
        return Ok(());
    }

    // Create directory structure
    for subdir in &["agents", "prompts", "skills", "starters"] {
        std::fs::create_dir_all(dotarmadai.join(subdir))?;
    }

    let content = crate::cli::init::generate_project_yaml(&pack, pack_name);
    std::fs::write(&dotarmadai_config, &content)?;
    println!(
        "\nCreated .armadai/config.yaml with pack '{}' agents",
        pack.name
    );

    Ok(())
}

fn run_link(target: &str) -> Result<()> {
    println!("\nLinking to '{}'...", target);

    // Reuse link logic from cli/link.rs
    // We need to call it synchronously, so we'll use a minimal version here

    let (root, config) = project::find_project_config()
        .ok_or_else(|| anyhow::anyhow!("No project config found after initialization"))?;

    if config.agents.is_empty() {
        return Err(anyhow::anyhow!("No agents declared in project config"));
    }

    // Resolve agents
    let (paths, errors) = project::resolve_all_agents(&config, &root);
    for err in &errors {
        eprintln!("  warn: {}", err);
    }

    let mut link_agents: Vec<crate::linker::LinkAgent> = Vec::new();
    for path in &paths {
        match crate::parser::parse_agent_file(path) {
            Ok(agent) => link_agents.push(crate::linker::LinkAgent::from(&agent)),
            Err(e) => eprintln!("  warn: failed to parse {}: {}", path.display(), e),
        }
    }

    if link_agents.is_empty() {
        return Err(anyhow::anyhow!("No agents could be resolved"));
    }

    // Resolve deprecated models
    for agent in &mut link_agents {
        crate::linker::model_aliases::resolve_model_deprecations(
            &mut agent.model,
            &mut agent.model_fallback,
        );
    }

    // Create linker
    let linker = crate::linker::create_linker(target)?;

    // Generate files
    let sources = &config.sources;
    let files = linker.generate(&link_agents, None, sources);

    if files.is_empty() {
        return Err(anyhow::anyhow!("No files to generate"));
    }

    // Resolve output paths
    let output_dir = PathBuf::from(linker.default_output_dir());
    let default_dir = PathBuf::from(linker.default_output_dir());

    for file in &files {
        let relative = file.path.strip_prefix(&default_dir).unwrap_or(&file.path);
        let final_path = root.join(&output_dir).join(relative);

        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&final_path, &file.content)?;
        println!("  wrote {}", final_path.display());
    }

    println!("\nLinked {} agent(s) to '{}'", link_agents.len(), target);

    Ok(())
}

// ---------------------------------------------------------------------------
// Result builder
// ---------------------------------------------------------------------------

fn build_wizard_result(provider: &str) -> Result<WizardResult> {
    let (command, args) = match provider {
        "gemini" => ("gemini", vec!["-p".to_string()]),
        "claude" => ("claude", vec![]),
        "aider" => ("aider", vec!["--yes".to_string()]),
        "codex" => ("codex", vec![]),
        _ => (provider, vec![]),
    };

    let model_name = detect_model_name(command);
    let project_name = detect_project_name();

    Ok(WizardResult {
        provider_command: command.to_string(),
        provider_args: args,
        model_name,
        project_name,
    })
}

fn detect_model_name(command: &str) -> String {
    match command {
        "gemini" => {
            // Try to read model from .gemini/settings.json
            if let Ok(content) = std::fs::read_to_string(".gemini/settings.json")
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
                && let Some(model) = json.get("model").and_then(|m| m.as_str())
            {
                return model.to_string();
            }
            "gemini-2.5-flash".to_string()
        }
        "claude" => "claude-sonnet-4-5".to_string(),
        "aider" => "gpt-4o".to_string(),
        "codex" => "codex".to_string(),
        _ => "unknown".to_string(),
    }
}

fn detect_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}
