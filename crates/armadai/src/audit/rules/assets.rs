use super::{AuditContext, Finding, Severity};
use crate::audit::reverse::{AgentFormat, ImportedAgent};

/// What to tell the user to fix, per format. A01 fires on files in two
/// different formats, and a single sentence cannot be true of both: telling
/// the owner of an ArmadAI agent to "fix the YAML frontmatter" names a
/// construct that format does not have — measured on one real library, that
/// was the advice printed for `my-agent.md`, whose actual defect is a missing
/// `## Metadata` section.
fn parse_fix_hint(format: AgentFormat) -> &'static str {
    match format {
        AgentFormat::ClaudeFrontmatter => "fix the YAML frontmatter so tools can read this file",
        AgentFormat::Armadai => {
            "restore the sections an ArmadAI agent needs: an H1 title, `## Metadata` \
             carrying a `provider:`, and `## System Prompt`"
        }
    }
}

/// A01 — a native file could not be fully parsed.
pub(super) fn a01_unparsable(ctx: &AuditContext) -> Vec<Finding> {
    let agent_issues = ctx
        .config
        .agents
        .iter()
        .flat_map(|a| a.issues.iter().map(move |i| (i, a.format)));
    // A skill is a `SKILL.md` with YAML frontmatter whatever installed it, so
    // there is only one shape of advice to give about one that fails to parse.
    let skill_issues = ctx
        .config
        .skills
        .iter()
        .flat_map(|s| s.issues.iter().map(|i| (i, AgentFormat::ClaudeFrontmatter)));
    agent_issues
        .chain(skill_issues)
        .map(|(i, format)| Finding {
            rule: "A01",
            severity: Severity::Critical,
            file: i.file.clone(),
            related: Vec::new(),
            message: i.message.clone(),
            suggestion: Some(parse_fix_hint(format).to_string()),
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
///
/// Only agents whose format *can* carry a tool list are considered, on both
/// sides of the ratio. An ArmadAI-format file has no syntax for one
/// (`AgentMetadata` has no such field, and neither does the `## Metadata`
/// grammar), so counting it as permissive measures the reader rather than the
/// fleet: measured on a real 76-agent ArmadAI library it produced
/// `76/76 inherit all tools`, and adding a single tool-restricted native
/// agent to the same library would have turned that Info into a fleet-wide
/// Warning. Neither statement is about anything the user can act on.
pub(super) fn a08_permissive_tools(ctx: &AuditContext) -> Vec<Finding> {
    // Anti-cascade: parse-broken agents are A01's job.
    let agents: Vec<&ImportedAgent> = ctx
        .config
        .agents
        .iter()
        .filter(|a| a.issues.is_empty())
        .filter(|a| a.format.declares_tools())
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
    use crate::audit::rules::test_support::{agent, armadai_agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings, Severity};

    fn ctx<'a>(
        config: &'a crate::audit::reverse::ImportedConfig,
        settings: &'a AuditSettings,
    ) -> AuditContext<'a> {
        AuditContext {
            config,
            settings,
            usage: None,
        }
    }

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

    /// A08 asks whether an agent restricts its tools. An ArmadAI-format file
    /// cannot answer: the format has no tool list. Measured on a real
    /// 76-agent library, reporting them anyway printed
    /// `76/76 parsed agents inherit all tools` — a fact about the reader.
    #[test]
    fn a08_never_reports_a_format_that_cannot_declare_tools() {
        let config = config_with(vec![
            armadai_agent("capitaine", "You coordinate."),
            armadai_agent("vigie", "You watch."),
        ]);
        let settings = AuditSettings::default();

        assert!(
            a08_permissive_tools(&ctx(&config, &settings)).is_empty(),
            "an all-ArmadAI fleet declares no tools and must produce no A08"
        );
    }

    /// And the ratio: one native agent alongside them must not become
    /// `1/3 permissive` — nor, worse, flip the whole fleet's Info into a
    /// Warning by looking "mixed". The denominator counts only the files that
    /// could have declared a tool list.
    #[test]
    fn a08_counts_only_the_agents_whose_format_declares_tools() {
        let mut open = agent("native-open", "Body");
        open.metadata.tools = None;
        let config = config_with(vec![
            open,
            agent("native-locked", "Body"), // tools: [Read]
            armadai_agent("capitaine", "You coordinate."),
            armadai_agent("vigie", "You watch."),
        ]);
        let settings = AuditSettings::default();

        let f = a08_permissive_tools(&ctx(&config, &settings));

        assert_eq!(f.len(), 1);
        assert!(
            f[0].message.contains("1/2"),
            "only the two native agents count, got: {}",
            f[0].message
        );
        assert!(
            !f[0].message.contains("1/4") && !f[0].message.contains("3/4"),
            "the ArmadAI files must be out of both sides of the ratio, got: {}",
            f[0].message
        );
    }

    /// A01 fires on two formats, and "fix the YAML frontmatter" is false for
    /// one of them: an ArmadAI agent has none. Measured on a real library, the
    /// only critical it produced (`my-agent.md`, missing `## Metadata`) came
    /// with exactly that unusable advice.
    #[test]
    fn a01_advises_the_format_the_broken_file_is_actually_in() {
        let mut native = agent("native-broken", "Body");
        native.issues.push(ParseIssue {
            file: native.source_path.clone(),
            message: "missing YAML frontmatter".to_string(),
        });
        let mut mine = armadai_agent("my-agent", "");
        mine.issues.push(ParseIssue {
            file: mine.source_path.clone(),
            message: "Missing ## Metadata section".to_string(),
        });
        let config = config_with(vec![mine, native]);
        let settings = AuditSettings::default();

        let f = a01_unparsable(&ctx(&config, &settings));

        assert_eq!(f.len(), 2, "{f:?}");
        let armadai = f
            .iter()
            .find(|f| f.file.ends_with("my-agent.md"))
            .expect("the ArmadAI file must be reported");
        let hint = armadai.suggestion.as_deref().unwrap_or("");
        assert!(
            hint.contains("## Metadata") && hint.contains("H1"),
            "an ArmadAI agent must be told about its own sections, got: {hint}"
        );
        assert!(
            !hint.contains("frontmatter"),
            "this format has no frontmatter to fix, got: {hint}"
        );
        let native = f
            .iter()
            .find(|f| f.file.ends_with("native-broken.md"))
            .expect("the native file must be reported");
        assert!(
            native
                .suggestion
                .as_deref()
                .unwrap_or("")
                .contains("frontmatter"),
            "and the native advice must be unchanged, got: {:?}",
            native.suggestion
        );
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
                    body_tokens: 0,
                    issues: Vec::new(),
                    extra: BTreeMap::new(),
                    space: ".claude".into(),
                },
                ImportedSkill {
                    name: "fine".into(),
                    source_path: ".claude/skills/fine/SKILL.md".into(),
                    description: Some("ok".into()),
                    has_skill_md: true,
                    has_frontmatter: true,
                    body_tokens: 0,
                    issues: Vec::new(),
                    extra: BTreeMap::new(),
                    space: ".claude".into(),
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
                body_tokens: 0,
                issues: Vec::new(),
                extra: BTreeMap::new(),
                space: ".claude".into(),
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
                body_tokens: 0,
                issues: vec![ParseIssue {
                    file: ".claude/skills/triage/SKILL.md".into(),
                    message: "unquoted value".into(),
                }],
                extra: BTreeMap::new(),
                space: ".claude".into(),
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
                    body_tokens: 0,
                    issues: vec![ParseIssue {
                        file: ".claude/skills/broken/SKILL.md".into(),
                        message: "bad".into(),
                    }],
                    extra: BTreeMap::new(),
                    space: ".claude".into(),
                },
                // No frontmatter at all: A09 Warning.
                ImportedSkill {
                    name: "bare".into(),
                    source_path: ".claude/skills/bare/SKILL.md".into(),
                    description: None,
                    has_skill_md: true,
                    has_frontmatter: false,
                    body_tokens: 0,
                    issues: Vec::new(),
                    extra: BTreeMap::new(),
                    space: ".claude".into(),
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
