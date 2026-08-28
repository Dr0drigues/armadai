use std::path::PathBuf;

use super::AuditScope;
use super::reverse::ImportedConfig;

mod assets;
mod collisions;
mod models;
pub(crate) mod references;
mod rightsizing;
mod similarity;
mod usage_rules;

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
    /// R01: estimated token count above which a skill with no `references/`
    /// is flagged.
    ///
    /// 4000 is a **context budget**, not a quantile: it is the point past
    /// which "the whole body enters context the moment this skill triggers"
    /// stops being a detail and becomes a cost worth naming. That claim holds
    /// whatever anyone else's skills look like.
    ///
    /// It was originally justified as a p90, and that justification was
    /// wrong twice over. The corpus it was measured on — 461 `SKILL.md`
    /// files — was 88% `~/.config/armadai/registry`, a synced catalogue of
    /// other people's assets that no scope audits. On the corpus that *is*
    /// auditable (the 48 skills installed on the same machine) the p90 is
    /// 2456, and 48 samples are far too thin a base to derive a threshold
    /// from anyway. Moving 4000 down to that p90 changes nothing observable:
    /// measured over those 48 skills, 4000 flags 1 (2%) and so does 2456 —
    /// only 4 skills exceed 2456 and 3 of them already have `references/`.
    /// The threshold is not what makes `R01` narrow; the `references/`
    /// condition is.
    pub skill_token_threshold: usize,
    /// C03: Jaccard similarity above which two activation descriptions are
    /// considered ambiguous for routing.
    pub activation_similarity: f64,
    /// Deep pass: max characters kept per prompt/instructions excerpt sent
    /// to the LLM auditor payload.
    pub deep_prompt_truncation: usize,
    /// Whether to scan this project's Claude Code transcripts for observed
    /// usage (the "Observed usage" section plus rules U01-U04). `true` by
    /// default, so existing configs are unaffected. `--no-usage` on the CLI
    /// always wins over this when both are set — see `cli::audit::execute`.
    pub usage: bool,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self {
            prompt_token_threshold: 4000,
            skill_token_threshold: 4000,
            activation_similarity: 0.6,
            deep_prompt_truncation: 2000,
            usage: true,
        }
    }
}

impl AuditSettings {
    /// Read the optional `audit:` section of the project config, if any.
    /// Missing file, missing section or unreadable YAML all yield defaults.
    ///
    /// Project scope only. The global library has its own source — see
    /// [`AuditSettings::from_global`] — because thresholds must follow the
    /// surface being audited, not the folder the command was typed in.
    pub fn from_project(root: &std::path::Path) -> Self {
        let mut settings = Self::default();
        for candidate in ["armadai.yaml", ".armadai/config.yaml"] {
            // The first of the two that *exists* wins, whether or not it
            // carries an `audit:` section — the second is a fallback for a
            // missing file, not for a missing key.
            if settings.apply_audit_section(&root.join(candidate)) {
                break;
            }
        }
        settings
    }

    /// Read the optional `audit:` section of the *user-level* config,
    /// `~/.config/armadai/config.yaml` (or wherever `$ARMADAI_CONFIG_DIR` /
    /// `$XDG_CONFIG_HOME` puts it).
    ///
    /// A global audit reads one fixed set of assets, so it must reach one
    /// fixed verdict. Sourcing its thresholds from `<cwd>/armadai.yaml` made
    /// that false: measured on one machine, the same global library produced
    /// 2 `R01` warnings from a directory carrying `skill_token_threshold: 5`
    /// and 0 from a neutral one. The library's settings belong next to the
    /// library.
    pub fn from_global() -> Self {
        let mut settings = Self::default();
        settings.apply_audit_section(&armadai_core::config::config_dir().join("config.yaml"));
        settings
    }

