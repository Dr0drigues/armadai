//! Audit of native agentic configurations (adoption funnel).
//!
//! Reads native CLI configs (Claude Code first) through `ReverseLinker`s,
//! runs static rules over the imported assets and produces an `AuditReport`.
pub mod report;
pub mod reverse;
pub mod rules;

use std::path::Path;

use report::AuditReport;
use reverse::ReverseLinker;

/// Detect, import and analyse every native surface under `root`.
pub fn run_audit(root: &Path, settings: &rules::AuditSettings) -> AuditReport {
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
    }
}
