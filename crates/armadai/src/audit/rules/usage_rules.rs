//! U0x — rules over observed usage. Every rule is silent when no usage was
//! observed: absence of measurement is never evidence of absence of use.

use std::path::PathBuf;

use super::{AuditContext, Finding, Severity};
use crate::audit::usage::UsageFacts;

/// Sub-agents Claude Code provides itself. They are legitimately used without
/// ever appearing in `.claude/agents/`, which is exactly why U02 reports them:
/// ArmadAI has no implicit equivalent, so a migration must materialise them.
const BUILTIN_AGENTS: &[&str] = &["general-purpose", "Explore", "Plan", "claude"];

/// Share of delegations below which a declared coordinator counts as bypassed.
const COORDINATOR_SHARE: f64 = 0.5;

fn observed<'a>(ctx: &AuditContext<'a>) -> Option<&'a UsageFacts> {
    ctx.usage.filter(|u| !u.is_empty())
}

/// U01 — a declared asset that never ran over the observed sessions.
pub(super) fn u01_declared_never_used(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for agent in &ctx.config.agents {
        if usage.agents.contains_key(&agent.name) {
            continue;
        }
        findings.push(Finding {
            rule: "U01",
            severity: Severity::Warning,
            file: agent.source_path.clone(),
            related: vec![],
            message: format!(
                "agent '{}' is declared but was never invoked across {} observed session(s)",
                agent.name, usage.sessions
            ),
            suggestion: Some(
                "remove it, or exclude it from the generated pack (--propose tags it `unused`)"
                    .to_string(),
            ),
        });
    }
    findings
}

/// U02 — a sub-agent that ran without being declared anywhere.
pub(super) fn u02_used_but_undeclared(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let declared: Vec<&str> = ctx.config.agents.iter().map(|a| a.name.as_str()).collect();
    let mut findings = Vec::new();
    for (name, stats) in &usage.agents {
        if declared.contains(&name.as_str()) {
            continue;
        }
        let builtin = BUILTIN_AGENTS.contains(&name.as_str());
        findings.push(Finding {
            rule: "U02",
            severity: Severity::Info,
            file: ctx
                .config
                .instructions
                .as_ref()
                .map(|i| i.source_path.clone())
                .unwrap_or_else(|| PathBuf::from(".")),
            related: vec![],
            message: format!(
                "sub-agent '{}' ran {} time(s) but is declared nowhere{}",
                name,
                stats.invocations,
                if builtin {
                    " (it is built into Claude Code)"
                } else {
                    ""
                }
            ),
            suggestion: Some(
                "ArmadAI has no implicit equivalent — materialise it as an explicit agent \
                 so a migrated fleet keeps the same workers"
                    .to_string(),
            ),
        });
    }
    findings
}

/// U03 — the root instructions name a coordinator that delegations bypass.
pub(super) fn u03_coordinator_bypassed(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let Some(instructions) = ctx.config.instructions.as_ref() else {
        return Vec::new();
    };
    let total: u32 = usage.agents.values().map(|a| a.invocations).sum();
    if total == 0 {
        return Vec::new();
    }
    let haystack = instructions.content.to_lowercase();
    let mut findings = Vec::new();
    for agent in &ctx.config.agents {
        // Only agents the instructions actually single out as coordinating.
        let named = haystack.contains(&format!("@{}", agent.name.to_lowercase()))
            || haystack.contains(&format!("delegate to {}", agent.name.to_lowercase()));
        if !named {
            continue;
        }
        let own = usage
            .agents
            .get(&agent.name)
            .map(|a| a.invocations)
            .unwrap_or(0);
        let share = f64::from(own) / f64::from(total);
        if share >= COORDINATOR_SHARE {
            continue;
        }
        findings.push(Finding {
            rule: "U03",
            severity: Severity::Warning,
            file: instructions.source_path.clone(),
            related: vec![agent.source_path.clone()],
            message: format!(
                "'{}' is named as coordinator but received {}/{} delegation(s) ({:.0}%)",
                agent.name,
                own,
                total,
                share * 100.0
            ),
            suggestion: Some(
                "an explicit orchestrator cannot be bypassed like prose can — \
                 --propose emits the observed root, with this one kept as a comment"
                    .to_string(),
            ),
        });
    }
    findings
}

