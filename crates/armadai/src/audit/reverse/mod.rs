use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub mod armadai;
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

/// Which on-disk format an agent was read from.
///
/// Not decoration: it is what lets a rule ask "could this file have declared
/// the thing I am about to report as missing?". `A08` reports agents that
/// inherit every tool, and an ArmadAI-format file has no syntax for a tool
/// list at all — so an ArmadAI library would score 100% "permissive" on a
/// property it cannot express, and mixing one native agent into it would flip
/// that Info into a fleet-wide Warning. That is a finding produced by the
/// reader, not by the fleet.
///
/// Scope is deliberately *not* available to rules (see [`AuditScope`]); format
/// is, and the two are different questions: scope is where a file was found,
/// format is what the file can say.
///
/// [`AuditScope`]: crate::audit::AuditScope
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFormat {
    /// Native Claude Code: YAML frontmatter (`name`, `description`, `tools`…).
    ClaudeFrontmatter,
    /// ArmadAI: `# H1` + `## Metadata` + `## System Prompt`.
    Armadai,
}

impl AgentFormat {
    /// Can a file in this format restrict which tools the agent may use?
    ///
    /// `AgentMetadata` carries no tool list, and neither does the
    /// `## Metadata` grammar `parser::metadata` accepts, so the answer for
    /// ArmadAI is no — and a rule about tool restrictions has nothing to say
    /// about such a file.
    pub fn declares_tools(self) -> bool {
        matches!(self, Self::ClaudeFrontmatter)
    }
}

/// The tree an asset was read from: the repository root in project scope,
/// `~/.claude` or `~/.config/armadai` in the global one.
///
/// Not decoration either, and not [`AuditScope`] in disguise: it is what lets
/// a rule ask **"are these two files in the same resolution space?"** — the
/// question every rule that compares two assets is really asking.
///
/// `C01` reports two files claiming one name as ambiguous routing, and `A06`
/// / `A07` report two agents as redundant. All three are only true of assets
/// something resolves *together*. The global pass assembles two unrelated
/// trees, and `armadai link` publishes the ArmadAI library into the native one
/// by design, so a healthy library seen through both roots produced
/// `2 critical` and a non-zero exit — one per agent the user had linked
/// (measured, issue #399). A name is ambiguous inside one tree; the same name
/// in two trees is one asset and its published copy.
///
/// Scope stays unavailable to rules: it says which surface the *run* reads, so
/// branching on it would make one rule behave two ways. A space is a property
/// of the *file*, like a format, and every rule treats every file the same way
/// whatever the scope filled it.
///
/// [`AuditScope`]: crate::audit::AuditScope
pub type ResolutionSpace = PathBuf;

/// An agent imported from a native config.
#[derive(Debug, Clone)]
pub struct ImportedAgent {
    pub name: String,
    pub source_path: PathBuf,
    pub metadata: PartialMetadata,
    pub system_prompt: String,
    pub issues: Vec<ParseIssue>,
    /// What the file it came from is able to declare — see [`AgentFormat`].
    pub format: AgentFormat,
    /// The tree this file was read from — see [`ResolutionSpace`].
    pub space: ResolutionSpace,
    /// The typed `## Metadata` block an ArmadAI-format source carried, kept
    /// verbatim for `--propose`. `None` for a native file.
    ///
    /// [`PartialMetadata`] is the *shared* view every rule reads, and it is
    /// deliberately the intersection of what both formats express: a
    /// description, a model, a tool list. An ArmadAI file says more —
    /// `temperature`, `max_tokens`, `tags`, `stacks`, `scope` — and `--propose`
    /// is the one consumer that must not lose it, because since #393 it can
    /// run on a library that is *already* ArmadAI and its output is offered as
    /// an installable replacement for it (issue #400).
    ///
    /// Kept out of [`PartialMetadata::extra`] on purpose, and the reason is
    /// measured: `extra` is Claude Code frontmatter the audit does not type, so
    /// `A12` reports its keys as non-standard and `C02`/`C05` read `paths` out
    /// of it. Routing ArmadAI's own vocabulary through it announced
    /// `provider (76), model (76), tags (76)` on a healthy library and turned
    /// `scope` into 149 phantom overlapping pairs. A separate field is what
    /// lets the proposal see the fields while the rules keep not seeing them.
    pub armadai_metadata: Option<armadai_core::agent::AgentMetadata>,
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
    /// The tree this skill was read from — see [`ResolutionSpace`].
    pub space: ResolutionSpace,
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
