use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use super::{AuditContext, Finding, Severity};

fn mention_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"@([a-z0-9][a-z0-9_-]{2,})")
            .unwrap_or_else(|e| unreachable!("hardcoded regex must compile: {e}"))
    })
}

/// Compiled plaintext-secret patterns used by A11. Also reused by the deep
/// pass (`audit::deep`) to redact secrets from prompt excerpts before they
/// are sent to an external LLM CLI.
pub(crate) fn secret_res() -> &'static [Regex] {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| {
        [
            r"\bsk-ant-[A-Za-z0-9_-]{20,}",
            r"\bsk-proj-[A-Za-z0-9_-]{20,}",
            r"\bsk-[A-Za-z0-9]{20,}",
            r"\bAIza[0-9A-Za-z_-]{35}",
            r"\bghp_[A-Za-z0-9]{36}",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// A10 — CLAUDE.md mentions an @agent that does not exist.
/// Only active when agents were imported (repos without subagents use
/// @handles for humans; flagging those would be noise).
pub(super) fn a10_broken_references(ctx: &AuditContext) -> Vec<Finding> {
    if ctx.config.agents.is_empty() {
        return Vec::new();
    }
    let Some(instructions) = &ctx.config.instructions else {
        return Vec::new();
    };
    let known: HashSet<&str> = ctx.config.agents.iter().map(|a| a.name.as_str()).collect();
    let mut seen = HashSet::new();
    mention_re()
        .captures_iter(&instructions.content)
        .filter_map(|c| c.get(1))
        .map(|m| m.as_str())
        .filter(|slug| !known.contains(slug) && seen.insert(slug.to_string()))
        .map(|slug| Finding {
            rule: "A10",
            severity: Severity::Warning,
            file: instructions.source_path.clone(),
            related: Vec::new(),
            message: format!("mentions '@{slug}' but no such agent exists"),
            suggestion: Some("create the agent or remove the stale mention".to_string()),
        })
        .collect()
}

/// A11 — plaintext API key patterns inside prompts or instructions.
/// The finding never echoes the matched secret.
pub(super) fn a11_plaintext_secret(ctx: &AuditContext) -> Vec<Finding> {
    let mut sources: Vec<(&std::path::Path, &str)> = ctx
        .config
        .agents
        .iter()
        .map(|a| (a.source_path.as_path(), a.system_prompt.as_str()))
        .collect();
    if let Some(instructions) = &ctx.config.instructions {
        sources.push((
            instructions.source_path.as_path(),
            instructions.content.as_str(),
        ));
    }
    sources
        .into_iter()
        .filter(|(_, text)| secret_res().iter().any(|re| re.is_match(text)))
        .map(|(path, _)| Finding {
            rule: "A11",
            severity: Severity::Critical,
            file: path.to_path_buf(),
            related: Vec::new(),
            message: "contains what looks like a plaintext API key".to_string(),
            suggestion: Some("move the secret to an env var or a secrets manager".to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::reverse::ImportedInstructions;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings, Severity};

    #[test]
    fn a10_flags_mention_of_unknown_agent() {
        let mut config = config_with(vec![agent("reviewer", "Body")]);
        config.instructions = Some(ImportedInstructions {
            source_path: "CLAUDE.md".into(),
            content: "Delegate to @reviewer and @ghost-agent.".into(),
        });
        let settings = AuditSettings::default();
        let f = a10_broken_references(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("ghost-agent"));
        assert_eq!(f[0].severity, Severity::Warning);
    }

    #[test]
    fn a10_is_silent_without_imported_agents() {
        let mut config = config_with(vec![]);
        config.instructions = Some(ImportedInstructions {
            source_path: "CLAUDE.md".into(),
            content: "Email @john for access.".into(),
        });
        let settings = AuditSettings::default();
        assert!(
            a10_broken_references(&AuditContext {
                config: &config,
                settings: &settings,
                usage: None,
            })
            .is_empty()
        );
    }

    #[test]
    fn a11_flags_api_key_patterns() {
        let key = format!("sk-ant-{}", "a".repeat(24));
        let a = agent("leaky", &format!("Use key {key} for calls."));
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let f = a11_plaintext_secret(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A11");
        // Never echo the secret back in the finding.
        assert!(!f[0].message.contains(&key));
    }

    #[test]
    fn a11_does_not_flag_lookalike_identifiers() {
        let a = agent(
            "clean",
            "Run task-abcdefghij0123456789 then check disk-Cache01234567890123456.",
        );
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let f = a11_plaintext_secret(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert!(f.is_empty());
    }

    #[test]
    fn a11_flags_openai_project_key() {
        let key = format!("sk-proj-{}", "b".repeat(24));
        let a = agent("leaky-openai", &format!("Use key {key} for calls."));
        let config = config_with(vec![a]);
        let settings = AuditSettings::default();
        let f = a11_plaintext_secret(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A11");
        assert!(!f[0].message.contains(&key));
    }
}
