use std::path::{Path, PathBuf};

use crate::core::project;
use crate::linker::{self, LinkAgent};
use crate::parser;

pub async fn execute(
    target: Option<String>,
    coordinator_flag: Option<String>,
    dry_run: bool,
    with_config: bool,
    output: Option<PathBuf>,
    agents_filter: Option<Vec<String>>,
) -> anyhow::Result<()> {
    // 1. Find project config
    let (root, config) = project::find_project_config().ok_or_else(|| {
        anyhow::anyhow!("No armadai.yaml found. Run `armadai init --project` to create one.")
    })?;

    if config.agents.is_empty() {
        anyhow::bail!("No agents declared in armadai.yaml.");
    }

    // 2. Resolve and parse agents
    let (paths, errors) = project::resolve_all_agents(&config, &root);
    for err in &errors {
        eprintln!("  warn: {err}");
    }

    let mut link_agents: Vec<LinkAgent> = Vec::new();
    for path in &paths {
        match parser::parse_agent_file(path) {
            Ok(agent) => link_agents.push(LinkAgent::from(&agent)),
            Err(e) => eprintln!("  warn: failed to parse {}: {e}", path.display()),
        }
    }

    if link_agents.is_empty() {
        anyhow::bail!("No agents could be resolved. Check your armadai.yaml.");
    }

    // 3. Filter by --agents if provided
    if let Some(ref filter) = agents_filter {
        let filter_lower: Vec<String> = filter.iter().map(|s| s.to_lowercase()).collect();
        link_agents.retain(|a| filter_lower.contains(&a.name.to_lowercase()));
        if link_agents.is_empty() {
            anyhow::bail!("No agents match the given filter: {}", filter.join(", "));
        }
    }

    // 3b. Extract coordinator if configured (CLI flag takes priority over config)
    let coordinator_name =
        coordinator_flag.or_else(|| config.link.as_ref().and_then(|l| l.coordinator.clone()));
    let coordinator = coordinator_name.and_then(|name| {
        let idx = link_agents
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(&name))?;
        Some(link_agents.remove(idx))
    });

    // 4. Determine target
    let target_name = target
        .or_else(|| config.link.as_ref().and_then(|l| l.target.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No link target specified. Use --target or set link.target in armadai.yaml.\n\
                 Supported targets: claude, copilot, gemini, opencode"
            )
        })?;

    // 5. Create linker
    let linker = linker::create_linker(&target_name)?;

    // 6. Determine output directory
    let output_dir = output
        .or_else(|| {
            config
                .link
                .as_ref()
                .and_then(|l| l.overrides.get(&target_name))
                .and_then(|o| o.output.as_ref())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from(linker.default_output_dir()));

    // 7. Generate file list (we only need paths, not content)
    let sources = &config.sources;
    let files = linker.generate(&link_agents, coordinator.as_ref(), sources);

    if files.is_empty() {
        println!("No files to remove.");
        return Ok(());
    }

    // 8. Resolve output paths relative to project root
    let mut targets: Vec<PathBuf> = files
        .into_iter()
        .map(|f| {
            let default_dir = PathBuf::from(linker.default_output_dir());
            let relative = f.path.strip_prefix(&default_dir).unwrap_or(&f.path);
            root.join(&output_dir).join(relative)
        })
        .collect();

    // 8b. Include skill files in targets
    let (skill_dirs, _) = project::resolve_all_skills(&config, &root);
    for skill_dir in &skill_dirs {
        let skill_name = skill_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let dest = root.join(&output_dir).join("skills").join(skill_name);
        if dest.exists() {
            collect_files_recursive(&dest, &mut targets);
        }
    }

    // 8c. Include prompt files in targets
    let (prompt_paths, _) = project::resolve_all_prompts(&config, &root);
    for prompt_path in &prompt_paths {
        let filename = prompt_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.md");
        let dest = root.join(&output_dir).join("prompts").join(filename);
        if dest.exists() {
            targets.push(dest);
        }
    }

    // 9. Optionally include armadai.yaml itself
    if with_config {
        let config_path = root.join("armadai.yaml");
        if config_path.exists() {
            targets.push(config_path);
        } else {
            let alt = root.join("armadai.yml");
            if alt.exists() {
                targets.push(alt);
            }
        }
    }

    // 10. Dry run
    if dry_run {
        println!(
            "Dry run — files that would be removed for '{}':\n",
            target_name
        );
        let mut existing = 0;
        for path in &targets {
            if path.exists() {
                println!("  {}", path.display());
                existing += 1;
            } else {
                println!("  {} (already absent)", path.display());
            }
        }
        println!(
            "\n  {} existing, {} already absent.",
            existing,
            targets.len() - existing
        );
        return Ok(());
    }

    // 11. Delete existing files
    let mut deleted = 0;
    let mut absent = 0;

    for path in &targets {
        if path.exists() {
            std::fs::remove_file(path)?;
            println!("  deleted {}", path.display());
            deleted += 1;
        } else {
            absent += 1;
        }
    }

    // 12. Clean up empty ancestor directories
    let stop_at = &root;
    for path in &targets {
        if let Some(parent) = path.parent() {
            remove_empty_ancestors(parent, stop_at);
        }
    }

    println!(
        "\nUnlinked '{}': {} deleted, {} already absent.",
        target_name, deleted, absent
    );

    Ok(())
}

/// Recursively collect all file paths under a directory.
fn collect_files_recursive(dir: &Path, targets: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, targets);
        } else if path.is_file() {
            targets.push(path);
        }
    }
}

/// Walk up from `path` removing empty directories, stopping at `stop_at` (exclusive).
fn remove_empty_ancestors(path: &Path, stop_at: &Path) {
    let mut current = path.to_path_buf();
    while current.starts_with(stop_at) && current != stop_at {
        if std::fs::read_dir(&current)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
        {
            if std::fs::remove_dir(&current).is_err() {
                break;
            }
        } else {
            break;
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
}
