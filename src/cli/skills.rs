use clap::Subcommand;

use crate::core::config::user_skills_dir;
use crate::core::project;
use crate::core::skill::{Skill, load_all_skills};
use crate::skills_registry::{cache, search, sync};

#[derive(Subcommand)]
pub enum SkillsAction {
    /// List available skills
    List,
    /// Show a skill's details
    Show {
        /// Skill name
        name: String,
    },
    /// Sync skills from remote sources
    Sync,
    /// Search skills in the registry
    Search {
        /// Search query (keywords, AND logic)
        query: String,
    },
    /// Add a skill from a GitHub repo (owner/repo or owner/repo/skill-name)
    Add {
        /// Skill source (e.g. "anthropics/skills/webapp-testing" or "owner/repo")
        source: String,
        /// Overwrite existing skill
        #[arg(long)]
        force: bool,
    },
    /// Show info about a skill from the registry
    Info {
        /// Skill name
        name: String,
    },
}

pub async fn execute(action: SkillsAction) -> anyhow::Result<()> {
    match action {
        SkillsAction::List => list().await,
        SkillsAction::Show { name } => show(&name).await,
        SkillsAction::Sync => cmd_sync().await,
        SkillsAction::Search { query } => cmd_search(&query).await,
        SkillsAction::Add { source, force } => cmd_add(&source, force).await,
        SkillsAction::Info { name } => cmd_info(&name).await,
    }
}

