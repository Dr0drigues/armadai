use std::path::{Path, PathBuf};

use clap::Subcommand;
use dialoguer::MultiSelect;

use crate::core::agent::Agent;
use crate::core::config;
use crate::core::fleet::FleetDefinition;

const FLEET_FILENAME: &str = "armadai.yaml";

#[derive(Subcommand)]
pub enum FleetAction {
    /// Create a new fleet from available agents
    #[command(long_about = "Create a new fleet by selecting agents.\n\n\
            Agents are picked interactively or via --agents/--tags/--all flags.")]
    Create {
        /// Fleet name
        name: String,
        /// Agents to include (comma-separated)
        #[arg(long)]
        agents: Option<String>,
        /// Include agents matching these tags
        #[arg(long)]
        tags: Option<Vec<String>>,
        /// Include all agents
        #[arg(long)]
        all: bool,
    },
    /// Link a fleet to a project directory
    #[command(
        long_about = "Create a armadai.yaml in the target directory, linking it to a fleet.\n\n\
            By default links to the current directory (--local). Use --global to store in \
            ~/.config/armadai/fleets/ or --path for a specific directory.",
        after_help = "Examples:\n  \
            armadai fleet link my-fleet\n  \
            armadai fleet link my-fleet --global\n  \
            armadai fleet link my-fleet --path /projects/web-app"
    )]
    Link {
        /// Fleet name
        name: String,
        /// Link in current directory (default)
        #[arg(long, default_value_t = true)]
        local: bool,
        /// Link in global config (~/.config/armadai/fleets/)
        #[arg(long)]
        global: bool,
        /// Link in a specific directory
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// List all known fleets (local + global)
    List,
    /// Show details of a specific fleet
    Show {
        /// Fleet name
        name: String,
    },
}

pub async fn execute(action: FleetAction) -> anyhow::Result<()> {
    match action {
        FleetAction::Create {
            name,
            agents,
            tags,
            all,
        } => create_fleet(&name, agents.as_deref(), tags.as_deref(), all).await,
        FleetAction::Link {
            name, global, path, ..
        } => link_fleet(&name, global, path.as_deref()),
        FleetAction::List => list_fleets(),
        FleetAction::Show { name } => show_fleet(&name),
    }
}

/// Create a fleet definition file in the global config directory.
async fn create_fleet(
    name: &str,
    agents_csv: Option<&str>,
    tags: Option<&[String]>,
    all: bool,
) -> anyhow::Result<()> {
    let paths = config::AppPaths::resolve();
    let agents_dir = &paths.agents_dir;
    let available = Agent::load_all(agents_dir)?;

    if available.is_empty() {
        anyhow::bail!("No agents found in {}", agents_dir.display());
    }

    let selected_names: Vec<String> = if all {
        available.iter().map(|a| agent_stem(&a.source)).collect()
    } else if let Some(csv) = agents_csv {
        crate::cli::new::parse_comma_list(csv)
    } else if let Some(tag_filter) = tags {
        available
            .iter()
            .filter(|a| a.matches_tags(tag_filter))
            .map(|a| agent_stem(&a.source))
            .collect()
    } else {
        // Interactive selection
        let labels: Vec<String> = available
            .iter()
            .map(|a| {
                let stem = agent_stem(&a.source);
                let tags_str = if a.metadata.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", a.metadata.tags.join(", "))
                };
                format!("{stem}{tags_str}")
            })
            .collect();

        let selected = MultiSelect::new()
            .with_prompt("Select agents for the fleet")
            .items(&labels)
            .interact()?;

        if selected.is_empty() {
            anyhow::bail!("No agents selected");
        }

        selected
            .iter()
            .map(|&i| agent_stem(&available[i].source))
            .collect()
    };

    if selected_names.is_empty() {
        anyhow::bail!("No agents matched the criteria");
    }

    // Resolve source directory (current working directory assumed to be the armadai-festai root)
    let source = std::env::current_dir()?;

    let fleet = FleetDefinition {
        fleet: name.to_string(),
        agents: selected_names.clone(),
        source,
    };

    // Save to global config
    let fleet_path = global_fleet_path(name);
    fleet.save(&fleet_path)?;

    println!(
        "Fleet '{}' created with {} agents:",
        name,
        selected_names.len()
    );
    for a in &selected_names {
        println!("  - {a}");
    }
    println!("\nSaved to: {}", fleet_path.display());
    println!("Link it to a project: armadai fleet link {name}");

    Ok(())
}