    /// Overlay the `audit:` section of one YAML file onto `self`. Returns
    /// whether the file was *readable* — the caller's "first candidate that
    /// exists wins" is about the file, not about the section, so a config with
    /// no `audit:` key still stops the search.
    ///
    /// Unparsable YAML leaves the defaults in place: the audit never refuses
    /// to run over its own configuration.
    fn apply_audit_section(&mut self, path: &std::path::Path) -> bool {
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct AuditYaml {
            audit: Option<AuditSection>,
        }
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct AuditSection {
            prompt_token_threshold: Option<usize>,
            skill_token_threshold: Option<usize>,
            activation_similarity: Option<f64>,
            deep_prompt_truncation: Option<usize>,
            usage: Option<bool>,
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            return false;
        };
        if let Ok(parsed) = serde_yaml_ng::from_str::<AuditYaml>(&raw)
            && let Some(section) = parsed.audit
        {
            if let Some(t) = section.prompt_token_threshold {
                self.prompt_token_threshold = t;
            }
            if let Some(t) = section.skill_token_threshold {
                self.skill_token_threshold = t;
            }
            if let Some(s) = section.activation_similarity {
                self.activation_similarity = s;
            }
            if let Some(t) = section.deep_prompt_truncation {
                self.deep_prompt_truncation = t;
            }
            if let Some(u) = section.usage {
                self.usage = u;
            }
        }
        true
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
    /// Observed usage, when transcripts were found. `None` means the static
    /// rules run alone — usage never becomes a prerequisite.
    pub usage: Option<&'a crate::audit::usage::UsageFacts>,
}

type RuleFn = fn(&AuditContext) -> Vec<Finding>;

/// Static rule registry: adding a rule = one module + one entry here.
///
/// Every family applies to both scopes — a rule reads the assets themselves,
/// and a property of an asset holds wherever the asset lives. `usage_rules`
/// is the single exception, and it is excluded *here* rather than inside the
/// rules: `U01`-`U04` correlate declarations against one project's Claude
/// Code transcripts, and a rule that only ever sees `ctx.config` cannot tell
/// which scope filled it. So the default is "applies", the exception is
/// registered once, and no rule branches on scope.
fn registry(scope: AuditScope) -> Vec<RuleFn> {
    let mut rules: Vec<RuleFn> = vec![
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
        rightsizing::r01_oversized_skill,
        rightsizing::r02_stale_path,
        rightsizing::r04_context_weight,
        collisions::c01_name_collisions,
        collisions::c02_scope_overlap,
        collisions::c03_activation_overlap,
        collisions::c04_double_ownership,
        collisions::c05_inconsistent_tools,
    ];
    if scope == AuditScope::Project {
        rules.extend::<[RuleFn; 4]>([
            usage_rules::u01_declared_never_used,
            usage_rules::u02_used_but_undeclared,
            usage_rules::u03_coordinator_bypassed,
            usage_rules::u04_skill_activity,
        ]);
    }
    rules
}

/// Run every rule registered for `scope` and return findings sorted by
/// severity then file.
pub fn run_rules(ctx: &AuditContext, scope: AuditScope) -> Vec<Finding> {
    let mut findings: Vec<Finding> = registry(scope).iter().flat_map(|rule| rule(ctx)).collect();
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
            format: AgentFormat::ClaudeFrontmatter,
            space: PathBuf::from(".claude"),
        }
    }

    /// The same shape read from an ArmadAI-format file: a description derived
    /// from the prompt (the format has no `description` field), and no tool
    /// list, because the format cannot express one.
    pub fn armadai_agent(name: &str, prompt: &str) -> ImportedAgent {
        ImportedAgent {
            name: name.to_string(),
            source_path: PathBuf::from(format!(".config/armadai/agents/{name}.md")),
            metadata: PartialMetadata {
                description: prompt.lines().next().map(str::to_string),
                model: Some("latest:pro".to_string()),
                tools: None,
                extra: BTreeMap::new(),
            },
            system_prompt: prompt.to_string(),
            issues: Vec::new(),
            format: AgentFormat::Armadai,
            space: PathBuf::from(".config/armadai"),
        }
    }

    /// A well-formed skill of a chosen size. `body_tokens` is the only knob
    /// the R rules read; everything else is the "nothing else is wrong" shape
    /// so a size finding cannot be confused with a structural one.
    pub fn skill(name: &str, body_tokens: usize) -> ImportedSkill {
        ImportedSkill {
            name: name.to_string(),
            source_path: PathBuf::from(format!(".claude/skills/{name}/SKILL.md")),
            description: Some(format!("{name} description")),
            has_skill_md: true,
            has_frontmatter: true,
            body_tokens,
            issues: Vec::new(),
            extra: BTreeMap::new(),
            space: PathBuf::from(".claude"),
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
            usage: None,
        };
        assert!(run_rules(&ctx, AuditScope::Project).is_empty());
        assert!(run_rules(&ctx, AuditScope::Global).is_empty());
    }

    #[test]
    fn from_project_reads_audit_section() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("armadai.yaml"),
            "audit:\n  prompt_token_threshold: 1234\n  skill_token_threshold: 1500\n  activation_similarity: 0.75\n  deep_prompt_truncation: 500\n  usage: false\n",
        )
        .unwrap();
        let s = AuditSettings::from_project(dir.path());
        assert_eq!(s.prompt_token_threshold, 1234);
        assert_eq!(s.skill_token_threshold, 1500);
        assert!((s.activation_similarity - 0.75).abs() < f64::EPSILON);
        assert_eq!(s.deep_prompt_truncation, 500);
        assert!(!s.usage, "usage: false in config must be honoured");
    }

    /// `.armadai/config.yaml` is the documented second candidate, and the
    /// refactor that split this loop out into `apply_audit_section` could have
    /// dropped it in silence — nothing covered it before.
    #[test]
    fn from_project_falls_back_to_the_dot_armadai_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/config.yaml"),
            "audit:\n  skill_token_threshold: 777\n",
        )
        .unwrap();
        let s = AuditSettings::from_project(dir.path());
        assert_eq!(s.skill_token_threshold, 777);
    }

    /// The first candidate that *exists* wins — even when it carries no
    /// `audit:` section at all. The second is a fallback for a missing file,
    /// not for a missing key, so a project that deliberately empties its
    /// `armadai.yaml` gets the defaults rather than a stale `.armadai/` value.
    #[test]
    fn the_first_existing_candidate_wins_even_with_no_audit_section() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("armadai.yaml"), "name: demo\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/config.yaml"),
            "audit:\n  skill_token_threshold: 777\n",
        )
        .unwrap();
        let s = AuditSettings::from_project(dir.path());
        assert_eq!(
            s.skill_token_threshold, 4000,
            "armadai.yaml exists and says nothing about audit: that is the answer"
        );
    }

    #[test]
    fn from_project_defaults_without_config() {
        let dir = tempfile::tempdir().unwrap();
        let s = AuditSettings::from_project(dir.path());
        assert_eq!(s.prompt_token_threshold, 4000);
        assert_eq!(
            s.skill_token_threshold, 4000,
            "R01's default is a context budget — the point past which \"the whole \
             body enters context when this skill triggers\" becomes a cost worth \
             naming — not a quantile of whatever happens to sit on one machine. \
             Changing it must be a deliberate act"
        );
        assert!((s.activation_similarity - 0.6).abs() < f64::EPSILON);
        assert_eq!(s.deep_prompt_truncation, 2000);
        assert!(
            s.usage,
            "usage must default to true so existing configs are unaffected"
        );
    }

    /// The one rule exclusion of the whole scope design, asserted where it is
    /// decided: the registry, not a rule body.
    ///
    /// The usage facts here are deliberately non-empty and the context
    /// deliberately carries them, because `U01`-`U04` are *also* silent when
    /// `ctx.usage` is `None` — which is what the global scope passes in
    /// practice. Testing the exclusion through that `None` would prove
    /// nothing about the registry: removing the `scope` guard entirely would
    /// leave such a test green. So this one hands the rules everything they
    /// need to fire and asserts the registry still keeps them out.
    #[test]
    fn usage_rules_are_registered_for_a_project_and_never_for_the_global_library() {
        let mut config = crate::audit::reverse::ImportedConfig::default();
        config
            .agents
            .push(test_support::agent("ghost", "never invoked"));
        let mut usage = crate::audit::usage::UsageFacts {
            sessions: 1,
            ..Default::default()
        };
        usage.record_delegation(
            crate::audit::usage::facts::ROOT_AGENT,
            "general-purpose",
            "claude-opus-5",
        );
        assert!(!usage.is_empty(), "the fixture must be able to fire U0x");
        let settings = AuditSettings::default();
        let ctx = AuditContext {
            config: &config,
            settings: &settings,
            usage: Some(&usage),
        };

        let project: Vec<&str> = run_rules(&ctx, AuditScope::Project)
            .iter()
            .map(|f| f.rule)
            .filter(|r| r.starts_with('U'))
            .collect();
        assert_eq!(
            project,
            vec!["U01", "U02"],
            "project scope must run the usage rules: a declared-but-unused \
             agent (U01) and an undeclared one that ran (U02)"
        );

        let global: Vec<&str> = run_rules(&ctx, AuditScope::Global)
            .iter()
            .map(|f| f.rule)
            .filter(|r| r.starts_with('U'))
            .collect();
        assert!(
            global.is_empty(),
            "usage rules correlate one project's transcripts and must not be \
             registered for the global library, got: {global:?}"
        );
    }

    /// The mirror of the above: every *other* family must be registered for
    /// both scopes. Excluding one family is a whitelist of one, and a
    /// regression that quietly widened it would otherwise be invisible.
    #[test]
    fn every_other_family_is_registered_for_both_scopes() {
        assert_eq!(
            registry(AuditScope::Project).len(),
            registry(AuditScope::Global).len() + 4,
            "the two registries must differ by exactly the four usage rules"
        );
        assert_eq!(
            registry(AuditScope::Global).len(),
            20,
            "global drops U01-U04 and keeps every other rule"
        );
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
