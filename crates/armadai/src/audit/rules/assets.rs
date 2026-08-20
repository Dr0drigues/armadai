use super::{AuditContext, Finding, Severity};
use crate::audit::reverse::ImportedAgent;

/// A01 — a native file could not be fully parsed.
pub(super) fn a01_unparsable(ctx: &AuditContext) -> Vec<Finding> {
    let agent_issues = ctx.config.agents.iter().flat_map(|a| a.issues.iter());
    let skill_issues = ctx.config.skills.iter().flat_map(|s| s.issues.iter());
    agent_issues
        .chain(skill_issues)
        .map(|i| Finding {
            rule: "A01",
            severity: Severity::Critical,
            file: i.file.clone(),
            related: Vec::new(),
            message: i.message.clone(),
            suggestion: Some("fix the YAML frontmatter so tools can read this file".to_string()),
        })
        .collect()
}

/// A02 — required descriptive fields are missing.
pub(super) fn a02_missing_fields(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .agents
        .iter()
        // Anti-cascade: parse-broken agents are A01's job (one root cause,
        // one finding); their fields are unreliable defaults.
        .filter(|a| a.issues.is_empty())
        .filter(|a| a.metadata.description.is_none())
        .map(|a| Finding {
            rule: "A02",
            severity: Severity::Warning,
            file: a.source_path.clone(),
            related: Vec::new(),
            message: format!("agent '{}' has no description", a.name),
            suggestion: Some(
                "add a `description:` field (used for routing and discovery)".to_string(),
            ),
        })
        .collect()
}

/// A05 — system prompt exceeds the configured token estimate.
pub(super) fn a05_oversized_prompt(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .agents
        .iter()
        // Anti-cascade: parse-broken agents are A01's job (one root cause,
        // one finding); their fields are unreliable defaults.
        .filter(|a| a.issues.is_empty())
        .filter_map(|a| {
            let estimate = super::estimate_tokens(&a.system_prompt);
            (estimate > ctx.settings.prompt_token_threshold).then(|| Finding {
                rule: "A05",
                severity: Severity::Warning,
                file: a.source_path.clone(),
                related: Vec::new(),
                message: format!(
                    "agent '{}' prompt is ~{estimate} tokens (threshold {})",
                    a.name, ctx.settings.prompt_token_threshold
                ),
                suggestion: Some(
                    "split shared conventions into a reusable prompt fragment".to_string(),
                ),
            })
        })
        .collect()
}

/// A08 — agents without any tool restriction, aggregated fleet-level.
/// A uniform fleet is an assumed team choice (Info); a mixed fleet is a
/// real inconsistency (Warning).
pub(super) fn a08_permissive_tools(ctx: &AuditContext) -> Vec<Finding> {
    // Anti-cascade: parse-broken agents are A01's job.
    let agents: Vec<&ImportedAgent> = ctx
        .config
        .agents
        .iter()
        .filter(|a| a.issues.is_empty())
        .collect();
    let offenders: Vec<&ImportedAgent> = agents
        .iter()
        .copied()
        .filter(|a| match &a.metadata.tools {
            None => true,
            Some(tools) => tools.iter().any(|t| t == "*"),
        })
        .collect();
    let Some(first) = offenders.first() else {
        return Vec::new();
    };
    let severity = if offenders.len() == agents.len() {
        Severity::Info
    } else {
        Severity::Warning
    };
    vec![Finding {
        rule: "A08",
        severity,
        file: first.source_path.clone(),
        related: offenders[1..]
            .iter()
            .map(|a| a.source_path.clone())
            .collect(),
        message: format!(
            "{}/{} parsed agents inherit all tools (no restriction)",
            offenders.len(),
            agents.len()
        ),
        suggestion: Some("declare the minimal `tools:` list each agent needs".to_string()),
    }]
}

/// A09 — skill directory does not follow the Agent Skills standard.
/// Parse failures are A01's job: a skill carrying a ParseIssue is skipped
/// here so one root cause yields one finding.
pub(super) fn a09_malformed_skill(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .skills
        .iter()
        .filter(|s| s.issues.is_empty())
        .filter_map(|s| {
            let mut problems = Vec::new();
            if !s.has_skill_md {
                problems.push("missing SKILL.md");
            } else if !s.has_frontmatter {
                problems.push("missing frontmatter");
            }
            if s.has_skill_md && s.description.is_none() {
                problems.push("missing description");
            }
            (!problems.is_empty()).then(|| Finding {
                rule: "A09",
                severity: Severity::Warning,
                file: s.source_path.clone(),
                related: Vec::new(),
                message: format!("skill '{}': {}", s.name, problems.join(", ")),
                suggestion: Some(
                    "follow the Agent Skills standard: SKILL.md with name + description"
                        .to_string(),
                ),
            })
        })
        .collect()
}