async fn list() -> anyhow::Result<()> {
    let skills = collect_skills();

    if skills.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No skills found.{m:#}");
        anstream::println!(
            "{m}Add skill directories (containing SKILL.md) in skills/ or ~/.config/armadai/skills/{m:#}"
        );
        anstream::println!(
            "{m}Or run `armadai skills sync` then `armadai skills search <query>` to discover remote skills.{m:#}"
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
    let h = crate::cli::style::header();
    anstream::println!(
        "{h}  {:<name_w$}  {:<desc_w$}  {:<ver_w$}  TOOLS{h:#}",
        "NAME",
        "DESCRIPTION",
        "VERSION",
    );
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  {:<name_w$}  {:<desc_w$}  {:<ver_w$}  -----{m:#}",
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
        let a = crate::cli::style::accent();
        anstream::println!(
            "  {a}{:<name_w$}{a:#}  {:<desc_w$}  {:<ver_w$}  {}",
            skill.name,
            desc,
            ver,
            tools,
        );
    }

    let m = crate::cli::style::muted();
    anstream::println!("\n{m}  {} skill(s) found.{m:#}", skills.len());
    Ok(())
}

async fn show(name: &str) -> anyhow::Result<()> {
    let skills = collect_skills();
    let skill = skills
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("Skill '{name}' not found"))?;

    let h = crate::cli::style::header();
    let a = crate::cli::style::accent();
    anstream::println!("{h}Skill:{h:#} {a}{}{a:#}", skill.name);
    let m = crate::cli::style::muted();
    anstream::println!("{m}Source: {}{m:#}", skill.source.display());

    if let Some(ref desc) = skill.description {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Description:{m:#} {desc}");
    }
    if let Some(ref ver) = skill.version {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Version:{m:#} {ver}");
    }
    if !skill.tools.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}Tools:{m:#} [{}]", skill.tools.join(", "));
    }

    if !skill.scripts.is_empty() {
        let h = crate::cli::style::header();
        anstream::println!("\n{h}Scripts:{h:#}");
        for s in &skill.scripts {
            let m = crate::cli::style::muted();
            anstream::println!("{m}  - {}{m:#}", s.display());
        }
    }
    if !skill.references.is_empty() {
        let h = crate::cli::style::header();
        anstream::println!("\n{h}References:{h:#}");
        for r in &skill.references {
            let m = crate::cli::style::muted();
            anstream::println!("{m}  - {}{m:#}", r.display());
        }
    }
    if !skill.assets.is_empty() {
        let h = crate::cli::style::header();
        anstream::println!("\n{h}Assets:{h:#}");
        for a in &skill.assets {
            let m = crate::cli::style::muted();
            anstream::println!("{m}  - {}{m:#}", a.display());
        }
    }

    println!();
    let h = crate::cli::style::header();
    anstream::println!("{h}## Body{h:#}");
    for line in skill.body.lines() {
        println!("  {line}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Registry commands
// ---------------------------------------------------------------------------

async fn cmd_sync() -> anyhow::Result<()> {
    let sources = cache::effective_sources();
    let r = crate::cli::style::running();
    anstream::println!("{r}Syncing {} skill source(s)...{r:#}", sources.len());
    sync::sync_all(&sources)?;
    let r = crate::cli::style::running();
    anstream::println!("{r}Building search index...{r:#}");
    let index = cache::build_index(&sources)?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Indexed {} skill(s).{o:#}", index.entries.len());
    Ok(())
}

async fn cmd_search(query: &str) -> anyhow::Result<()> {
    check_staleness();
    let sources = cache::effective_sources();
    let index = cache::load_or_build_index(&sources)?;

    if index.entries.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No skills indexed. Run `armadai skills sync` first.{m:#}");
        return Ok(());
    }

    let results = search::search(&index.entries, query);

    if results.is_empty() {
        let m = crate::cli::style::muted();
        anstream::println!("{m}No skills matching '{query}'.{m:#}");
        return Ok(());
    }

    // Compute column widths
    let name_w = results
        .iter()
        .map(|r| r.entry.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let repo_w = results
        .iter()
        .map(|r| r.entry.source_repo.len())
        .max()
        .unwrap_or(6)
        .max(6);

    let h = crate::cli::style::header();
    anstream::println!(
        "{h}  {:<name_w$}  {:>5}  {:<repo_w$}  DESCRIPTION{h:#}",
        "NAME",
        "SCORE",
        "SOURCE",
    );
    let m = crate::cli::style::muted();
    anstream::println!(
        "{m}  {:<name_w$}  {:>5}  {:<repo_w$}  -----------{m:#}",
        "-".repeat(name_w),
        "-----",
        "-".repeat(repo_w),
    );

    for r in &results {
        let desc = r.entry.description.as_deref().unwrap_or("-");
        let desc_display = if desc.len() > 50 {
            format!("{}...", &desc[..47])
        } else {
            desc.to_string()
        };
        let a = crate::cli::style::accent();
        anstream::println!(
            "  {a}{:<name_w$}{a:#}  {:>5}  {:<repo_w$}  {}",
            r.entry.name,
            r.score,
            r.entry.source_repo,
            desc_display,
        );
    }

    let m = crate::cli::style::muted();
    anstream::println!("\n{m}  {} result(s).{m:#}", results.len());
    Ok(())
}

async fn cmd_add(source: &str, force: bool) -> anyhow::Result<()> {
    // Parse source: owner/repo or owner/repo/skill-name
    let parts: Vec<&str> = source.split('/').collect();
    let (owner, repo, skill_name) = match parts.len() {
        2 => (parts[0], parts[1], None),
        n if n >= 3 => (parts[0], parts[1], Some(parts[2..].join("/"))),
        _ => anyhow::bail!("Invalid source format. Use owner/repo or owner/repo/skill-name"),
    };

    // Clone/pull the repo
    let repo_slug = format!("{owner}/{repo}");
    let r = crate::cli::style::running();
    anstream::println!("{r}Syncing {repo_slug}...{r:#}");
    let repo_path = sync::sync_repo(&repo_slug)?;

    // Scan for skills in the repo
    let mut skill_dirs = Vec::new();
    find_skill_dirs(&repo_path, &mut skill_dirs);

    if skill_dirs.is_empty() {
        anyhow::bail!("No skills (SKILL.md) found in {repo_slug}");
    }

    // Determine which skill to install
    let skill_dir = if let Some(ref name) = skill_name {
        // Find skill matching the name (by dir name or relative path)
        skill_dirs
            .iter()
            .find(|d| {
                let rel = d.strip_prefix(&repo_path).unwrap_or(d);
                let dir_name = d.file_name().and_then(|s| s.to_str()).unwrap_or("");
                dir_name == name.as_str() || rel.to_string_lossy() == *name
            })
            .ok_or_else(|| {
                let available: Vec<String> = skill_dirs
                    .iter()
                    .filter_map(|d| d.file_name().and_then(|s| s.to_str()).map(String::from))
                    .collect();
                anyhow::anyhow!(
                    "Skill '{name}' not found in {repo_slug}. Available: {}",
                    available.join(", ")
                )
            })?
            .clone()
    } else if skill_dirs.len() == 1 {
        skill_dirs[0].clone()
    } else {
        let h = crate::cli::style::header();
        anstream::println!("{h}Multiple skills found in {repo_slug}:{h:#}");
        for (i, d) in skill_dirs.iter().enumerate() {
            let name = d.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            let a = crate::cli::style::accent();
            anstream::println!("  {}. {a}{}{a:#}", i + 1, name);
        }
        anyhow::bail!(
            "Specify which skill to install: armadai skills add {repo_slug}/<skill-name>"
        );
    };

    // Load skill metadata for display
    let skill = Skill::load(&skill_dir)?;
    let dest = user_skills_dir().join(&skill.name);

    if dest.exists() && !force {
        anyhow::bail!(
            "Skill '{}' already exists at {}. Use --force to overwrite.",
            skill.name,
            dest.display()
        );
    }

    // Copy entire skill directory
    copy_dir_recursive(&skill_dir, &dest)?;

    let o = crate::cli::style::ok();
    let a = crate::cli::style::accent();
    let m = crate::cli::style::muted();
    anstream::println!(
        "{o}Installed skill{o:#} {a}'{}'{a:#} {o}to{o:#} {m}{}{m:#}",
        skill.name,
        dest.display()
    );
    Ok(())
}

async fn cmd_info(name: &str) -> anyhow::Result<()> {
    check_staleness();
    let sources = cache::effective_sources();
    let index = cache::load_or_build_index(&sources)?;

    let entry = index
        .entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Skill '{name}' not found in registry. Try `armadai skills search {name}`"
            )
        })?;

    let m = crate::cli::style::muted();
    let a = crate::cli::style::accent();
    anstream::println!("{m}Name:        {m:#}{a}{}{a:#}", entry.name);
    anstream::println!("{m}Source:      {m:#}{}", entry.source_repo);
    anstream::println!("{m}Path:        {m:#}{}", entry.path);
    if let Some(ref desc) = entry.description {
        anstream::println!("{m}Description: {m:#}{desc}");
    }
    if !entry.tags.is_empty() {
        anstream::println!("{m}Tags:        {m:#}[{}]", entry.tags.join(", "));
    }

    // Try to show SKILL.md content from cloned repo
    if let Some((owner, repo)) = sync::parse_source(&entry.source_repo) {
        let skill_file = sync::repo_dir(&owner, &repo)
            .join(&entry.path)
            .join("SKILL.md");
        if skill_file.is_file() {
            let h = crate::cli::style::header();
            anstream::println!("\n{h}--- SKILL.md ---{h:#}");
            let content = std::fs::read_to_string(&skill_file)?;
            for (i, line) in content.lines().enumerate() {
                if i >= 40 {
                    let m = crate::cli::style::muted();
                    anstream::println!(
                        "{m}  ... (truncated, {} more lines){m:#}",
                        content.lines().count() - 40
                    );
                    break;
                }
                println!("  {line}");
            }
        }
    }

    let m = crate::cli::style::muted();
    anstream::println!(
        "\n{m}Install with: armadai skills add {}/{}{m:#}",
        entry.source_repo,
        entry.name
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recursively find directories containing SKILL.md.
fn find_skill_dirs(dir: &std::path::Path, results: &mut Vec<std::path::PathBuf>) {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };

    for entry in read.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_str().is_some_and(|s| s.starts_with('.')))
        {
            continue;
        }
        if path.is_dir() {
            if path.join("SKILL.md").is_file() {
                results.push(path.clone());
            }
            find_skill_dirs(&path, results);
        }
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)?;
    }
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)?.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Print a hint if the skills registry is stale.
fn check_staleness() {
    if sync::is_stale(7) && sync::repos_dir().is_dir() {
        let w = crate::cli::style::warn();
        anstream::eprintln!(
            "{w}hint: skills registry may be outdated. Run `armadai skills sync` to refresh.{w:#}"
        );
    }
}

/// Collect skills from project config and/or default directories.
fn collect_skills() -> Vec<Skill> {
    let mut skills = Vec::new();

    // Project-level skills
    if let Some((root, config)) = project::find_project_config() {
        let (paths, errors) = project::resolve_all_skills(&config, &root);
        for err in &errors {
            let w = crate::cli::style::warn();
            anstream::eprintln!("{w}  warn: {err}{w:#}");
        }
        for path in &paths {
            match Skill::load(path) {
                Ok(s) => skills.push(s),
                Err(e) => {
                    let w = crate::cli::style::warn();
                    anstream::eprintln!(
                        "{w}  warn: failed to load skill {}: {e}{w:#}",
                        path.display()
                    );
                }
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