/// Link a fleet to a directory by creating armadai.yaml.
fn link_fleet(name: &str, global: bool, path: Option<&Path>) -> anyhow::Result<()> {
    // Load fleet definition
    let fleet_path = global_fleet_path(name);
    if !fleet_path.exists() {
        // Maybe it's in the current directory?
        let local_check = Path::new(FLEET_FILENAME);
        if local_check.exists() {
            let def = FleetDefinition::load(local_check)?;
            if def.fleet == name {
                println!("Fleet '{name}' is already linked in current directory.");
                return Ok(());
            }
        }
        anyhow::bail!("Fleet '{name}' not found. Create it first: armadai fleet create {name}");
    }

    let fleet = FleetDefinition::load(&fleet_path)?;

    let target = if global {
        global_fleet_dir().join(format!("{name}.yaml"))
    } else if let Some(p) = path {
        p.join(FLEET_FILENAME)
    } else {
        PathBuf::from(FLEET_FILENAME)
    };

    fleet.save(&target)?;

    println!("Fleet '{name}' linked: {}", target.display());
    println!("  Agents: {}", fleet.agents.join(", "));
    println!("  Source: {}", fleet.source.display());

    Ok(())
}

/// List all known fleets (local + global).
fn list_fleets() -> anyhow::Result<()> {
    let mut found = false;

    // Check local
    let local_path = Path::new(FLEET_FILENAME);
    if local_path.exists()
        && let Ok(fleet) = FleetDefinition::load(local_path)
    {
        println!("Local fleet:");
        print_fleet_summary(&fleet, "  ");
        found = true;
    }

    // Check global
    let global_dir = global_fleet_dir();
    if global_dir.exists() {
        let mut global_fleets = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&global_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "yaml")
                    && let Ok(fleet) = FleetDefinition::load(&path)
                {
                    global_fleets.push(fleet);
                }
            }
        }
        if !global_fleets.is_empty() {
            if found {
                println!();
            }
            println!("Global fleets:");
            for fleet in &global_fleets {
                print_fleet_summary(fleet, "  ");
            }
            found = true;
        }
    }

    if !found {
        println!("No fleets found.");
        println!("Create one: armadai fleet create <name>");
    }

    Ok(())
}

/// Show details of a specific fleet.
fn show_fleet(name: &str) -> anyhow::Result<()> {
    // Try local first
    let local_path = Path::new(FLEET_FILENAME);
    if local_path.exists()
        && let Ok(fleet) = FleetDefinition::load(local_path)
        && fleet.fleet == name
    {
        print_fleet_detail(&fleet)?;
        return Ok(());
    }

    // Try global
    let fleet_path = global_fleet_path(name);
    if fleet_path.exists() {
        let fleet = FleetDefinition::load(&fleet_path)?;
        print_fleet_detail(&fleet)?;
        return Ok(());
    }

    anyhow::bail!("Fleet '{name}' not found");
}

fn print_fleet_summary(fleet: &FleetDefinition, indent: &str) {
    println!("{indent}{} ({} agents)", fleet.fleet, fleet.agents.len());
    println!("{indent}  source: {}", fleet.source.display());
}

fn print_fleet_detail(fleet: &FleetDefinition) -> anyhow::Result<()> {
    println!("Fleet: {}", fleet.fleet);
    println!("Source: {}", fleet.source.display());
    println!("Agents ({}):", fleet.agents.len());

    let agents_dir = fleet.agents_dir();
    for name in &fleet.agents {
        let status = match Agent::find_file(&agents_dir, name)
            .and_then(|p| crate::parser::parse_agent_file(&p).ok())
        {
            Some(agent) => {
                format!(
                    "{} [{}]",
                    agent.model_display(),
                    agent.metadata.tags.join(", ")
                )
            }
            None => "MISSING".to_string(),
        };
        println!("  - {name}: {status}");
    }

    Ok(())
}

/// Get the global fleet config directory.
fn global_fleet_dir() -> PathBuf {
    config::user_fleets_dir()
}

/// Get the path for a specific fleet's global config file.
fn global_fleet_path(name: &str) -> PathBuf {
    global_fleet_dir().join(format!("{name}.yaml"))
}

/// Extract the agent stem (filename without .md) from a path.
fn agent_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}
