use std::path::{Path, PathBuf};

use crate::core::project::{self, ProjectConfig};
use crate::linker::model_aliases::resolve_alias;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct DeprecationFinding {
    pub agent_path: PathBuf,
    pub agent_name: String,
    pub field: String,
    pub current: String,
    pub replacement: String,
}

// ---------------------------------------------------------------------------
// Check
// ---------------------------------------------------------------------------

/// Check a single agent file for deprecated model references.
pub fn check_agent_file(path: &Path) -> Vec<DeprecationFinding> {
    let agent = match crate::parser::parse_agent_file(path) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let mut findings = Vec::new();

    if let Some(ref model) = agent.metadata.model
        && let Some(replacement) = resolve_alias(model)
    {
        findings.push(DeprecationFinding {
            agent_path: path.to_path_buf(),
            agent_name: agent.name.clone(),
            field: "model".to_string(),
            current: model.clone(),
            replacement,
        });
    }

    for (i, fb) in agent.metadata.model_fallback.iter().enumerate() {
        if let Some(replacement) = resolve_alias(fb) {
            findings.push(DeprecationFinding {
                agent_path: path.to_path_buf(),
                agent_name: agent.name.clone(),
                field: format!("model_fallback[{i}]"),
                current: fb.clone(),
                replacement,
            });
        }
    }

    findings
}

