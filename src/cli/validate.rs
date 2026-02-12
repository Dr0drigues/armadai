use std::path::Path;

use crate::core::agent::Agent;
use crate::core::config::AppPaths;
use crate::parser;

pub async fn execute(agent_name: Option<String>) -> anyhow::Result<()> {
    let paths = AppPaths::resolve();
    let agents_dir = paths.agents_dir.as_path();

    match agent_name {
        Some(name) => validate_one(agents_dir, &name),
        None => validate_all(agents_dir),
    }
}

fn validate_one(agents_dir: &Path, name: &str) -> anyhow::Result<()> {
    let path = Agent::find_file(agents_dir, name).ok_or_else(|| {
        anyhow::anyhow!(
            "Agent '{name}' not found in {}/ (looked for {name}.md)",
            agents_dir.display()
        )
    })?;

    match parser::validate_agent(&path) {
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

fn validate_all(agents_dir: &Path) -> anyhow::Result<()> {
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
