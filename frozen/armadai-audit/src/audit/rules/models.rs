use super::{AuditContext, Finding, Severity};

/// A03 — model is a known deprecated alias.
/// Deliberately fires on salvaged values too: a deprecated model is a
/// distinct root cause worth reporting even on a parse-broken file.
pub(super) fn a03_deprecated_model(ctx: &AuditContext) -> Vec<Finding> {
    a03_with_resolver(ctx, &armadai_core::model_aliases::resolve_alias)
}

pub(super) fn a03_with_resolver(
    ctx: &AuditContext,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Vec<Finding> {
    ctx.config
        .agents
        .iter()
        .filter_map(|a| {
            let model = a.metadata.model.as_deref()?;
            let replacement = resolve(model)?;
            Some(Finding {
                rule: "A03",
                severity: Severity::Critical,
                file: a.source_path.clone(),
                related: Vec::new(),
                message: format!("agent '{}' uses deprecated model '{model}'", a.name),
                suggestion: Some(format!("replace with '{replacement}'")),
            })
        })
        .collect()
}

/// A04 — model absent from the cached models.dev catalog.
/// Silent when the cache is missing/expired (offline-friendly, spec §5).
pub(super) fn a04_unknown_model(ctx: &AuditContext) -> Vec<Finding> {
    let catalog = armadai_providers::model_registry::fetch::load_all_providers_cached();
    let known = catalog.map(|providers| {
        let ids: std::collections::HashSet<String> =
            providers.into_values().flatten().map(|m| m.id).collect();
        move |model: &str| ids.contains(model)
    });
    a04_with_catalog(ctx, known.as_ref())
}

pub(super) fn a04_with_catalog<F: Fn(&str) -> bool>(
    ctx: &AuditContext,
    known: Option<&F>,
) -> Vec<Finding> {
    let Some(known) = known else {
        return Vec::new();
    };
    ctx.config
        .agents
        .iter()
        // Anti-cascade: salvaged fields on parse-broken agents are unreliable.
        .filter(|a| a.issues.is_empty())
        .filter_map(|a| {
            let model = a.metadata.model.as_deref()?;
            // Portable tiers and deprecated aliases are handled elsewhere.
            if model.starts_with("latest:")
                || armadai_core::model_aliases::resolve_alias(model).is_some()
                || known(model)
            {
                return None;
            }
            Some(Finding {
                rule: "A04",
                severity: Severity::Warning,
                file: a.source_path.clone(),
                related: Vec::new(),
                message: format!("agent '{}' uses unknown model '{model}'", a.name),
                suggestion: Some("check the spelling against `armadai models`".to_string()),
            })
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
    fn a03_flags_deprecated_model_with_replacement() {
        let mut a = agent("old", "Body");
        a.metadata.model = Some("gemini-3.0-pro".to_string());
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let ctx = AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        };
        let resolver = |m: &str| (m == "gemini-3.0-pro").then(|| "latest:pro".to_string());
        let f = a03_with_resolver(&ctx, &resolver);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A03");
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].suggestion.as_deref().unwrap().contains("latest:pro"));
    }

    #[test]
    fn a04_flags_model_absent_from_catalog() {
        let mut a = agent("typo", "Body");
        a.metadata.model = Some("claude-sonet-5".to_string()); // typo
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let ctx = AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        };
        let known = |m: &str| m == "claude-sonnet-5";
        let f = a04_with_catalog(&ctx, Some(&known));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A04");
    }

    #[test]
    fn a04_skips_parse_broken_agents() {
        let mut a = agent("broken", "Body");
        a.metadata.model = Some("whatever-unknown".to_string());
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
        let known = |_: &str| false;
        assert!(a04_with_catalog(&ctx, Some(&known)).is_empty());
    }

    #[test]
    fn a04_is_silent_without_cache() {
        let mut a = agent("any", "Body");
        a.metadata.model = Some("whatever".to_string());
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let ctx = AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        };
        assert!(a04_with_catalog(&ctx, None::<&fn(&str) -> bool>).is_empty());
    }
}
