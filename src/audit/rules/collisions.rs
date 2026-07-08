//! Collision rules (C01-C05): conflicting claims between agentic assets.
use std::collections::BTreeMap;

use super::{AuditContext, Finding, Severity};

/// C01 — two assets claim the same name.
/// Same-kind duplicates are Critical (routing is ambiguous); an agent/skill
/// homonym is Warning (different namespaces, but confusing).
pub(super) fn c01_name_collisions(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut agents_by_name: BTreeMap<&str, Vec<&std::path::Path>> = BTreeMap::new();
    for a in ctx.config.agents.iter().filter(|a| a.issues.is_empty()) {
        agents_by_name
            .entry(a.name.as_str())
            .or_default()
            .push(&a.source_path);
    }
    let mut skills_by_name: BTreeMap<&str, Vec<&std::path::Path>> = BTreeMap::new();
    for s in ctx.config.skills.iter().filter(|s| s.issues.is_empty()) {
        skills_by_name
            .entry(s.name.as_str())
            .or_default()
            .push(&s.source_path);
    }
    for (kind, by_name) in [("agent", &agents_by_name), ("skill", &skills_by_name)] {
        for (name, paths) in by_name {
            if paths.len() > 1 {
                findings.push(Finding {
                    rule: "C01",
                    severity: Severity::Critical,
                    file: paths[0].to_path_buf(),
                    related: paths[1..].iter().map(|p| p.to_path_buf()).collect(),
                    message: format!(
                        "{} {kind} files share the name '{name}' — routing is ambiguous",
                        paths.len()
                    ),
                    suggestion: Some("rename or remove the duplicates".to_string()),
                });
            }
        }
    }
    for (name, agent_paths) in &agents_by_name {
        if let Some(skill_paths) = skills_by_name.get(name) {
            findings.push(Finding {
                rule: "C01",
                severity: Severity::Warning,
                file: agent_paths[0].to_path_buf(),
                related: skill_paths.iter().map(|p| p.to_path_buf()).collect(),
                message: format!("agent and skill share the name '{name}'"),
                suggestion: Some("give the skill or the agent a distinct name".to_string()),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings, Severity};

    #[test]
    fn c01_flags_duplicate_agent_names() {
        let mut a = agent("reviewer", "Body");
        a.source_path = ".claude/agents/backend/reviewer.md".into();
        let b = agent("reviewer", "Body");
        let c = agent("other", "Body");
        let config = config_with(vec![a, b, c]);
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].message.contains("reviewer"));
        assert_eq!(f[0].related.len(), 1);
    }

    #[test]
    fn c01_flags_agent_skill_homonym_as_warning() {
        use crate::audit::reverse::{ImportedConfig, ImportedSkill};
        use std::collections::BTreeMap;
        let config = ImportedConfig {
            agents: vec![agent("deploy", "Body")],
            skills: vec![ImportedSkill {
                name: "deploy".into(),
                source_path: ".claude/skills/deploy/SKILL.md".into(),
                description: Some("d".into()),
                has_skill_md: true,
                has_frontmatter: true,
                issues: Vec::new(),
                extra: BTreeMap::new(),
            }],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warning);
    }
}
