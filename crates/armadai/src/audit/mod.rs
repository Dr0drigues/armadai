//! Audit of native agentic configurations (adoption funnel).
//!
//! Reads native CLI configs (Claude Code first) through `ReverseLinker`s,
//! runs static rules over the imported assets and produces an `AuditReport`.
pub mod deep;
pub mod proposal;
pub mod report;
pub mod reverse;
pub mod rules;
pub mod usage;

use std::path::Path;

use report::AuditReport;
use reverse::ReverseLinker;

/// Detect and parse every native surface under `root`.
/// Shared by the audit run and `--propose` (which needs the raw imports).
pub fn import_surfaces(root: &Path) -> (Vec<String>, reverse::ImportedConfig) {
    let linkers: Vec<Box<dyn ReverseLinker>> = vec![Box::new(reverse::claude::ClaudeReverseLinker)];
    let mut detected = Vec::new();
    let mut config = reverse::ImportedConfig::default();
    for linker in &linkers {
        if linker.detect(root) {
            detected.push(linker.name().to_string());
            let parsed = linker.parse(root);
            config.agents.extend(parsed.agents);
            config.skills.extend(parsed.skills);
            if config.instructions.is_none() {
                config.instructions = parsed.instructions;
            }
        }
    }
    (detected, config)
}

/// Detect, import and analyse every native surface under `root`.
pub fn run_audit(root: &Path, settings: &rules::AuditSettings) -> AuditReport {
    let (detected, config) = import_surfaces(root);
    let ctx = rules::AuditContext {
        config: &config,
        settings,
    };
    AuditReport {
        root: root.to_path_buf(),
        detected,
        agent_count: config.agents.len(),
        skill_count: config.skills.len(),
        findings: rules::run_rules(&ctx),
        deep_raw: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_surfaces_returns_detected_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("a.md"),
            "---\nname: a\ndescription: d\n---\nBody",
        )
        .unwrap();
        let (detected, config) = import_surfaces(dir.path());
        assert_eq!(detected, vec!["claude".to_string()]);
        assert_eq!(config.agents.len(), 1);
    }
}
