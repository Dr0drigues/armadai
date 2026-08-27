//! Audit of native agentic configurations (adoption funnel).
//!
//! Reads native CLI configs (Claude Code first) through `ReverseLinker`s,
//! runs static rules over the imported assets and produces an `AuditReport`.
pub mod deep;
pub mod proposal;
pub mod report;
pub mod reverse;
pub mod rules;
pub mod scope;
pub mod usage;

use std::path::Path;

use report::AuditReport;
use reverse::ReverseLinker;
pub use scope::{AuditScope, GlobalLayout, import_global_surfaces};

/// Detect and parse every native surface under `root`.
/// Shared by the audit run and `--propose` (which needs the raw imports).
pub fn import_surfaces(root: &Path) -> (Vec<String>, reverse::ImportedConfig) {
    let linkers: Vec<Box<dyn ReverseLinker>> = vec![Box::new(reverse::claude::ClaudeReverseLinker)];
    let mut detected = Vec::new();
    let mut config = reverse::ImportedConfig::default();
    for linker in &linkers {
        if linker.detect(root) {
            detected.push(linker.name().to_string());
            let parsed = linker.parse(root);
            config.agents.extend(parsed.agents);
            config.skills.extend(parsed.skills);
            if config.instructions.is_none() {
                config.instructions = parsed.instructions;
            }
        }
    }
    (detected, config)
}

/// Everything one audit run reads, before any rule sees it.
///
/// The two scopes differ only in how this is filled — one repository root, or
/// the user's own library — so keeping the assembly separate from the analysis
/// is what lets both share every rule, every renderer and every exit path.
///
/// Every field is private, and the two factories below are the only way to
/// build one. `root` and `scope` are not independent: `Global` means "paths are
/// shown relative to `$HOME`", and a literal `AuditInput { root: <a project>,
/// scope: Global, .. }` compiled happily while producing a report titled
/// `armadai audit (global)` anchored on a repository. Nothing in the type
/// stopped it; now the constructor is the only door and it does.
#[derive(Debug, Clone)]
pub struct AuditInput {
    /// What findings paths are displayed relative to: the repository root, or
    /// `$HOME` in global scope.
    root: std::path::PathBuf,
    scope: AuditScope,
    detected: Vec<String>,
    skipped: Vec<String>,
    config: reverse::ImportedConfig,
}

impl AuditInput {
    /// Which surface this input covers.
    pub fn scope(&self) -> AuditScope {
        self.scope
    }

    /// Labels of the roots that held at least one readable surface. Empty
    /// means "nothing to audit", which the CLI reports as such.
    pub fn detected(&self) -> &[String] {
        &self.detected
    }

    /// The imported surfaces, shared with `--propose` and `--deep` so neither
    /// re-reads the same files.
    pub fn config(&self) -> &reverse::ImportedConfig {
        &self.config
    }

    /// Project scope: the native surfaces under one repository root.
    pub fn for_project(root: &Path) -> Self {
        let (detected, config) = import_surfaces(root);
        Self {
            root: root.to_path_buf(),
            scope: AuditScope::Project,
            detected,
            skipped: Vec::new(),
            config,
        }
    }

    /// Global scope: the user's own library, assembled from known locations.
    pub fn for_global(layout: &GlobalLayout) -> Self {
        let imported = import_global_surfaces(layout);
        Self {
            root: layout.home.clone(),
            scope: AuditScope::Global,
            detected: imported.detected,
            skipped: imported.skipped,
            config: imported.config,
        }
    }

    /// Run the rules registered for this input's scope.
    ///
    /// `usage` is only ever `Some` in project scope: `U01`-`U04` correlate
    /// declarations against *one project's* Claude Code transcripts, which is
    /// why they are the single family the global registry leaves out. Every
    /// other family reads the assets themselves and holds wherever they live.
    pub fn analyse(
        &self,
        settings: &rules::AuditSettings,
        usage: Option<&usage::UsageFacts>,
    ) -> AuditReport {
        let ctx = rules::AuditContext {
            config: &self.config,
            settings,
            usage,
        };
        AuditReport {
            root: self.root.clone(),
            scope: self.scope,
            detected: self.detected.clone(),
            agent_count: self.config.agents.len(),
            skill_count: self.config.skills.len(),
            findings: rules::run_rules(&ctx, self.scope),
            deep_raw: None,
            usage: usage.cloned(),
            skipped: self.skipped.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_surfaces_returns_detected_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("a.md"),
            "---\nname: a\ndescription: d\n---\nBody",
        )
        .unwrap();
        let (detected, config) = import_surfaces(dir.path());
        assert_eq!(detected, vec!["claude".to_string()]);
        assert_eq!(config.agents.len(), 1);
    }

    #[test]
    fn analyse_accepts_observed_usage() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("a.md"),
            "---\nname: a\ndescription: d\n---\nBody",
        )
        .unwrap();
        let mut usage = usage::UsageFacts::default();
        usage.record_delegation(usage::facts::ROOT_AGENT, "a", "claude-opus-5");

        let report = AuditInput::for_project(dir.path())
            .analyse(&rules::AuditSettings::default(), Some(&usage));
        assert_eq!(report.agent_count, 1);
        assert_eq!(report.scope, AuditScope::Project);
    }
}
