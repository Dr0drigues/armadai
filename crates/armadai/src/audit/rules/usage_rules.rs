//! U0x — rules over observed usage. Every rule is silent when no usage was
//! observed: absence of measurement is never evidence of absence of use.

use std::path::PathBuf;

use super::{AuditContext, Finding, Severity};
use crate::audit::usage::UsageFacts;
use crate::audit::usage::facts::ROOT_AGENT;

/// Sub-agents Claude Code provides itself. They are legitimately used without
/// ever appearing in `.claude/agents/`, which is exactly why U02 reports them:
/// ArmadAI has no implicit equivalent, so a migration must materialise them.
/// `ROOT_AGENT` ("claude") is kept here defensively even though U02 already
/// skips it explicitly before this list is ever consulted — it is the native
/// CLI's own thread, not a declarable asset.
const BUILTIN_AGENTS: &[&str] = &["general-purpose", "Explore", "Plan", "claude"];

/// Share of delegations below which a declared coordinator counts as bypassed.
const COORDINATOR_SHARE: f64 = 0.5;

/// The fix for a declared-but-unused asset — identical whether it is an
/// agent or a skill.
const UNUSED_ASSET_SUGGESTION: &str =
    "remove it, or exclude it from the generated pack (--propose tags it `unused`)";

/// Words meaning delegation/coordination, matched case-insensitively (the
/// haystack is already lowercased) against the line carrying a coordinator
/// mention. English and French, since a project's own instructions may be
/// written in either.
const DELEGATION_WORDS: &[&str] = &["delegate", "coordinat", "délégu", "coordonn"];

fn observed<'a>(ctx: &AuditContext<'a>) -> Option<&'a UsageFacts> {
    ctx.usage.filter(|u| !u.is_empty())
}

