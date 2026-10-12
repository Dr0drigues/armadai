mod codex;
mod copilot;
pub mod manifest;
mod opencode;

pub use codex::CodexLinker;
pub use copilot::CopilotLinker;
pub use opencode::OpencodeLinker;

use std::path::PathBuf;

/// Supported link targets for autocompletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LinkTarget {
    Claude,
    Codex,
    Copilot,
    Opencode,
}

impl LinkTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Opencode => "opencode",
        }
    }
}

impl std::fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An agent as the Codex, Copilot and OpenCode linkers take it
/// (built from the canonical model by `compile::legacy_targets`).
pub struct LinkAgent {
    pub name: String,
    pub system_prompt: String,
    pub instructions: Option<String>,
    pub output_format: Option<String>,
    pub context: Option<String>,
    pub description: Option<String>,
    pub model: Option<String>,
    pub temperature: f32,
}

/// A file to be written by a linker.
pub struct OutputFile {
    pub path: PathBuf,
    pub content: String,
}

/// Trait for generating target-specific config files.
pub trait Linker: Send + Sync {
    fn default_output_dir(&self) -> &str;
    fn generate(&self, agents: &[LinkAgent], coordinator: Option<&LinkAgent>) -> Vec<OutputFile>;
}

/// Every link target, for error messages.
///
/// Derived from `LinkTarget` rather than typed out: a target added to the enum
/// reaches every message at once (issue #369).
pub fn supported_target_names() -> Vec<&'static str> {
    use clap::ValueEnum;
    LinkTarget::value_variants()
        .iter()
        .map(|t| t.as_str())
        .collect()
}

/// The sentence every "no/unknown target" message ends with.
pub fn supported_targets_sentence() -> String {
    format!("Supported targets: {}", supported_target_names().join(", "))
}

/// The legacy linker of a target other than Claude (whose adapter is
/// `compile::claude`).
pub fn create_linker(target: &str) -> anyhow::Result<Box<dyn Linker>> {
    match target {
        "codex" => Ok(Box::new(CodexLinker)),
        "copilot" => Ok(Box::new(CopilotLinker)),
        "opencode" => Ok(Box::new(OpencodeLinker)),
        _ => anyhow::bail!(
            "No legacy linker for '{target}'. {}",
            supported_targets_sentence()
        ),
    }
}

/// Convert an agent name to a kebab-case slug suitable for filenames.
pub use armadai_core::agent::slugify;

impl LinkAgent {
    /// This agent's sections composed into one prompt body, through the same
    /// core definition the legacy path used (`compose_agent_prompt`).
    pub fn composed_prompt(&self) -> String {
        armadai_core::agent::compose_agent_prompt(
            &self.system_prompt,
            self.instructions.as_deref(),
            self.output_format.as_deref(),
            self.context.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `armadai_model::TARGETS` is the one list the new format validates against;
    /// it must be exactly what `link` supports. The exhaustive match breaks the
    /// build if a `LinkTarget` variant is added without revisiting this test.
    #[test]
    fn model_targets_match_the_link_targets() {
        let all = [
            LinkTarget::Claude,
            LinkTarget::Codex,
            LinkTarget::Copilot,
            LinkTarget::Opencode,
        ];
        for t in all {
            match t {
                LinkTarget::Claude
                | LinkTarget::Codex
                | LinkTarget::Copilot
                | LinkTarget::Opencode => {}
            }
        }
        let names: Vec<&str> = all.iter().map(|t| t.as_str()).collect();
        assert_eq!(names, armadai_model::TARGETS);
    }

    /// The names the "unknown target" messages advertise must be the names
    /// `link` can actually compile, and every target must be advertised.
    #[test]
    fn every_advertised_link_target_can_actually_be_compiled() {
        for name in supported_target_names() {
            if let Err(e) = crate::compile::compiler_for(name) {
                panic!("target '{name}' is advertised but: {e}");
            }
        }
        let err = match crate::compile::compiler_for("kilocode") {
            Ok(_) => panic!("an unknown target must be refused"),
            Err(e) => e.to_string(),
        };
        use clap::ValueEnum;
        for target in LinkTarget::value_variants() {
            assert!(
                err.contains(target.as_str()),
                "refusal must advertise '{target}': {err}"
            );
        }
    }

    #[test]
    fn test_slugify_simple() {
        assert_eq!(slugify("Code Reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Agent (v2.0)"), "agent-v20");
    }

    #[test]
    fn test_slugify_multiple_separators() {
        assert_eq!(slugify("a--b__c  d"), "a-b-c-d");
    }

    #[test]
    fn create_linker_builds_the_three_legacy_targets_only() {
        for t in ["codex", "copilot", "opencode"] {
            assert!(create_linker(t).is_ok(), "{t}");
        }
        assert!(
            create_linker("claude").is_err(),
            "claude has its own adapter"
        );
        assert!(create_linker("unknown").is_err());
    }
}
