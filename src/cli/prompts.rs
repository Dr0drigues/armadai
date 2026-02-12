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
        println!("No prompts found.");
        println!("Add .md files in prompts/ or ~/.config/armadai/prompts/");
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
    println!(
        "  {:<name_w$}  {:<desc_w$}  APPLY_TO",
        "NAME", "DESCRIPTION",
    );
    println!(
        "  {:<name_w$}  {:<desc_w$}  --------",
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
        println!("  {:<name_w$}  {:<desc_w$}  {}", prompt.name, desc, apply);
    }

    println!("\n  {} prompt(s) found.", prompts.len());
    Ok(())
}

async fn show(name: &str) -> anyhow::Result<()> {
    let prompts = collect_prompts();
    let prompt = prompts
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| anyhow::anyhow!("Prompt '{name}' not found"))?;

    println!("Prompt: {}", prompt.name);
    println!("Source: {}", prompt.source.display());

    if let Some(ref desc) = prompt.description {
        println!("Description: {desc}");
    }
    if !prompt.apply_to.is_empty() {
        println!("Apply to: [{}]", prompt.apply_to.join(", "));
    }

    println!();
    println!("## Body");
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
            eprintln!("  warn: {err}");
        }
        for path in &paths {
            match Prompt::load(path) {
                Ok(p) => prompts.push(p),
                Err(e) => eprintln!("  warn: failed to load prompt {}: {e}", path.display()),
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
