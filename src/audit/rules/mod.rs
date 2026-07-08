use std::path::PathBuf;

use super::reverse::ImportedConfig;

mod assets;
mod models;

/// Finding severity. Ordering: Critical < Warning < Info (sort shows critical first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => "CRIT",
            Severity::Warning => "WARN",
            Severity::Info => "INFO",
        }
    }
}

/// One audit finding. `suggestion` is a concrete, human-applicable fix.
#[derive(Debug, Clone)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub file: PathBuf,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Tunable thresholds (spec §8). Defaults are embedded; the optional
/// `audit:` section of armadai.yaml overrides them (Task 11).
#[derive(Debug, Clone)]
pub struct AuditSettings {
    /// A05: estimated token count above which a prompt is flagged.
    pub prompt_token_threshold: usize,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self {
            prompt_token_threshold: 4000,
        }
    }
}

pub struct AuditContext<'a> {
    pub config: &'a ImportedConfig,
    pub settings: &'a AuditSettings,
}

type RuleFn = fn(&AuditContext) -> Vec<Finding>;

/// Static rule registry: adding a rule = one module + one entry here.
fn registry() -> Vec<RuleFn> {
    vec![
        assets::a01_unparsable,
        assets::a02_missing_fields,
        models::a03_deprecated_model,
        models::a04_unknown_model,
    ]
}

/// Run every registered rule and return findings sorted by severity then file.
pub fn run_rules(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings: Vec<Finding> = registry().iter().flat_map(|rule| rule(ctx)).collect();
    findings.sort_by(|a, b| (a.severity, &a.file, a.rule).cmp(&(b.severity, &b.file, b.rule)));
    findings
}

/// Rough token estimate (chars / 4) — good enough for thresholds and savings.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    use crate::audit::reverse::*;
    use crate::linker::LinkTarget;

    pub fn agent(name: &str, prompt: &str) -> ImportedAgent {
        ImportedAgent {
            name: name.to_string(),
            source_path: PathBuf::from(format!(".claude/agents/{name}.md")),
            source_format: LinkTarget::Claude,
            metadata: PartialMetadata {
                description: Some(format!("{name} description")),
                model: Some("claude-sonnet-5".to_string()),
                tools: Some(vec!["Read".to_string()]),
            },
            system_prompt: prompt.to_string(),
            issues: Vec::new(),
        }
    }

    pub fn config_with(agents: Vec<ImportedAgent>) -> ImportedConfig {
        ImportedConfig {
            agents,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_critical_first() {
        assert!(Severity::Critical < Severity::Warning);
        assert!(Severity::Warning < Severity::Info);
    }

    #[test]
    fn estimate_tokens_is_chars_over_four() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn run_rules_on_empty_config_is_empty() {
        let config = crate::audit::reverse::ImportedConfig::default();
        let settings = AuditSettings::default();
        let ctx = AuditContext {
            config: &config,
            settings: &settings,
        };
        assert!(run_rules(&ctx).is_empty());
    }
}
