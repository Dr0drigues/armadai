use std::path::{Path, PathBuf};

use crate::core::project;
use crate::linker::{self, LinkAgent};
use crate::parser;

pub async fn execute(
    target: Option<String>,
    coordinator_flag: Option<String>,
    dry_run: bool,
    force: bool,
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
                 Supported targets: claude, copilot, gemini"
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

    // 7. Generate files
    let sources = &config.sources;
    let files = linker.generate(&link_agents, coordinator.as_ref(), sources);

    if files.is_empty() {
        println!("No files to generate.");
        return Ok(());
    }

    // 8. Resolve output paths relative to project root
    let output_files: Vec<_> = files
        .into_iter()
        .map(|f| {
            // Replace the default output dir prefix with the custom output dir
            let default_dir = PathBuf::from(linker.default_output_dir());
            let relative = f.path.strip_prefix(&default_dir).unwrap_or(&f.path);
            let final_path = root.join(&output_dir).join(relative);
            (final_path, f.content)
        })
        .collect();

    // 8b. Resolve and collect skill files
    let (skill_dirs, skill_errors) = project::resolve_all_skills(&config, &root);
    for err in &skill_errors {
        eprintln!("  warn: {err}");
    }

    let mut extra_files: Vec<(PathBuf, String)> = Vec::new();
    let mut skill_count = 0;
    for skill_dir in &skill_dirs {
        if let Ok(entries) = collect_dir_files(skill_dir) {
            let skill_name = skill_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            for (relative, content) in entries {
                let final_path = root
                    .join(&output_dir)
                    .join("skills")
                    .join(skill_name)
                    .join(&relative);
                extra_files.push((final_path, content));
            }
            skill_count += 1;
        }
    }

    // 8c. Resolve and collect prompt files
    let (prompt_paths, prompt_errors) = project::resolve_all_prompts(&config, &root);
    for err in &prompt_errors {
        eprintln!("  warn: {err}");
    }

    let mut prompt_count = 0;
    for prompt_path in &prompt_paths {
        if let Ok(content) = std::fs::read_to_string(prompt_path) {
            let filename = prompt_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.md");
            let final_path = root.join(&output_dir).join("prompts").join(filename);
            extra_files.push((final_path, content));
            prompt_count += 1;
        }
    }

    // 9. Dry run or write
    if dry_run {
        println!(
            "Dry run — files that would be generated for '{}':\n",
            target_name
        );
        for (path, _) in &output_files {
            println!("  {}", path.display());
        }
        for (path, _) in &extra_files {
            println!("  {}", path.display());
        }
        println!(
            "\n  {} file(s) total.",
            output_files.len() + extra_files.len()
        );
        return Ok(());
    }

    let mut written = 0;
    let mut skipped = 0;

    for (path, content) in output_files.iter().chain(extra_files.iter()) {
        if path.exists() && !force {
            eprintln!(
                "  skip: {} already exists (use --force to overwrite)",
                path.display()
            );
            skipped += 1;
            continue;
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        println!("  wrote {}", path.display());
        written += 1;
    }

    let mut summary = format!("Linked {} agent(s)", link_agents.len());
    if skill_count > 0 {
        summary.push_str(&format!(", {} skill(s)", skill_count));
    }
    if prompt_count > 0 {
        summary.push_str(&format!(", {} prompt(s)", prompt_count));
    }
    println!(
        "\n{} to '{}': {} written, {} skipped.",
        summary, target_name, written, skipped
    );

    Ok(())
}

/// Collect all files from a directory recursively as (relative_path, content) pairs.
/// Only includes text files (valid UTF-8).
fn collect_dir_files(dir: &Path) -> anyhow::Result<Vec<(PathBuf, String)>> {
    let mut files = Vec::new();
    collect_dir_files_recursive(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn collect_dir_files_recursive(
    base: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, String)>,
) -> anyhow::Result<()> {
    let entries = std::fs::read_dir(current)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_files_recursive(base, &path, files)?;
        } else if path.is_file()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let relative = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
            files.push((relative, content));
        }
    }
    Ok(())
}