/// Frontmatter fields documented by Claude Code (beyond the typed ones).
const DOCUMENTED_AGENT_FIELDS: &[&str] = &[
    "effort",
    "color",
    "permissionMode",
    "disallowedTools",
    "maxTurns",
    "skills",
    "hooks",
    "memory",
    "mcpServers",
    "background",
    "isolation",
    "initialPrompt",
];
// `tools` is deliberately excluded from the skill allowlist: it is
// non-standard for skills (Claude Code ignores it there) — `allowed-tools`
// is the documented field for restricting a skill's tool access.
const DOCUMENTED_SKILL_FIELDS: &[&str] = &[
    "version",
    "allowed-tools",
    "license",
    "metadata",
    "argument-hint",
    "model",
    "context",
    "agent",
    "disable-model-invocation",
    "user-invocable",
    "disallowed-tools",
    "effort",
    "hooks",
    "paths",
    "shell",
    "when_to_use",
    "arguments",
    "compatibility",
];

/// A12 — non-standard frontmatter fields, one aggregated Info finding.
/// They are kept verbatim (`extra`) so `--propose` can round-trip them;
/// this rule only surfaces their existence.
pub(super) fn a12_nonstandard_fields(ctx: &AuditContext) -> Vec<Finding> {
    use std::collections::BTreeMap;
    let mut per_field: BTreeMap<&str, usize> = BTreeMap::new();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let agent_fields = ctx
        .config
        .agents
        .iter()
        .filter(|a| a.issues.is_empty())
        .flat_map(|a| {
            a.metadata
                .extra
                .keys()
                .filter(|k| !DOCUMENTED_AGENT_FIELDS.contains(&k.as_str()))
                .map(|k| (k.as_str(), a.source_path.clone()))
        });
    let skill_fields = ctx
        .config
        .skills
        .iter()
        .filter(|s| s.issues.is_empty())
        .flat_map(|s| {
            s.extra
                .keys()
                .filter(|k| !DOCUMENTED_SKILL_FIELDS.contains(&k.as_str()))
                .map(|k| (k.as_str(), s.source_path.clone()))
        });
    for (field, path) in agent_fields.chain(skill_fields) {
        *per_field.entry(field).or_insert(0) += 1;
        if seen.insert(path.clone()) {
            files.push(path);
        }
    }
    let Some(first) = files.first().cloned() else {
        return Vec::new();
    };
    let breakdown: Vec<String> = per_field
        .iter()
        .map(|(field, count)| format!("{field} ({count})"))
        .collect();
    vec![Finding {
        rule: "A12",
        severity: Severity::Info,
        file: first,
        related: files[1..].to_vec(),
        message: format!(
            "non-standard frontmatter field(s) across {} file(s): {}",
            files.len(),
            breakdown.join(", ")
        ),
        suggestion: Some(
            "fields are kept as-is; document them or align with Claude Code standards".to_string(),
        ),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::reverse::ParseIssue;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings, Severity};

    #[test]
    fn a01_reports_each_parse_issue_as_critical() {
        let mut a = agent("broken", "Body");
        a.issues.push(ParseIssue {
            file: a.source_path.clone(),
            message: "invalid YAML frontmatter: mapping".to_string(),
        });
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let f = a01_unparsable(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A01");
        assert_eq!(f[0].severity, Severity::Critical);
    }

    #[test]
    fn a02_flags_missing_description() {
        let mut a = agent("bare", "Body");
        a.metadata.description = None;
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let f = a02_missing_fields(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A02");
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn a05_flags_prompt_over_threshold() {
        let a = agent("fat", &"word ".repeat(5000)); // ~6250 tokens estimés
        let config = config_with(vec![a]);
        let settings = AuditSettings::default(); // seuil 4000
        let f = a05_oversized_prompt(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A05");
    }

    #[test]
    fn a08_aggregates_mixed_fleet_as_warning() {
        let mut a = agent("wild", "Body");
        a.metadata.tools = None;
        let mut b = agent("star", "Body");
        b.metadata.tools = Some(vec!["*".to_string()]);
        let c = agent("ok", "Body"); // tools: [Read]
        let config = config_with(vec![a, b, c]);
        let settings = AuditSettings::default();
        let f = a08_permissive_tools(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warning); // mixed fleet
        assert!(f[0].message.contains("2/3"));
        assert_eq!(f[0].related.len(), 1);
    }

    #[test]
    fn a08_uniform_fleet_is_single_info() {
        let mut a = agent("one", "Body");
        a.metadata.tools = None;
        let mut b = agent("two", "Body");
        b.metadata.tools = None;
        let config = config_with(vec![a, b]);
        let settings = AuditSettings::default();
        let f = a08_permissive_tools(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Info);
        assert!(f[0].message.contains("2/2"));
    }

    #[test]
    fn a09_flags_broken_skills_once_each() {
        use crate::audit::reverse::{ImportedConfig, ImportedSkill};
        use std::collections::BTreeMap;
        let config = ImportedConfig {
            skills: vec![
                ImportedSkill {
                    name: "no-md".into(),
                    source_path: ".claude/skills/no-md".into(),
                    description: None,
                    has_skill_md: false,
                    has_frontmatter: false,
                    issues: Vec::new(),
                    extra: BTreeMap::new(),
                },
                ImportedSkill {
                    name: "fine".into(),
                    source_path: ".claude/skills/fine/SKILL.md".into(),
                    description: Some("ok".into()),
                    has_skill_md: true,
                    has_frontmatter: true,
                    issues: Vec::new(),
                    extra: BTreeMap::new(),
                },
            ],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = a09_malformed_skill(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A09");
    }

    #[test]
    fn a09_missing_skill_md_does_not_stack_missing_description() {
        use crate::audit::reverse::{ImportedConfig, ImportedSkill};
        use std::collections::BTreeMap;
        let config = ImportedConfig {
            skills: vec![ImportedSkill {
                name: "no-md".into(),
                source_path: ".claude/skills/no-md".into(),
                description: None,
                has_skill_md: false,
                has_frontmatter: false,
                issues: Vec::new(),
                extra: BTreeMap::new(),
            }],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = a09_malformed_skill(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("missing SKILL.md"));
        assert!(!f[0].message.contains("missing description"));
    }

    #[test]
    fn a01_covers_skill_parse_issues() {
        use crate::audit::reverse::{ImportedConfig, ImportedSkill, ParseIssue};
        use std::collections::BTreeMap;
        let config = ImportedConfig {
            skills: vec![ImportedSkill {
                name: "triage".into(),
                source_path: ".claude/skills/triage/SKILL.md".into(),
                description: Some("salvaged".into()),
                has_skill_md: true,
                has_frontmatter: true,
                issues: vec![ParseIssue {
                    file: ".claude/skills/triage/SKILL.md".into(),
                    message: "unquoted value".into(),
                }],
                extra: BTreeMap::new(),
            }],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = a01_unparsable(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Critical);
    }

    #[test]
    fn a09_does_not_double_report_parse_broken_skills() {
        use crate::audit::reverse::{ImportedConfig, ImportedSkill, ParseIssue};
        use std::collections::BTreeMap;
        let config = ImportedConfig {
            skills: vec![
                // Parse-broken skill: A01's job, A09 stays silent.
                ImportedSkill {
                    name: "broken".into(),
                    source_path: ".claude/skills/broken/SKILL.md".into(),
                    description: None,
                    has_skill_md: true,
                    has_frontmatter: true,
                    issues: vec![ParseIssue {
                        file: ".claude/skills/broken/SKILL.md".into(),
                        message: "bad".into(),
                    }],
                    extra: BTreeMap::new(),
                },
                // No frontmatter at all: A09 Warning.
                ImportedSkill {
                    name: "bare".into(),
                    source_path: ".claude/skills/bare/SKILL.md".into(),
                    description: None,
                    has_skill_md: true,
                    has_frontmatter: false,
                    issues: Vec::new(),
                    extra: BTreeMap::new(),
                },
            ],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = a09_malformed_skill(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("bare"));
        assert!(f[0].message.contains("missing frontmatter"));
    }

    #[test]
    fn field_rules_skip_agents_with_parse_issues() {
        let mut a = agent("broken", "Body");
        a.metadata.description = None;
        a.metadata.tools = None;
        a.issues.push(ParseIssue {
            file: a.source_path.clone(),
            message: "invalid".into(),
        });
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let ctx = AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        };
        assert!(a02_missing_fields(&ctx).is_empty());
        assert!(a08_permissive_tools(&ctx).is_empty());
    }

    #[test]
    fn a12_aggregates_nonstandard_fields_fleet_wide() {
        use serde_yaml_ng::Value;
        let mut a = agent("one", "Body");
        a.metadata
            .extra
            .insert("paths".into(), Value::String("src/**".into()));
        a.metadata
            .extra
            .insert("effort".into(), Value::String("medium".into())); // documented
        let mut b = agent("two", "Body");
        b.metadata
            .extra
            .insert("paths".into(), Value::String("docs/**".into()));
        b.metadata
            .extra
            .insert("phase".into(), Value::String("2".into()));
        let config = config_with(vec![a, b]);
        let settings = AuditSettings::default();
        let f = a12_nonstandard_fields(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Info);
        assert!(f[0].message.contains("paths (2)"));
        assert!(f[0].message.contains("phase (1)"));
        assert!(!f[0].message.contains("effort"));
        assert_eq!(f[0].related.len(), 1);
    }
}
