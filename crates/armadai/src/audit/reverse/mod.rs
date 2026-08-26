use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod claude;

/// Metadata recovered from a native agent file. Everything is optional:
/// a partial native config must never abort the audit (it IS the report).
#[derive(Debug, Clone, Default)]
pub struct PartialMetadata {
    pub description: Option<String>,
    pub model: Option<String>,
    /// `None` means the agent inherits all tools (no restriction declared).
    pub tools: Option<Vec<String>>,
    /// Frontmatter fields we do not type (kept verbatim for --propose and
    /// custom-field rules). Never populated by salvage.
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

impl PartialMetadata {
    /// Path claims from the non-standard `paths:` field (YAML list or CSV string).
    pub fn scope_globs(&self) -> Vec<String> {
        match self.extra.get("paths") {
            Some(serde_yaml_ng::Value::Sequence(seq)) => seq
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Some(serde_yaml_ng::Value::String(s)) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Something in a native file that could not be mapped.
#[derive(Debug, Clone)]
pub struct ParseIssue {
    pub file: PathBuf,
    pub message: String,
}

/// An agent imported from a native config.
#[derive(Debug, Clone)]
pub struct ImportedAgent {
    pub name: String,
    pub source_path: PathBuf,
    pub metadata: PartialMetadata,
    pub system_prompt: String,
    pub issues: Vec<ParseIssue>,
}

/// A skill imported from a native config (Agent Skills standard layout).
#[derive(Debug, Clone)]
pub struct ImportedSkill {
    pub name: String,
    pub source_path: PathBuf,
    pub description: Option<String>,
    pub has_skill_md: bool,
    pub has_frontmatter: bool,
    /// Estimated tokens of the whole `SKILL.md`, frontmatter included. `0`
    /// when the file is absent or unreadable.
    ///
    /// A count, not the body: `R01` only asks how big the file is, and
    /// loading a whole SKILL.md into the audit context to answer that would be
    /// the very defect the R family exists to measure.
    pub body_tokens: usize,
    pub issues: Vec<ParseIssue>,
    /// Frontmatter fields we do not type (kept verbatim for --propose and
    /// custom-field rules). Never populated by salvage.
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

/// Root instructions file (e.g. CLAUDE.md).
#[derive(Debug, Clone)]
pub struct ImportedInstructions {
    pub source_path: PathBuf,
    pub content: String,
}

/// Everything one ReverseLinker recovered from a repository.
#[derive(Debug, Clone, Default)]
pub struct ImportedConfig {
    pub agents: Vec<ImportedAgent>,
    pub skills: Vec<ImportedSkill>,
    pub instructions: Option<ImportedInstructions>,
}

/// Mirror of `crate::linker::Linker`, in the read direction.
pub trait ReverseLinker {
    fn name(&self) -> &'static str;
    /// Does this repository contain a surface this linker can read?
    fn detect(&self, root: &Path) -> bool;
    /// Parse everything readable. Never fails: unreadable pieces become
    /// `ParseIssue`s on the closest imported asset.
    fn parse(&self, root: &Path) -> ImportedConfig;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_config_default_is_empty() {
        let config = ImportedConfig::default();
        assert!(config.agents.is_empty());
        assert!(config.skills.is_empty());
        assert!(config.instructions.is_none());
    }

    #[test]
    fn scope_globs_reads_yaml_list_csv_string_or_absent() {
        use serde_yaml_ng::Value;

        let mut with_list = PartialMetadata::default();
        with_list.extra.insert(
            "paths".into(),
            Value::Sequence(vec![
                Value::String("src/cli/**".into()),
                Value::String("docs/".into()),
            ]),
        );
        assert_eq!(
            with_list.scope_globs(),
            vec!["src/cli/**".to_string(), "docs/".to_string()]
        );

        let mut with_csv = PartialMetadata::default();
        with_csv
            .extra
            .insert("paths".into(), Value::String("src/**, tests/".into()));
        assert_eq!(
            with_csv.scope_globs(),
            vec!["src/**".to_string(), "tests/".to_string()]
        );

        let absent = PartialMetadata::default();
        assert!(absent.scope_globs().is_empty());
    }
}
