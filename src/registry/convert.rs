use std::path::{Path, PathBuf};

use super::cache::converted_dir;
use super::sync::repo_dir;
use crate::core::config::user_agents_dir;

/// Convert a Copilot `.agent.md` file to ArmadAI Markdown format.
///
/// Copilot format:
/// ```markdown
/// ---
/// name: Agent Name
/// description: What it does
/// tools: [tool1, tool2]
/// ---
/// Instructions body
/// ```
///
/// ArmadAI format:
/// ```markdown
/// # Agent Name
///
/// ## Metadata
/// - provider: anthropic
/// - model: claude-sonnet-4-5-20250929
/// - tags: []
///
/// ## System Prompt
///
/// <instructions body>
/// ```
pub fn convert_to_armadai(content: &str, fallback_name: &str) -> String {
    let (frontmatter, body) = crate::parser::frontmatter::extract_frontmatter(content);

    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut tools: Vec<String> = Vec::new();

    if let Some(fm) = frontmatter {
        for line in fm.lines() {
            let trimmed = line.trim();
            if let Some(val) = trimmed.strip_prefix("name:") {
                name = val.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(val) = trimmed.strip_prefix("description:") {
                description = val.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(val) = trimmed.strip_prefix("tools:") {
                let cleaned = val.trim().trim_start_matches('[').trim_end_matches(']');
                tools = cleaned
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }

    // If body starts with H1, use it as the name instead
    let body_trimmed = body.trim();
    let system_prompt = if let Some(rest) = body_trimmed.strip_prefix("# ") {
        let first_line_end = rest.find('\n').unwrap_or(rest.len());
        name = rest[..first_line_end].trim().to_string();
        rest[first_line_end..].trim().to_string()
    } else {
        body_trimmed.to_string()
    };

    let mut output = format!(
        "# {name}\n\n## Metadata\n- provider: anthropic\n- model: claude-sonnet-4-5-20250929\n- temperature: 0.5\n- max_tokens: 4096\n"
    );

    if !tools.is_empty() {
        output.push_str(&format!("- tags: [{}]\n", tools.join(", ")));
    }

    if !description.is_empty() {
        output.push_str(&format!("\n## Instructions\n\n{description}\n"));
    }

    output.push_str(&format!("\n## System Prompt\n\n{system_prompt}\n"));

    output
}

/// Convert a registry agent and cache the result.
///
/// `registry_path` is relative to the repo root (e.g. "agents/official/security.agent.md").
pub fn convert_cached(registry_path: &str) -> anyhow::Result<PathBuf> {
    let repo = repo_dir();
    let src = repo.join(registry_path);

    if !src.is_file() {
        anyhow::bail!("Registry file not found: {}", src.display());
    }

    let cache_dir = converted_dir();
    std::fs::create_dir_all(&cache_dir)?;

    // Derive output filename
    let stem = Path::new(registry_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("agent");
    let out_name = format!("{}.md", stem.trim_end_matches(".agent"));
    let dst = cache_dir.join(&out_name);

    let content = std::fs::read_to_string(&src)?;
    let converted = convert_to_armadai(&content, stem);
    std::fs::write(&dst, &converted)?;

    Ok(dst)
}

/// Import a registry agent into the user library (~/.config/armadai/agents/).
pub fn import_to_library(registry_path: &str, force: bool) -> anyhow::Result<PathBuf> {
    let cached = convert_cached(registry_path)?;

    let agents_dir = user_agents_dir();
    std::fs::create_dir_all(&agents_dir)?;

    let filename = cached.file_name().unwrap();
    let dst = agents_dir.join(filename);

    if dst.exists() && !force {
        anyhow::bail!(
            "Agent already exists at {}. Use --force to overwrite.",
            dst.display()
        );
    }

    std::fs::copy(&cached, &dst)?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_copilot_format() {
        let content = r#"---
name: Security Scanner
description: Scan code for vulnerabilities
tools: [codebase, web-search]
---
You are a security expert. Analyze the codebase for OWASP Top 10 vulnerabilities.

Focus on:
- SQL injection
- XSS
- Authentication flaws
"#;

        let result = convert_to_armadai(content, "security-scanner");
        assert!(result.starts_with("# Security Scanner"));
        assert!(result.contains("## Metadata"));
        assert!(result.contains("- provider: anthropic"));
        assert!(result.contains("- tags: [codebase, web-search]"));
        assert!(result.contains("## System Prompt"));
        assert!(result.contains("OWASP Top 10"));
    }

    #[test]
    fn test_convert_plain_markdown() {
        let content = "# My Agent\n\nYou are a helpful assistant.\n\nDo things well.";
        let result = convert_to_armadai(content, "my-agent");
        assert!(result.starts_with("# My Agent"));
        assert!(result.contains("## System Prompt"));
        assert!(result.contains("You are a helpful assistant."));
    }

    #[test]
    fn test_convert_no_frontmatter() {
        let content = "Just some instructions without any structure.";
        let result = convert_to_armadai(content, "plain");
        assert!(result.starts_with("# plain"));
        assert!(result.contains("## System Prompt"));
        assert!(result.contains("Just some instructions"));
    }
}
