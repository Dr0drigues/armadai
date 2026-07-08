//! Collision rules (C01-C05): conflicting claims between agentic assets.
use std::collections::BTreeMap;

use super::similarity::jaccard;
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

/// C03 — two activation surfaces (skill↔skill or agent↔skill) with
/// near-identical descriptions: the router cannot pick reliably.
/// Agent↔agent redundancy stays A07 (higher threshold, Info): merging two
/// agents is a design suggestion, an ambiguous skill trigger is a defect.
pub(super) fn c03_activation_overlap(ctx: &AuditContext) -> Vec<Finding> {
    let threshold = ctx.settings.activation_similarity;
    // (kind, name, path, description)
    let mut surfaces: Vec<(&str, &str, &std::path::Path, &str)> = Vec::new();
    for s in ctx.config.skills.iter().filter(|s| s.issues.is_empty()) {
        if let Some(d) = &s.description {
            surfaces.push(("skill", &s.name, &s.source_path, d));
        }
    }
    for a in ctx.config.agents.iter().filter(|a| a.issues.is_empty()) {
        if let Some(d) = &a.metadata.description {
            surfaces.push(("agent", &a.name, &a.source_path, d));
        }
    }
    let mut findings = Vec::new();
    for i in 0..surfaces.len() {
        for j in (i + 1)..surfaces.len() {
            let (ka, na, pa, da) = surfaces[i];
            let (kb, nb, pb, db) = surfaces[j];
            if ka == "agent" && kb == "agent" {
                continue; // A07's turf
            }
            if jaccard(da, db) >= threshold {
                findings.push(Finding {
                    rule: "C03",
                    severity: Severity::Warning,
                    file: pa.to_path_buf(),
                    related: vec![pb.to_path_buf()],
                    message: format!(
                        "{ka} '{na}' and {kb} '{nb}' have overlapping activation descriptions — routing is ambiguous"
                    ),
                    suggestion: Some(
                        "sharpen the descriptions so each one triggers on distinct intents"
                            .to_string(),
                    ),
                });
            }
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

    fn skill(name: &str, description: &str) -> crate::audit::reverse::ImportedSkill {
        crate::audit::reverse::ImportedSkill {
            name: name.into(),
            source_path: format!(".claude/skills/{name}/SKILL.md").into(),
            description: Some(description.into()),
            has_skill_md: true,
            has_frontmatter: true,
            issues: Vec::new(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn c03_flags_ambiguous_skill_descriptions() {
        use crate::audit::reverse::ImportedConfig;
        let config = ImportedConfig {
            skills: vec![
                skill("audit-a", "runs a full audit of the project quality"),
                skill("audit-b", "runs a full quality audit of the project"),
                skill("deploy", "ships the app to production servers"),
            ],
            ..Default::default()
        };
        let settings = AuditSettings::default(); // activation_similarity 0.6
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warning);
        assert!(f[0].message.contains("audit-a") && f[0].message.contains("audit-b"));
    }

    #[test]
    fn c03_crosses_agents_and_skills() {
        use crate::audit::reverse::ImportedConfig;
        let mut a = agent("checker", "Body");
        a.metadata.description = Some("reviews rust code for style and bugs".into());
        let config = ImportedConfig {
            agents: vec![a],
            skills: vec![skill("review", "reviews rust code for bugs and style")],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
    }
}
