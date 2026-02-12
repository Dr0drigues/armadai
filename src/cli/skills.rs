use clap::Subcommand;

use crate::core::config::user_skills_dir;
use crate::core::project;
use crate::core::skill::{Skill, load_all_skills};

#[derive(Subcommand)]
pub enum SkillsAction {
    /// List available skills
    List,
    /// Show a skill's details
    Show {
        /// Skill name
        name: String,
    },
}

pub async fn execute(action: SkillsAction) -> anyhow::Result<()> {
    match action {
        SkillsAction::List => list().await,
        SkillsAction::Show { name } => show(&name).await,
    }
}

async fn list() -> anyhow::Result<()> {
    let skills = collect_skills();

    if skills.is_empty() {
        println!("No skills found.");
        println!(
            "Add skill directories (containing SKILL.md) in skills/ or ~/.config/armadai/skills/"
        );
        return Ok(());
    }

    // Compute column widths
    let name_w = skills
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let desc_w = skills
        .iter()
        .map(|s| s.description.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(11)
        .max(11);
    let ver_w = skills
        .iter()
        .map(|s| s.version.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(7)
        .max(7);

    // Header
    println!(
        "  {:<name_w$}  {:<desc_w$}  {:<ver_w$}  TOOLS",
        "NAME", "DESCRIPTION", "VERSION",
    );
    println!(
        "  {:<name_w$}  {:<desc_w$}  {:<ver_w$}  -----",
        "-".repeat(name_w),
        "-".repeat(desc_w),
        "-".repeat(ver_w),
    );

    // Rows
    for skill in &skills {
        let desc = skill.description.as_deref().unwrap_or("-");
        let ver = skill.version.as_deref().unwrap_or("-");
        let tools = if skill.tools.is_empty() {
            "-".to_string()
        } else {
            skill.tools.join(", ")
        };
        println!(
            "  {:<name_w$}  {:<desc_w$}  {:<ver_w$}  {}",
            skill.name, desc, ver, tools,
        );
    }

    println!("\n  {} skill(s) found.", skills.len());
    Ok(())
}

async fn show(name: &str) -> anyhow::Result<()> {
    let skills = collect_skills();
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{name}' not found"))?;

    println!("Skill: {}", skill.name);
    println!("Source: {}", skill.source.display());

    if let Some(ref desc) = skill.description {
        println!("Description: {desc}");
    }
    if let Some(ref ver) = skill.version {
        println!("Version: {ver}");
    }
    if !skill.tools.is_empty() {
        println!("Tools: [{}]", skill.tools.join(", "));
    }

    if !skill.scripts.is_empty() {
        println!("\nScripts:");
        for s in &skill.scripts {
            println!("  - {}", s.display());
        }
    }
    if !skill.references.is_empty() {
        println!("\nReferences:");
        for r in &skill.references {
            println!("  - {}", r.display());
        }
    }
    if !skill.assets.is_empty() {
        println!("\nAssets:");
        for a in &skill.assets {
            println!("  - {}", a.display());
        }
    }

    println!();
    println!("## Body");
    for line in skill.body.lines() {
        println!("  {line}");
    }

    Ok(())
}

/// Collect skills from project config and/or default directories.
fn collect_skills() -> Vec<Skill> {
    let mut skills = Vec::new();

    // Project-level skills
    if let Some((root, config)) = project::find_project_config() {
        let (paths, errors) = project::resolve_all_skills(&config, &root);
        for err in &errors {
            eprintln!("  warn: {err}");
        }
        for path in &paths {
            match Skill::load(path) {
                Ok(s) => skills.push(s),
                Err(e) => eprintln!("  warn: failed to load skill {}: {e}", path.display()),
            }
        }

        // Also scan project-local skills/ directory
        let local_dir = root.join("skills");
        if local_dir.is_dir() {
            for s in load_all_skills(&local_dir) {
                if !skills.iter().any(|existing| existing.name == s.name) {
                    skills.push(s);
                }
            }
        }
    } else {
        // No project config — scan local skills/ dir
        let local_dir = std::path::Path::new("skills");
        if local_dir.is_dir() {
            skills.extend(load_all_skills(local_dir));
        }
    }

    // Always include user-global skills
    let global_dir = user_skills_dir();
    if global_dir.is_dir() {
        for s in load_all_skills(&global_dir) {
            if !skills.iter().any(|existing| existing.name == s.name) {
                skills.push(s);
            }
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}
