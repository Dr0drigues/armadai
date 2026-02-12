use std::path::Path;

use crate::core::agent::Agent;
use crate::core::config::AppPaths;
use crate::core::project;
use crate::parser;

pub async fn execute(agent_name: Option<String>) -> anyhow::Result<()> {
    match agent_name {
        Some(name) => validate_one(&name),
        None => validate_all(),
    }
}

fn validate_one(name: &str) -> anyhow::Result<()> {
    // Try project config first
    if let Some((root, config)) = project::find_project_config()
        && let Some(agent_ref) = config.agents.iter().find(|r| match r {
            project::AgentRef::Named { name: n } => n == name,
            project::AgentRef::Path { path } => path.file_stem().is_some_and(|s| s == name),
            project::AgentRef::Registry { registry } => registry.ends_with(name),
        })
    {
        let path = project::resolve_agent(agent_ref, &root)?;
        return validate_file(&path, name);
    }

    // Fallback to default paths
    let paths = AppPaths::resolve();
    let agents_dir = paths.agents_dir.as_path();
    let path = Agent::find_file(agents_dir, name).ok_or_else(|| {
        anyhow::anyhow!(
            "Agent '{name}' not found in {}/ (looked for {name}.md)",
            agents_dir.display()
        )
    })?;
    validate_file(&path, name)
}

fn validate_file(path: &Path, name: &str) -> anyhow::Result<()> {
    match parser::validate_agent(path) {
        Ok(agent) => {
            println!("  OK  {}", agent.name);
            Ok(())
        }
        Err(e) => {
            eprintln!("  FAIL  {name} ({})", path.display());
            eprintln!("        {e}");
            anyhow::bail!("Validation failed");
        }
    }
}

fn validate_all() -> anyhow::Result<()> {
    // If a project config exists with agents, validate those
    if let Some((root, config)) = project::find_project_config()
        && !config.agents.is_empty()
    {
        return validate_project_agents(&root, &config);
    }

    // Fallback to all agents in the default directory
    let paths = AppPaths::resolve();
    let agents_dir = paths.agents_dir.as_path();
    validate_dir(agents_dir)
}

fn validate_project_agents(root: &Path, config: &project::ProjectConfig) -> anyhow::Result<()> {
    let (paths, errors) = project::resolve_all_agents(config, root);

    for err in &errors {
        eprintln!("  SKIP  {err}");
    }

    let mut passed = 0;
    let mut failed = 0;

    for path in &paths {
        let display_name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        match parser::validate_agent(path) {
            Ok(agent) => {
                println!("  OK    {:<30}  ({})", agent.name, path.display());
                passed += 1;
            }
            Err(e) => {
                eprintln!("  FAIL  {:<30}  ({})", display_name, e);
                failed += 1;
            }
        }
    }

    println!();
    println!(
        "  {} agent(s) checked: {} passed, {} failed, {} skipped.",
        passed + failed,
        passed,
        failed,
        errors.len()
    );

    if failed > 0 {
        anyhow::bail!("{failed} agent(s) failed validation");
    }

    Ok(())
}

fn validate_dir(agents_dir: &Path) -> anyhow::Result<()> {
    if !agents_dir.exists() {
        anyhow::bail!("No agents directory found at {}/", agents_dir.display());
    }

    let mut files = Vec::new();
    collect_md_files(agents_dir, &mut files)?;
    files.sort();

    if files.is_empty() {
        println!("No agent files found in {}/", agents_dir.display());
        return Ok(());
    }

    let mut passed = 0;
    let mut failed = 0;

    for path in &files {
        let display_name = path.strip_prefix(agents_dir).unwrap_or(path).display();

        match parser::validate_agent(path) {
            Ok(agent) => {
                println!("  OK    {:<30}  ({})", agent.name, display_name);
                passed += 1;
            }
            Err(e) => {
                eprintln!("  FAIL  {:<30}  ({})", display_name, e);
                failed += 1;
            }
        }
    }

    println!();
    println!(
        "  {} agent(s) checked: {} passed, {} failed.",
        passed + failed,
        passed,
        failed
    );

    if failed > 0 {
        anyhow::bail!("{failed} agent(s) failed validation");
    }

    Ok(())
}

fn collect_md_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }
    Ok(())
}
