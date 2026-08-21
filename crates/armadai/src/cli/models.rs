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
    let mut any_failed = false;

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

            let (n, failed) = apply_and_report(&findings, &agent_source::declarations_path(root));
            total_updated += n;
            any_failed |= failed;
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

        let (total, failed) = apply_and_report(&findings, &agent_source::declarations_path(&root));
        any_failed = failed;

        let o = crate::cli::style::ok();
        anstream::println!("{o}\n{total} model(s) updated.{o:#}");
    }

    // Every group/project is processed to the end regardless of an earlier
    // one's failure — a problem isolated to one file must not stop this
    // command from fixing every other file/project that is fine, the same
    // reasoning already applied above to a `check_project` error. But the
    // command as a whole must not exit 0 when something was left broken: a
    // caller that only checks the exit code (a script, CI, `--all` from a
    // cron job) needs to be able to tell "everything fixed" from "some
    // file(s) still need a look", and the summary line above already
    // reports the true (partial) count rather than lying about it.
    if any_failed {
        anyhow::bail!(
            "one or more file(s) could not be fully updated (see error(s) above) — nothing in \
             those files was rewritten"
        );
    }

    Ok(())
}

/// Apply every finding, grouped by the file it came from, and print one
/// status line per file — the shared body of both branches above.
///
/// One [`model_updater::apply_findings`] call per file, carrying that
/// file's ENTIRE finding set — never split into one call per finding. A
/// prior version of this function did split per finding, which silently
/// broke `update_declarations`'s own all-or-nothing guarantee one layer up:
/// a `defaults.model` fix could land on disk from one call, immediately
/// before a second call — for the SAME file, a finding the textual rewrite
/// could not locate — errored, leaving a half-fixed file reported as "0
/// model(s) updated" with a plain `eprintln` and no effect on the exit
/// code. Batching restores the guarantee: either the whole file is fixed
/// and counted, or nothing in it changes and the caller sees an `Err`.
///
/// `decls_path` (the project's `agents.yaml`, whether or not it exists) is
/// what routes a file's findings to the right rewriter:
/// [`model_updater::apply_findings`] sends anything whose `agent_path`
/// matches it through `update_declarations`, and everything else through
/// `update_agent_file`. Applying a whole file's findings through the wrong
/// one is exactly the bug this exists to avoid — `update_agent_file`'s
/// single `replacen(.., 1)` and unbounded `: <model>` pattern can rewrite a
/// comment in `agents.yaml` while leaving the real deprecated field
/// untouched, all while reporting success.
///
/// Returns `(replacements actually made, whether any file failed)` — never
/// invents a non-zero count for a file that errored.
fn apply_and_report(
    findings: &[model_updater::DeprecationFinding],
    decls_path: &std::path::Path,
) -> (usize, bool) {
    let mut by_file: std::collections::HashMap<
        std::path::PathBuf,
        Vec<model_updater::DeprecationFinding>,
    > = std::collections::HashMap::new();
    for f in findings {
        by_file
            .entry(f.agent_path.clone())
            .or_default()
            .push(f.clone());
    }

    let mut total = 0;
    let mut any_failed = false;
    for (path, file_findings) in &by_file {
        match model_updater::apply_findings(file_findings, decls_path) {
            Ok(n) => {
                if n > 0 {
                    let o = crate::cli::style::ok();
                    anstream::println!("{o}  updated {}: {n} replacement(s){o:#}", path.display());
                }
                total += n;
            }
            Err(e) => {
                any_failed = true;
                let er = crate::cli::style::err();
                anstream::eprintln!("{er}  error: {}: {e}{er:#}", path.display());
            }
        }
    }
    (total, any_failed)
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
