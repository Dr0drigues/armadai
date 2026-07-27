use clap::Subcommand;

use crate::core::config::{AppPaths, user_prompts_dir};
use crate::core::project;
use crate::core::prompt::{Prompt, load_all_prompts};

#[derive(Subcommand)]
pub enum PromptsAction {
    /// List available prompts
    List,
    /// Show a prompt's details
    Show {
        /// Prompt name
        name: String,
    },
}

pub async fn execute(action: PromptsAction) -> anyhow::Result<()> {
    match action {
        PromptsAction::List => list().await,
        PromptsAction::Show { name } => show(&name).await,
    }
}

async fn list() -> anyhow::Result<()> {
    let prompts = collect_prompts();

    if prompts.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No prompts found.{m:#}");
        anstream::println!("{m}Add .md files in prompts/ or ~/.config/armadai/prompts/{m:#}");
        return Ok(());
    }

    // Compute column widths
    let name_w = prompts
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let desc_w = prompts
        .iter()
        .map(|p| p.description.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(11)
        .max(11);

    // Header
    let h = crate::cli::style::header();
    anstream::println!(
        "{h}  {:<name_w$}  {:<desc_w$}  APPLY_TO{h:#}",
        "NAME",
        "DESCRIPTION",
    );
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  {:<name_w$}  {:<desc_w$}  --------{m:#}",
        "-".repeat(name_w),
        "-".repeat(desc_w),
    );

    // Rows
    for prompt in &prompts {
        let desc = prompt.description.as_deref().unwrap_or("-");
        let apply = if prompt.apply_to.is_empty() {
            "-".to_string()
        } else {
            prompt.apply_to.join(", ")
        };
        let a = crate::cli::style::accent();
        anstream::println!(
            "  {a}{:<name_w$}{a:#}  {:<desc_w$}  {}",
            prompt.name,
            desc,
            apply
        );
    }

    let m = crate::cli::style::muted();
    anstream::println!("\n{m}  {} prompt(s) found.{m:#}", prompts.len());
    Ok(())
}

async fn show(name: &str) -> anyhow::Result<()> {
    let prompts = collect_prompts();
    let prompt = prompts
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow::anyhow!("Prompt '{name}' not found"))?;

    let h = crate::cli::style::header();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!("{h}Prompt:{h:#} {a}{}{a:#}", prompt.name);
    anstream::println!("{m}Source: {}{m:#}", prompt.source.display());

    if let Some(ref desc) = prompt.description {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Description:{m:#} {desc}");
    }
    if !prompt.apply_to.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Apply to:{m:#} [{}]", prompt.apply_to.join(", "));
    }

    println!();
    let h = crate::cli::style::header();
    anstream::println!("{h}## Body{h:#}");
    for line in prompt.body.lines() {
        println!("  {line}");
    }

    Ok(())
}

/// Collect prompts from project config and/or default directories.
fn collect_prompts() -> Vec<Prompt> {
    let mut prompts = Vec::new();

    // Project-level prompts
    if let Some((root, config)) = project::find_project_config() {
        let (paths, errors) = project::resolve_all_prompts(&config, &root);
        for err in &errors {
            let w = crate::cli::style::warn();
            anstream::eprintln!("{w}  warn: {err}{w:#}");
        }
        for path in &paths {
            match Prompt::load(path) {
                Ok(p) => prompts.push(p),
                Err(e) => {
                    let w = crate::cli::style::warn();
                    anstream::eprintln!(
                        "{w}  warn: failed to load prompt {}: {e}{w:#}",
                        path.display()
                    );
                }
            }
        }

        // Also scan project-local prompts/ directory for prompts not in config
        let local_dir = root.join("prompts");
        if local_dir.is_dir() {
            for p in load_all_prompts(&local_dir) {
                if !prompts.iter().any(|existing| existing.name == p.name) {
                    prompts.push(p);
                }
            }
        }
    } else {
        // No project config — scan default local prompts/ dir
        let paths = AppPaths::resolve();
        let local_dir = paths
            .agents_dir
            .parent()
            .unwrap_or(paths.agents_dir.as_ref())
            .join("prompts");
        if local_dir.is_dir() {
            prompts.extend(load_all_prompts(&local_dir));
        }
    }

    // Always include user-global prompts
    let global_dir = user_prompts_dir();
    if global_dir.is_dir() {
        for p in load_all_prompts(&global_dir) {
            if !prompts.iter().any(|existing| existing.name == p.name) {
                prompts.push(p);
            }
        }
    }

    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    prompts
}
