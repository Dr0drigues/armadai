use clap::Subcommand;

use armadai_core::agent_source;
use armadai_core::model_updater;
use armadai_core::project;
use armadai_core::project_registry;

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
                let m = crate::cli::style::muted();
                anstream::println!("{m}Pruned {} stale project(s):{m:#}", pruned.len());
                let a = crate::cli::style::accent();
                for p in &pruned {
                    anstream::println!("  {a}- {p}{a:#}");
                }
                project_registry::save(&registry)?;
                anstream::println!();
            }
        }

        if registry.projects.is_empty() {
            let m = crate::cli::style::muted();
            anstream::println!(
                "{m}No registered projects. Run `armadai run` or `armadai link` in a project first.{m:#}"
            );
            return Ok(());
        }

        let mut total = 0;
        for entry in &registry.projects {
            let findings = match model_updater::check_project(std::path::Path::new(&entry.path)) {
                Ok(f) => f,
                Err(e) => {
                    let w = crate::cli::style::warn();
                    anstream::eprintln!("{w}  warn: {}: {e}{w:#}", entry.path);
                    continue;
                }
            };
            if !findings.is_empty() {
                let h = crate::cli::style::header();
                anstream::println!("{h}{}:{h:#}", entry.path);
                print_findings(&findings);
                total += findings.len();
            }
        }

        if total == 0 {
            let o = crate::cli::style::ok();
            anstream::println!(
                "{o}All models are up to date across {} project(s).{o:#}",
                registry.projects.len()
            );
        } else {
            let w = crate::cli::style::warn();
            anstream::println!(
                "\n{w}{total} deprecated model(s) found. Run `armadai models update --all` to fix.{w:#}"
            );
        }
    } else {
        let (root, _config) = project::find_project_config().ok_or_else(|| {
            anyhow::anyhow!("No project config found. Run from a project directory or use --all.")
        })?;

        let findings = model_updater::check_project(&root)?;
        if findings.is_empty() {
            let o = crate::cli::style::ok();
            anstream::println!("{o}All models are up to date.{o:#}");
        } else {
            print_findings(&findings);
            let w = crate::cli::style::warn();
            anstream::println!(
                "\n{w}{} deprecated model(s) found. Run `armadai models update` to fix.{w:#}",
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
            let m = crate::cli::style::muted();
            anstream::println!("{m}No registered projects.{m:#}");
            return Ok(());
        }

        let mut total_updated = 0;
        for entry in &registry.projects {
            let root = std::path::Path::new(&entry.path);
            let findings = match model_updater::check_project(root) {
                Ok(f) => f,
                Err(e) => {
                    let w = crate::cli::style::warn();
                    anstream::eprintln!("{w}  warn: {}: {e}{w:#}", entry.path);
                    continue;
                }
            };

            if findings.is_empty() {
                continue;
            }

            total_updated += apply_and_report(&findings, &agent_source::declarations_path(root));
        }

        let o = crate::cli::style::ok();
        anstream::println!("{o}\n{total_updated} model(s) updated across all projects.{o:#}");
    } else {
        let (root, _config) = project::find_project_config().ok_or_else(|| {
            anyhow::anyhow!("No project config found. Run from a project directory or use --all.")
        })?;

        let findings = model_updater::check_project(&root)?;
        if findings.is_empty() {
            let o = crate::cli::style::ok();
            anstream::println!("{o}All models are up to date.{o:#}");
            return Ok(());
        }

        let total = apply_and_report(&findings, &agent_source::declarations_path(&root));

        let o = crate::cli::style::ok();
        anstream::println!("{o}\n{total} model(s) updated.{o:#}");
    }

    Ok(())
}

/// Apply every finding, grouped by the file it came from, and print one
/// status line per file — the shared body of both branches above.
///
/// `decls_path` (the project's `agents.yaml`, whether or not it exists) is
/// what routes a finding to the right rewriter: [`model_updater::apply_finding`]
/// sends anything whose `agent_path` matches it through
/// `update_declarations`, and everything else through `update_agent_file`.
/// Applying a whole file's findings through the wrong one is exactly the
/// bug this exists to avoid — `update_agent_file`'s single
/// `replacen(.., 1)` and unbounded `: <model>` pattern can rewrite a
/// comment in `agents.yaml` while leaving the real deprecated field
/// untouched, all while reporting success.
fn apply_and_report(
    findings: &[model_updater::DeprecationFinding],
    decls_path: &std::path::Path,
) -> usize {
    let mut by_file: std::collections::HashMap<
        std::path::PathBuf,
        Vec<&model_updater::DeprecationFinding>,
    > = std::collections::HashMap::new();
    for f in findings {
        by_file.entry(f.agent_path.clone()).or_default().push(f);
    }

    let mut total = 0;
    for (path, file_findings) in &by_file {
        let mut file_total = 0;
        let mut file_err = None;
        for f in file_findings {
            match model_updater::apply_finding(f, decls_path) {
                Ok(n) => file_total += n,
                Err(e) => {
                    file_err = Some(e);
                    break;
                }
            }
        }
        match file_err {
            Some(e) => {
                let er = crate::cli::style::err();
                anstream::eprintln!("{er}  error: {}: {e}{er:#}", path.display());
            }
            None if file_total > 0 => {
                let o = crate::cli::style::ok();
                anstream::println!(
                    "{o}  updated {}: {file_total} replacement(s){o:#}",
                    path.display()
                );
                total += file_total;
            }
            None => {}
        }
    }
    total
}

fn list() -> anyhow::Result<()> {
    let registry = project_registry::load();

    if registry.projects.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No registered projects.{m:#}");
        anstream::println!(
            "{m}Projects are auto-registered when you run `armadai run` or `armadai link`.{m:#}"
        );
        return Ok(());
    }

    let h = crate::cli::style::header();
    anstream::println!("{h}Registered projects:{h:#}\n");
    for entry in &registry.projects {
        let a = crate::cli::style::accent();
        let m = crate::cli::style::muted();
        anstream::println!(
            "  {a}{}{a:#}  {m}(last seen: {}){m:#}",
            entry.path,
            entry.last_seen
        );
    }
    let m = crate::cli::style::muted();
    anstream::println!("\n{m}{} project(s) total.{m:#}", registry.projects.len());

    Ok(())
}

fn print_findings(findings: &[model_updater::DeprecationFinding]) {
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    let w = crate::cli::style::warn();
    let o = crate::cli::style::ok();
    for f in findings {
        anstream::println!(
            "  {a}{}{a:#} {m}[{}]{m:#}: {w}{}{w:#} -> {o}{}{o:#}",
            f.agent_name,
            f.field,
            f.current,
            f.replacement
        );
    }
}