/// Check all agents in a project for deprecated models.
pub fn check_project(project_root: &Path) -> anyhow::Result<Vec<DeprecationFinding>> {
    let config = load_project_config(project_root)?;
    let (paths, _errors) = project::resolve_all_agents(&config, project_root);

    let mut all_findings = Vec::new();
    for path in &paths {
        all_findings.extend(check_agent_file(path));
    }

    Ok(all_findings)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Update deprecated models in an agent file in-place.
/// Returns the number of replacements made.
pub fn update_agent_file(path: &Path, findings: &[DeprecationFinding]) -> anyhow::Result<usize> {
    if findings.is_empty() {
        return Ok(0);
    }

    let mut content = std::fs::read_to_string(path)?;
    let mut count = 0;

    for finding in findings {
        // Replace `model: <old>` patterns (handles both model and model_fallback values)
        let old_pattern = format!(": {}", finding.current);
        let new_pattern = format!(": {}", finding.replacement);

        if content.contains(&old_pattern) {
            content = content.replacen(&old_pattern, &new_pattern, 1);
            count += 1;
        }

        // Also handle fallback values that appear as list items: `  - <old>`
        let old_list_item = format!("- {}", finding.current);
        let new_list_item = format!("- {}", finding.replacement);
        if content.contains(&old_list_item) {
            content = content.replacen(&old_list_item, &new_list_item, 1);
            // Only count if we didn't already count from the `: ` pattern
            if count == 0 {
                count += 1;
            }
        }
    }

    if count > 0 {
        std::fs::write(path, content)?;
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_project_config(project_root: &Path) -> anyhow::Result<ProjectConfig> {
    let config_path = project_root.join(".armadai").join("config.yaml");
    if config_path.is_file() {
        return ProjectConfig::load(&config_path);
    }

    for name in &["armadai.yaml", "armadai.yml"] {
        let path = project_root.join(name);
        if path.is_file() {
            return ProjectConfig::load(&path);
        }
    }

    anyhow::bail!("No project config found in {}", project_root.display())
}

// ---------------------------------------------------------------------------
// Auto-check & interactive prompt
// ---------------------------------------------------------------------------

/// Auto-check deprecated models in a project and optionally prompt for update.
///
/// - If `interactive` is true and deprecations found: prompt user with dialoguer::Confirm
/// - If `interactive` is false: print hint to stderr
///
/// Returns true if models were updated.
pub fn auto_check_and_prompt(project_root: &Path, interactive: bool) -> bool {
    let findings = match check_project(project_root) {
        Ok(f) if !f.is_empty() => f,
        _ => return false,
    };

    // Print summary to stderr
    eprintln!("\nhint: {} deprecated model(s) found:", findings.len());
    for f in &findings {
        eprintln!(
            "  {} [{}]: {} -> {}",
            f.agent_name, f.field, f.current, f.replacement
        );
    }

    if !interactive {
        eprintln!("hint: run `armadai models update` to fix.\n");
        return false;
    }

    // Interactive prompt
    let confirm = dialoguer::Confirm::new()
        .with_prompt("Update deprecated models now?")
        .default(true)
        .interact()
        .unwrap_or(false);

    if confirm {
        let mut total = 0;
        for f in &findings {
            if let Ok(n) = update_agent_file(&f.agent_path, std::slice::from_ref(f)) {
                total += n;
            }
        }
        if total > 0 {
            eprintln!("  Updated {total} model(s).\n");
        }
        return true;
    }

    eprintln!("hint: run `armadai models update` when ready.\n");
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn create_agent_md(name: &str, model: &str, fallbacks: &[&str]) -> String {
        let mut md =
            format!("# {name}\n\n## Metadata\n\n```yaml\nprovider: anthropic\nmodel: {model}\n");
        if !fallbacks.is_empty() {
            let fb_str = fallbacks.join(", ");
            md.push_str(&format!("model_fallback: [{fb_str}]\n"));
        }
        md.push_str("```\n\n## System Prompt\n\nYou are a helpful assistant.\n");
        md
    }

    #[test]
    fn test_check_no_deprecation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(&path, create_agent_md("Test", "claude-sonnet-4-5", &[])).unwrap();

        let findings = check_agent_file(&path);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_check_deprecated_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(&path, create_agent_md("Test", "gpt-4-turbo", &[])).unwrap();

        let findings = check_agent_file(&path);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field, "model");
        assert_eq!(findings[0].current, "gpt-4-turbo");
        assert_eq!(findings[0].replacement, "gpt-4o");
    }

    #[test]
    fn test_check_deprecated_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(
            &path,
            create_agent_md("Test", "claude-sonnet-4-5", &["gemini-3.0-pro"]),
        )
        .unwrap();

        let findings = check_agent_file(&path);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field, "model_fallback[0]");
        assert_eq!(findings[0].current, "gemini-3.0-pro");
        assert_eq!(findings[0].replacement, "gemini-2.5-pro");
    }

    #[test]
    fn test_update_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        let original = create_agent_md("Test", "gpt-4-turbo", &[]);
        std::fs::write(&path, &original).unwrap();

        let findings = check_agent_file(&path);
        let count = update_agent_file(&path, &findings).unwrap();
        assert_eq!(count, 1);

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("model: gpt-4o"));
        assert!(!updated.contains("model: gpt-4-turbo"));
    }

    #[test]
    fn test_update_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        let original = create_agent_md("My Agent", "gpt-4-turbo", &[]);
        std::fs::write(&path, &original).unwrap();

        let findings = check_agent_file(&path);
        update_agent_file(&path, &findings).unwrap();

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("# My Agent"));
        assert!(updated.contains("provider: anthropic"));
        assert!(updated.contains("You are a helpful assistant."));
    }

    #[test]
    fn test_update_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        let original = create_agent_md("Test", "claude-sonnet-4-5", &[]);
        std::fs::write(&path, &original).unwrap();

        let count = update_agent_file(&path, &[]).unwrap();
        assert_eq!(count, 0);

        // Content unchanged
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, original);
    }

    #[test]
    fn test_check_project_with_agents() {
        let dir = tempfile::tempdir().unwrap();

        // Create project config
        let armadai_dir = dir.path().join(".armadai");
        std::fs::create_dir_all(armadai_dir.join("agents")).unwrap();
        std::fs::write(
            armadai_dir.join("config.yaml"),
            "agents:\n  - name: test-agent\n",
        )
        .unwrap();

        // Create agent with deprecated model
        std::fs::write(
            armadai_dir.join("agents").join("test-agent.md"),
            create_agent_md("Test Agent", "gpt-3.5-turbo", &[]),
        )
        .unwrap();

        let findings = check_project(dir.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].current, "gpt-3.5-turbo");
        assert_eq!(findings[0].replacement, "gpt-4o-mini");
    }
}
