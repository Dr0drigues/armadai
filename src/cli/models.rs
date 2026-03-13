use clap::Subcommand;

use crate::core::model_updater;
use crate::core::project;
use crate::core::project_registry;

#[derive(Subcommand)]
pub enum ModelsAction {
    /// Check for deprecated models in agent files
    Check {
        /// Check all registered projects
        #[arg(long)]
        all: bool,
        /// Remove stale projects from the registry (with --all)
        #[arg(long)]
        prune: bool,
    },
    /// Update deprecated models in agent files in-place
    Update {
        /// Update all registered projects
        #[arg(long)]
        all: bool,
    },
    /// List registered projects
    List,
}

pub async fn execute(action: ModelsAction) -> anyhow::Result<()> {
    match action {
        ModelsAction::Check { all, prune } => check(all, prune),
        ModelsAction::Update { all } => update(all),
        ModelsAction::List => list(),
    }
}

fn check(all: bool, prune: bool) -> anyhow::Result<()> {
    if all {
        let mut registry = project_registry::load();

        if prune {
            let pruned = project_registry::prune_stale(&mut registry);
            if !pruned.is_empty() {
                println!("Pruned {} stale project(s):", pruned.len());
                for p in &pruned {
                    println!("  - {p}");
                }
                project_registry::save(&registry)?;
                println!();
            }
        }

        if registry.projects.is_empty() {
            println!(
                "No registered projects. Run `armadai run` or `armadai link` in a project first."
            );
            return Ok(());
        }

        let mut total = 0;
        for entry in &registry.projects {
            let findings = match model_updater::check_project(std::path::Path::new(&entry.path)) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("  warn: {}: {e}", entry.path);
                    continue;
                }
            };
            if !findings.is_empty() {
                println!("{}:", entry.path);
                print_findings(&findings);
                total += findings.len();
            }
        }

        if total == 0 {
            println!(
                "All models are up to date across {} project(s).",
                registry.projects.len()
            );
        } else {
            println!(
                "\n{total} deprecated model(s) found. Run `armadai models update --all` to fix."
            );
        }
    } else {
        let (root, _config) = project::find_project_config().ok_or_else(|| {
            anyhow::anyhow!("No project config found. Run from a project directory or use --all.")
        })?;

        let findings = model_updater::check_project(&root)?;
        if findings.is_empty() {
            println!("All models are up to date.");
        } else {
            print_findings(&findings);
            println!(
                "\n{} deprecated model(s) found. Run `armadai models update` to fix.",
                findings.len()
            );
        }
    }

    Ok(())
}

fn update(all: bool) -> anyhow::Result<()> {
    if all {
        let registry = project_registry::load();
        if registry.projects.is_empty() {
            println!("No registered projects.");
            return Ok(());
        }

        let mut total_updated = 0;
        for entry in &registry.projects {
            let root = std::path::Path::new(&entry.path);
            let findings = match model_updater::check_project(root) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("  warn: {}: {e}", entry.path);
                    continue;
                }
            };

            if findings.is_empty() {
                continue;
            }

            // Group findings by file path
            let mut by_file: std::collections::HashMap<
                std::path::PathBuf,
                Vec<&model_updater::DeprecationFinding>,
            > = std::collections::HashMap::new();
            for f in &findings {
                by_file.entry(f.agent_path.clone()).or_default().push(f);
            }

            for (path, file_findings) in &by_file {
                let owned: Vec<_> = file_findings
                    .iter()
                    .map(|f| model_updater::DeprecationFinding {
                        agent_path: f.agent_path.clone(),
                        agent_name: f.agent_name.clone(),
                        field: f.field.clone(),
                        current: f.current.clone(),
                        replacement: f.replacement.clone(),
                    })
                    .collect();
                match model_updater::update_agent_file(path, &owned) {
                    Ok(n) => {
                        if n > 0 {
                            println!("  updated {}: {n} replacement(s)", path.display());
                            total_updated += n;
                        }
                    }
                    Err(e) => eprintln!("  error: {}: {e}", path.display()),
                }
            }
        }

        println!("\n{total_updated} model(s) updated across all projects.");
    } else {
        let (root, _config) = project::find_project_config().ok_or_else(|| {
            anyhow::anyhow!("No project config found. Run from a project directory or use --all.")
        })?;

        let findings = model_updater::check_project(&root)?;
        if findings.is_empty() {
            println!("All models are up to date.");
            return Ok(());
        }

        // Group findings by file path
        let mut by_file: std::collections::HashMap<
            std::path::PathBuf,
            Vec<&model_updater::DeprecationFinding>,
        > = std::collections::HashMap::new();
        for f in &findings {
            by_file.entry(f.agent_path.clone()).or_default().push(f);
        }

        let mut total = 0;
        for (path, file_findings) in &by_file {
            let owned: Vec<_> = file_findings
                .iter()
                .map(|f| model_updater::DeprecationFinding {
                    agent_path: f.agent_path.clone(),
                    agent_name: f.agent_name.clone(),
                    field: f.field.clone(),
                    current: f.current.clone(),
                    replacement: f.replacement.clone(),
                })
                .collect();
            match model_updater::update_agent_file(path, &owned) {
                Ok(n) => {
                    if n > 0 {
                        println!("  updated {}: {n} replacement(s)", path.display());
                        total += n;
                    }
                }
                Err(e) => eprintln!("  error: {}: {e}", path.display()),
            }
        }

        println!("\n{total} model(s) updated.");
    }

    Ok(())
}

fn list() -> anyhow::Result<()> {
    let registry = project_registry::load();

    if registry.projects.is_empty() {
        println!("No registered projects.");
        println!("Projects are auto-registered when you run `armadai run` or `armadai link`.");
        return Ok(());
    }

    println!("Registered projects:\n");
    for entry in &registry.projects {
        println!("  {}  (last seen: {})", entry.path, entry.last_seen);
    }
    println!("\n{} project(s) total.", registry.projects.len());

    Ok(())
}

fn print_findings(findings: &[model_updater::DeprecationFinding]) {
    for f in findings {
        println!(
            "  {} [{}]: {} -> {}",
            f.agent_name, f.field, f.current, f.replacement
        );
    }
}