/// U01 — a declared asset (agent or skill) that never ran over the observed
/// sessions.
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
            suggestion: Some(UNUSED_ASSET_SUGGESTION.to_string()),
        });
    }
    for skill in &ctx.config.skills {
        if usage.skills.contains_key(&skill.name) {
            continue;
        }
        findings.push(Finding {
            rule: "U01",
            severity: Severity::Warning,
            file: skill.source_path.clone(),
            related: vec![],
            message: format!(
                "skill '{}' is declared but was never used across {} observed session(s)",
                skill.name, usage.sessions
            ),
            suggestion: Some(UNUSED_ASSET_SUGGESTION.to_string()),
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
        if name == ROOT_AGENT {
            // The native CLI's own thread, not a declarable agent asset.
            continue;
        }
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
            // Not "declared nowhere": this check only sees `.claude/agents/`
            // on this project, so a plugin-provided agent (invisible on that
            // side by design, out of scope for this check) would also land
            // here — the message must not claim more than the code knows.
            message: format!(
                "sub-agent '{}' ran {} but is not declared in this project's \
                 `.claude/agents/`{}",
                name,
                // `invocations` counts main-thread delegations only. An agent
                // spawned solely from inside another sub-agent has none, so
                // reporting "0 time(s)" about one that demonstrably ran would
                // be false — fall back to the turns its transcript proves.
                if stats.invocations == 0 && stats.turns > 0 {
                    format!("{} turn(s) inside other agents", stats.turns)
                } else {
                    format!("{} time(s)", stats.invocations)
                },
                if builtin {
                    " (it is built into Claude Code)"
                } else {
                    " (if this isn't a typo, it may come from a plugin, which is out of scope \
                     for this check)"
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

/// True when the character right after a matched mention does not continue
/// the same identifier — guards `@qa` from matching inside `@qa-specialist`.
/// End of string counts as a boundary.
fn is_word_boundary(next: Option<char>) -> bool {
    match next {
        None => true,
        Some(c) => !c.is_alphanumeric() && c != '-' && c != '_',
    }
}

/// Does `haystack` (already lowercased) name `name` as a coordinator?
///
/// A mention only counts when both hold:
/// - it is word-bounded right after the match (so `@qa` cannot match inside
///   `@qa-specialist`);
/// - the same line also carries delegation language (so a passing "see also
///   @agent" reference does not count). The `delegate to {name}` phrase
///   already contains "delegate", so it satisfies this by construction — no
///   separate check is needed for it.
fn names_as_coordinator(haystack: &str, name: &str) -> bool {
    for pattern in [format!("@{name}"), format!("delegate to {name}")] {
        for (start, matched) in haystack.match_indices(pattern.as_str()) {
            let end = start + matched.len();
            if !is_word_boundary(haystack[end..].chars().next()) {
                continue;
            }
            let line_start = haystack[..start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = haystack[end..]
                .find('\n')
                .map_or(haystack.len(), |i| end + i);
            let line = &haystack[line_start..line_end];
            if DELEGATION_WORDS.iter().any(|word| line.contains(word)) {
                return true;
            }
        }
    }
    false
}

/// U03 — the root instructions name a coordinator that delegations bypass.
///
/// Ambiguity is resolved by silence, not by guessing: if more than one
/// declared agent qualifies as a named coordinator, the instructions do not
/// actually tell us who coordinates, and picking one anyway would accuse the
/// wrong agent of being bypassed. This rule names and accuses a specific
/// agent, so a false positive here is materially worse than a miss.
pub(super) fn u03_coordinator_bypassed(ctx: &AuditContext) -> Vec<Finding> {
    let Some(usage) = observed(ctx) else {
        return Vec::new();
    };
    let Some(instructions) = ctx.config.instructions.as_ref() else {
        return Vec::new();
    };
    // The spec's wording is "declared coordinator ≠ observed *root* of
    // delegations", so the denominator must be the root's own delegations,
    // not every delegation observed anywhere in the tree. Summing
    // `usage.agents` (as this used to) counts nested fan-out too: for
    // `root → dev-lead (1) → 10 specialists (10)`, that denominator is 11,
    // making dev-lead look bypassed at 9% when it is precisely the
    // coordinator fanning out. Restricting to `usage.edges[root]` fixes
    // that; it is a no-op on today's data, where every delegation is
    // attributed to the root, and correct once nested topologies appear.
    // This is still not exact — an edge only says "root delegated to X at
    // least once", not how many times, since `edges` is a `BTreeSet` with no
    // per-edge counter. Getting the exact count would need one; that's a
    // concern for a future lot, not this fix.
    let root = if usage.root_agent.is_empty() {
        ROOT_AGENT
    } else {
        usage.root_agent.as_str()
    };
    let total: u32 = usage
        .edges
        .get(root)
        .into_iter()
        .flatten()
        .filter_map(|child| usage.agents.get(child))
        .map(|a| a.invocations)
        .sum();
    if total == 0 {
        return Vec::new();
    }
    let haystack = instructions.content.to_lowercase();
    let mut candidates = ctx
        .config
        .agents
        .iter()
        .filter(|agent| names_as_coordinator(&haystack, &agent.name.to_lowercase()));
    let Some(agent) = candidates.next() else {
        return Vec::new();
    };
    if candidates.next().is_some() {
        return Vec::new();
    }
    let own = usage
        .agents
        .get(&agent.name)
        .map(|a| a.invocations)
        .unwrap_or(0);
    let share = f64::from(own) / f64::from(total);
    if share >= COORDINATOR_SHARE {
        return Vec::new();
    }
    vec![Finding {
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
    }]
}

/// U04 — activity of a declared skill (turns it governed across the scanned
/// sessions), reported without judgement.
///
/// This is *not* a coverage ratio: `UsageFacts` holds no per-session
/// breakdown of which sessions a skill was active in, only a total turn
/// count and a total session count, so a "how many sessions did this skill
/// touch out of how many" percentage is not something the aggregate can
/// compute. What is reported here — turns governed, and how many sessions
/// were scanned in total — is exactly what the data supports.
pub(super) fn u04_skill_activity(ctx: &AuditContext) -> Vec<Finding> {
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
            continue; // U01's territory, not an activity report.
        }
        findings.push(Finding {
            rule: "U04",
            severity: Severity::Info,
            file: skill.source_path.clone(),
            related: vec![],
            message: format!(
                "skill '{}' governed {} turn(s) across {} scanned session(s)",
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
    fn u01_flags_a_declared_skill_that_never_ran() {
        let mut config = config_with(vec![]);
        config.skills.push(crate::audit::reverse::ImportedSkill {
            name: "armadai".to_string(),
            source_path: std::path::PathBuf::from(".claude/skills/armadai/SKILL.md"),
            description: Some("project skill".to_string()),
            has_skill_md: true,
            has_frontmatter: true,
            body_tokens: 0,
            issues: vec![],
            extra: Default::default(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.record_delegation(ROOT_AGENT, "qa", "m");

        let f = u01_declared_never_used(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "only the unused skill: {f:?}");
        assert!(f[0].message.contains("skill"));
        assert!(f[0].message.contains("armadai"));
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
        assert!(
            f[0].message
                .contains("is not declared in this project's `.claude/agents/`"),
            "the message must not claim the agent is declared *nowhere* — it may still be \
             declared by a plugin, which this check cannot see: {}",
            f[0].message
        );
        assert!(
            f[0].message.contains("built into Claude Code"),
            "a Claude Code built-in must keep its own annotation: {}",
            f[0].message
        );
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].suggestion.is_some(),
            "the fix (materialise it as an agent) must be spelled out"
        );
    }

    /// A non-built-in undeclared agent (e.g. one provided by a plugin, like
    /// the real `claude-code-guide` case that motivated this wording) must
    /// get a hint that it may come from a plugin, not the built-in
    /// annotation and not an unqualified "declared nowhere" claim.
    #[test]
    fn u02_hints_at_a_plugin_for_a_non_builtin_undeclared_agent() {
        let config = config_with(vec![agent("qa", "p")]);
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.record_delegation(ROOT_AGENT, "qa", "m");
        usage.record_delegation(ROOT_AGENT, "claude-code-guide", "m");

        let f = u02_used_but_undeclared(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("claude-code-guide"));
        assert!(
            f[0].message.contains("plugin"),
            "a non-built-in agent must be hinted as possibly coming from a plugin: {}",
            f[0].message
        );
        assert!(
            !f[0].message.contains("built into Claude Code"),
            "the built-in annotation must not appear for a non-built-in agent: {}",
            f[0].message
        );
    }

    #[test]
    fn u02_skips_the_root_agent() {
        let config = config_with(vec![agent("qa", "p")]);
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.record_delegation(ROOT_AGENT, ROOT_AGENT, "m");
        usage.record_delegation(ROOT_AGENT, "general-purpose", "m");

        let f = u02_used_but_undeclared(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "the root thread must not be reported: {f:?}");
        assert!(f[0].message.contains("general-purpose"));
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
    fn u03_silent_on_a_bare_mention_without_delegation_language() {
        let mut config = config_with(vec![agent("qa-specialist", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "see also @qa-specialist for test conventions".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..5 {
            usage.record_delegation(ROOT_AGENT, "other", "m");
        }

        assert!(
            u03_coordinator_bypassed(&ctx(&config, &settings, &usage)).is_empty(),
            "a passing mention with no delegation language must not accuse anyone"
        );
    }

    #[test]
    fn u03_word_boundary_qa_does_not_match_qa_specialist() {
        let mut config = config_with(vec![agent("qa", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "coordinate via @qa-specialist for questions".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..3 {
            usage.record_delegation(ROOT_AGENT, "other", "m");
        }

        assert!(
            u03_coordinator_bypassed(&ctx(&config, &settings, &usage)).is_empty(),
            "'@qa' must not match inside '@qa-specialist'"
        );
    }

    /// Regression: the denominator must be the observed *root*'s own
    /// delegations, not the global delegation total. For
    /// `root → dev-lead (1) → 10 specialists (10)`, the old denominator (11)
    /// made dev-lead look bypassed at 9% even though it is precisely the
    /// coordinator fanning out — the fix restricts it to `edges[root]`,
    /// under which dev-lead is the root's only delegation (own=1, total=1).
    #[test]
    fn u03_silent_for_a_coordinator_whose_fanout_is_nested_under_it() {
        let mut config = config_with(vec![agent("dev-lead", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "delegate to @dev-lead so that he can delegate".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        usage.record_delegation(ROOT_AGENT, "dev-lead", "m");
        for i in 0..10 {
            usage.record_delegation("dev-lead", &format!("specialist-{i}"), "m");
        }

        assert!(
            u03_coordinator_bypassed(&ctx(&config, &settings, &usage)).is_empty(),
            "dev-lead received all of the root's own delegations (1/1); the specialists it \
             fanned out to must not inflate the denominator"
        );
    }

    #[test]
    fn u03_silent_when_multiple_agents_qualify_as_coordinator() {
        let mut config = config_with(vec![agent("alpha", "p"), agent("beta", "p")]);
        config.instructions = Some(crate::audit::reverse::ImportedInstructions {
            source_path: std::path::PathBuf::from("CLAUDE.md"),
            content: "delegate to @alpha or delegate to @beta as needed".to_string(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts::default();
        for _ in 0..90 {
            usage.record_delegation(ROOT_AGENT, "other", "m");
        }
        for _ in 0..5 {
            usage.record_delegation(ROOT_AGENT, "alpha", "m");
        }
        for _ in 0..5 {
            usage.record_delegation(ROOT_AGENT, "beta", "m");
        }

        assert!(
            u03_coordinator_bypassed(&ctx(&config, &settings, &usage)).is_empty(),
            "two qualifying candidates must yield silence, not a guess"
        );
    }

    #[test]
    fn u04_reports_activity_of_a_declared_skill_across_scanned_sessions() {
        let mut config = config_with(vec![]);
        config.skills.push(crate::audit::reverse::ImportedSkill {
            name: "armadai".to_string(),
            source_path: std::path::PathBuf::from(".claude/skills/armadai/SKILL.md"),
            description: Some("project skill".to_string()),
            has_skill_md: true,
            has_frontmatter: true,
            body_tokens: 0,
            issues: vec![],
            extra: Default::default(),
        });
        let settings = AuditSettings::default();
        let mut usage = UsageFacts {
            sessions: 59,
            ..Default::default()
        };
        usage.record_skill_turn("armadai");

        let f = u04_skill_activity(&ctx(&config, &settings, &usage));
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, Severity::Info);
        assert!(
            f[0].message.contains("59"),
            "the message must state how many sessions were scanned: {}",
            f[0].message
        );
    }
}
