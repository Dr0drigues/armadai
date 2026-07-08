use super::{AuditContext, Finding, Severity};

/// A01 — a native file could not be fully parsed.
pub(super) fn a01_unparsable(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .agents
        .iter()
        .flat_map(|a| a.issues.iter())
        .map(|i| Finding {
            rule: "A01",
            severity: Severity::Critical,
            file: i.file.clone(),
            message: i.message.clone(),
            suggestion: Some("fix the YAML frontmatter so tools can read this agent".to_string()),
        })
        .collect()
}

/// A02 — required descriptive fields are missing.
pub(super) fn a02_missing_fields(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .agents
        .iter()
        .filter(|a| a.metadata.description.is_none())
        .map(|a| Finding {
            rule: "A02",
            severity: Severity::Warning,
            file: a.source_path.clone(),
            message: format!("agent '{}' has no description", a.name),
            suggestion: Some(
                "add a `description:` field (used for routing and discovery)".to_string(),
            ),
        })
        .collect()
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
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A02");
        assert_eq!(f[0].severity, Severity::Warning);
    }
}