/// U04 — session coverage of a declared skill, reported without judgement.
pub(super) fn u04_session_coverage(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    if usage.sessions == 0 {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for skill in &ctx.config.skills {
        let turns = usage.skills.get(&skill.name).copied().unwrap_or(0);
        if turns == 0 {
            continue; // U01's territory, not a coverage report.
        }
        findings.push(Finding {
            rule: "U04",
            severity: Severity::Info,
            file: skill.source_path.clone(),
            related: vec![],
            message: format!(
                "skill '{}' governed {} turn(s) across {} observed session(s)",
                skill.name, turns, usage.sessions
            ),
            suggestion: None,
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings, Severity};
    use crate::audit::usage::UsageFacts;
    use crate::audit::usage::facts::ROOT_AGENT;

    fn ctx<'a>(
        config: &'a crate::audit::reverse::ImportedConfig,
        settings: &'a AuditSettings,
        usage: &'a UsageFacts,
    ) -> AuditContext<'a> {
        AuditContext {
            config,
            settings,
            usage: Some(usage),
        }
    }

    #[test]
    fn u01_flags_a_declared_agent_that_never_ran() {
        let config = config_with(vec![agent("ghost", "prompt"), agent("qa", "prompt")]);
        let settings = AuditSettings::default();
        let mut usage = UsageFacts {
            sessions: 3,
            ..Default::default()
        };
        usage.record_delegation(ROOT_AGENT, "qa", "m");

        let f = u01_declared_never_used(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "only the unused one: {f:?}");
        assert!(f[0].message.contains("ghost"));
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn u01_is_silent_without_usage() {
        let config = config_with(vec![agent("ghost", "p")]);
        let settings = AuditSettings::default();
        let f = u01_declared_never_used(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert!(f.is_empty(), "no observation means no claim");
    }

    #[test]
    fn u01_is_silent_when_nothing_was_observed_at_all() {
        let config = config_with(vec![agent("ghost", "p")]);
        let settings = AuditSettings::default();
        let usage = UsageFacts::default();
        let f = u01_declared_never_used(&ctx(&config, &settings, &usage));
        assert!(
            f.is_empty(),
            "empty facts prove nothing about the declared assets"
        );
    }

    #[test]
    fn u02_flags_an_agent_used_but_not_declared() {
        let config = config_with(vec![agent("qa", "p")]);
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.record_delegation(ROOT_AGENT, "qa", "m");
        usage.record_delegation(ROOT_AGENT, "general-purpose", "m");

        let f = u02_used_but_undeclared(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("general-purpose"));
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].suggestion.is_some(),
            "the fix (materialise it as an agent) must be spelled out"
        );
    }

    #[test]
    fn u03_flags_a_bypassed_declared_coordinator() {
        let mut config = config_with(vec![agent("dev-lead", "p"), agent("qa", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "delegate to @dev-lead so that he can delegate".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..40 {
            usage.record_delegation(ROOT_AGENT, "qa", "m");
        }
        usage.record_delegation(ROOT_AGENT, "dev-lead", "m");

        let f = u03_coordinator_bypassed(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("dev-lead"));
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn u03_silent_when_the_declared_coordinator_leads() {
        let mut config = config_with(vec![agent("dev-lead", "p"), agent("qa", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "delegate to dev-lead".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..10 {
            usage.record_delegation(ROOT_AGENT, "dev-lead", "m");
        }
        usage.record_delegation(ROOT_AGENT, "qa", "m");

        assert!(u03_coordinator_bypassed(&ctx(&config, &settings, &usage)).is_empty());
    }

    #[test]
    fn u04_reports_session_coverage_of_a_declared_skill() {
        let mut config = config_with(vec![]);
        config.skills.push(crate::audit::reverse::ImportedSkill {
            name: "armadai".to_string(),
            source_path: std::path::PathBuf::from(".claude/skills/armadai/SKILL.md"),
            description: Some("project skill".to_string()),
            has_skill_md: true,
            has_frontmatter: true,
            issues: vec![],
            extra: Default::default(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts {
            sessions: 59,
            ..Default::default()
        };
        usage.record_skill_turn("armadai");

        let f = u04_session_coverage(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].message.contains("59"),
            "coverage must state the denominator: {}",
            f[0].message
        );
    }
}
