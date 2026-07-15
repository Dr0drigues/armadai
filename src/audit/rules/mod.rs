use std::path::PathBuf;

use super::reverse::ImportedConfig;

mod assets;
mod collisions;
mod models;
pub(crate) mod references;
mod similarity;

pub(crate) use similarity::{DUPLICATION_WINDOW, duplication_clusters};

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
    /// Other files carried by an aggregated finding; `file` stays the anchor.
    pub related: Vec<PathBuf>,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Tunable thresholds (spec §8). Defaults are embedded; the optional
/// `audit:` section of armadai.yaml overrides them (Task 11).
#[derive(Debug, Clone)]
pub struct AuditSettings {
    /// A05: estimated token count above which a prompt is flagged.
    pub prompt_token_threshold: usize,
    /// C03: Jaccard similarity above which two activation descriptions are
    /// considered ambiguous for routing.
    pub activation_similarity: f64,
    /// Deep pass: max characters kept per prompt/instructions excerpt sent
    /// to the LLM auditor payload.
    pub deep_prompt_truncation: usize,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self {
            prompt_token_threshold: 4000,
            activation_similarity: 0.6,
            deep_prompt_truncation: 2000,
        }
    }
}

impl AuditSettings {
    /// Read the optional `audit:` section of the project config, if any.
    /// Missing file, missing section or unreadable YAML all yield defaults.
    pub fn from_project(root: &std::path::Path) -> Self {
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct AuditYaml {
            audit: Option<AuditSection>,
        }
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct AuditSection {
            prompt_token_threshold: Option<usize>,
            activation_similarity: Option<f64>,
            deep_prompt_truncation: Option<usize>,
        }
        let mut settings = Self::default();
        for candidate in ["armadai.yaml", ".armadai/config.yaml"] {
            let Ok(raw) = std::fs::read_to_string(root.join(candidate)) else {
                continue;
            };
            if let Ok(parsed) = serde_yaml_ng::from_str::<AuditYaml>(&raw)
                && let Some(section) = parsed.audit
            {
                if let Some(t) = section.prompt_token_threshold {
                    settings.prompt_token_threshold = t;
                }
                if let Some(s) = section.activation_similarity {
                    settings.activation_similarity = s;
                }
                if let Some(t) = section.deep_prompt_truncation {
                    settings.deep_prompt_truncation = t;
                }
            }
            break;
        }
        settings
    }
}

/// Minimal union-find over asset indices (no dependency needed).
pub(super) struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub(super) fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    pub(super) fn find(&mut self, i: usize) -> usize {
        if self.parent[i] != i {
            let root = self.find(self.parent[i]);
            self.parent[i] = root;
        }
        self.parent[i]
    }
    pub(super) fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
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
        assets::a05_oversized_prompt,
        similarity::a06_duplicated_blocks,
        similarity::a07_redundant_agents,
        assets::a08_permissive_tools,
        assets::a09_malformed_skill,
        references::a10_broken_references,
        references::a11_plaintext_secret,
        assets::a12_nonstandard_fields,
        collisions::c01_name_collisions,
        collisions::c02_scope_overlap,
        collisions::c03_activation_overlap,
        collisions::c04_double_ownership,
        collisions::c05_inconsistent_tools,
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
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::audit::reverse::*;

    pub fn agent(name: &str, prompt: &str) -> ImportedAgent {
        ImportedAgent {
            name: name.to_string(),
            source_path: PathBuf::from(format!(".claude/agents/{name}.md")),
            metadata: PartialMetadata {
                description: Some(format!("{name} description")),
                model: Some("claude-sonnet-5".to_string()),
                tools: Some(vec!["Read".to_string()]),
                extra: BTreeMap::new(),
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

    #[test]
    fn from_project_reads_audit_section() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("armadai.yaml"),
            "audit:\n  prompt_token_threshold: 1234\n  activation_similarity: 0.75\n  deep_prompt_truncation: 500\n",
        )
        .unwrap();
        let s = AuditSettings::from_project(dir.path());
        assert_eq!(s.prompt_token_threshold, 1234);
        assert!((s.activation_similarity - 0.75).abs() < f64::EPSILON);
        assert_eq!(s.deep_prompt_truncation, 500);
    }

    #[test]
    fn from_project_defaults_without_config() {
        let dir = tempfile::tempdir().unwrap();
        let s = AuditSettings::from_project(dir.path());
        assert_eq!(s.prompt_token_threshold, 4000);
        assert!((s.activation_similarity - 0.6).abs() < f64::EPSILON);
        assert_eq!(s.deep_prompt_truncation, 2000);
    }

    #[test]
    fn finding_carries_related_files() {
        let f = Finding {
            rule: "A06",
            severity: Severity::Warning,
            file: "a.md".into(),
            related: vec!["b.md".into(), "c.md".into()],
            message: String::new(),
            suggestion: None,
        };
        assert_eq!(f.related.len(), 2);
    }
}
