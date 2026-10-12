use clap::Subcommand;

use std::path::{Path, PathBuf};

use armadai_core::model_aliases::resolve_alias;
use armadai_core::project_registry;

#[derive(Subcommand)]
pub enum ModelsAction {
    /// Check the `native.<target>.model` ids of the project's agents for deprecated ones
    Check {
        /// Check all registered projects
        #[arg(long)]
        all: bool,
        /// Remove stale projects from the registry (with --all)
        #[arg(long)]
        prune: bool,
    },
    /// Replace deprecated `native.<target>.model` ids in the agent files
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
        ModelsAction::Check { all, prune } => run(all, prune, false),
        ModelsAction::Update { all } => run(all, false, true),
        ModelsAction::List => list(),
    }
}

/// A deprecated model id in an agent's `native.<target>.model` (canonical agents
/// carry a tier; a model id only lives there).
#[derive(Debug, Clone, PartialEq)]
struct Finding {
    agent: String,
    target: String,
    current: String,
    replacement: String,
}

impl Finding {
    /// A `latest:*` replacement is a tier, not an id: reported, not rewritten.
    fn rewritable(&self) -> bool {
        !self.replacement.starts_with("latest")
    }
}

fn findings(root: &Path) -> anyhow::Result<Vec<Finding>> {
    let (project, _) = armadai_model::load_checked(root).map_err(|ds| {
        anyhow::anyhow!(
            "{} is invalid ({} diagnostic(s)): run `armadai validate`",
            root.display(),
            ds.len()
        )
    })?;
    let mut out = Vec::new();
    for (id, agent) in &project.agents {
        for (target, native) in &agent.front.native {
            let Some(current) = native.get("model").and_then(|m| m.as_str()) else {
                continue;
            };
            if let Some(replacement) = resolve_alias(current) {
                out.push(Finding {
                    agent: id.clone(),
                    target: target.clone(),
                    current: current.to_string(),
                    replacement,
                });
            }
        }
    }
    Ok(out)
}

/// Replace the deprecated ids inside each agent's frontmatter only, as text, so
/// the rest of the file (comments, order, body) is kept; the project must still
/// validate afterwards.
fn apply(root: &Path, findings: &[Finding]) -> anyhow::Result<usize> {
    let mut by_agent: std::collections::BTreeMap<&str, Vec<&Finding>> = Default::default();
    for f in findings.iter().filter(|f| f.rewritable()) {
        by_agent.entry(&f.agent).or_default().push(f);
    }
    let mut n = 0;
    for (agent, fs) in by_agent {
        let path = root.join(".armadai/agents").join(format!("{agent}.md"));
        let text = std::fs::read_to_string(&path)?;
        let (yaml, _, _) = armadai_model::split_frontmatter(&text)
            .ok_or_else(|| anyhow::anyhow!("{}: no frontmatter", path.display()))?;
        let start = yaml.as_ptr() as usize - text.as_ptr() as usize;
        let mut front = yaml.to_string();
        for f in &fs {
            front = front.replace(f.current.as_str(), &f.replacement);
        }
        let new = format!("{}{front}{}", &text[..start], &text[start + yaml.len()..]);
        std::fs::write(&path, new)?;
        n += fs.len();
        let o = crate::cli::style::ok();
        anstream::println!(
            "{o}  updated {}: {} replacement(s){o:#}",
            path.display(),
            fs.len()
        );
    }
    if armadai_model::load_checked(root).is_err() {
        anyhow::bail!(
            "{}: the project no longer validates after the update",
            root.display()
        );
    }
    Ok(n)
}

fn print_findings(findings: &[Finding]) {
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    let w = crate::cli::style::warn();
    let o = crate::cli::style::ok();
    for f in findings {
        let note = if f.rewritable() {
            String::new()
        } else {
            format!(
                " {m}(a tier: set `tier:` and drop native.{}.model){m:#}",
                f.target
            )
        };
        anstream::println!(
            "  {a}{}{a:#} {m}[native.{}.model]{m:#}: {w}{}{w:#} -> {o}{}{o:#}{note}",
            f.agent,
            f.target,
            f.current,
            f.replacement
        );
    }
}

/// The projects to look at: the current one, or every registered one.
fn projects(all: bool, prune: bool) -> anyhow::Result<Vec<PathBuf>> {
    if !all {
        return Ok(vec![crate::compile::find_project(
            &std::env::current_dir()?
        )?]);
    }
    let mut registry = project_registry::load();
    if prune {
        let pruned = project_registry::prune_stale(&mut registry);
        if !pruned.is_empty() {
            let m = crate::cli::style::muted();
            anstream::println!("{m}Pruned {} stale project(s):{m:#}", pruned.len());
            for p in &pruned {
                anstream::println!("  - {p}");
            }
            project_registry::save(&registry)?;
        }
    }
    Ok(registry
        .projects
        .iter()
        .map(|e| PathBuf::from(&e.path))
        .collect())
}

fn run(all: bool, prune: bool, update: bool) -> anyhow::Result<()> {
    let roots = projects(all, prune)?;
    if roots.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!(
            "{m}No registered projects. Run `armadai link` in a project first.{m:#}"
        );
        return Ok(());
    }
    let (mut found, mut updated, mut failed) = (0, 0, false);
    for root in &roots {
        if !root.join(".armadai/project.yaml").is_file() {
            let w = crate::cli::style::warn();
            anstream::eprintln!(
                "{w}  warn: {}: legacy format, run `armadai migrate`{w:#}",
                root.display()
            );
            continue;
        }
        let fs = match findings(root) {
            Ok(f) => f,
            Err(e) => {
                let w = crate::cli::style::warn();
                anstream::eprintln!("{w}  warn: {e}{w:#}");
                failed = true;
                continue;
            }
        };
        if fs.is_empty() {
            continue;
        }
        if all {
            let h = crate::cli::style::header();
            anstream::println!("{h}{}:{h:#}", root.display());
        }
        print_findings(&fs);
        found += fs.len();
        if update {
            updated += apply(root, &fs)?;
        }
    }
    if found == 0 {
        let o = crate::cli::style::ok();
        anstream::println!("{o}All models are up to date.{o:#}");
    } else if update {
        let o = crate::cli::style::ok();
        anstream::println!("{o}\n{updated} model(s) updated.{o:#}");
    } else {
        let w = crate::cli::style::warn();
        let all_flag = if all { " --all" } else { "" };
        anstream::println!(
            "\n{w}{found} deprecated model(s) found. Run `armadai models update{all_flag}` to fix.{w:#}"
        );
    }
    if failed && !all {
        anyhow::bail!("the project could not be checked");
    }
    Ok(())
}

fn list() -> anyhow::Result<()> {
    let registry = project_registry::load();

    if registry.projects.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No registered projects.{m:#}");
        anstream::println!("{m}Projects are auto-registered when you run `armadai link`.{m:#}");
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
