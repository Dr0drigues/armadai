mod claude;
mod copilot;
mod gemini;

pub use claude::ClaudeLinker;
pub use copilot::CopilotLinker;
pub use gemini::GeminiLinker;

use std::path::PathBuf;

use crate::core::agent::Agent;

/// A resolved agent ready for linking.
#[allow(dead_code)]
pub struct LinkAgent {
    pub name: String,
    pub system_prompt: String,
    pub instructions: Option<String>,
    pub output_format: Option<String>,
    pub context: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub stacks: Vec<String>,
}

/// A file to be written by a linker.
pub struct OutputFile {
    pub path: PathBuf,
    pub content: String,
}

/// Trait for generating target-specific config files.
#[allow(dead_code)]
pub trait Linker: Send + Sync {
    fn name(&self) -> &str;
    fn default_output_dir(&self) -> &str;
    fn generate(&self, agents: &[LinkAgent], sources: &[String]) -> Vec<OutputFile>;
}

/// Create a linker for the given target name.
pub fn create_linker(target: &str) -> anyhow::Result<Box<dyn Linker>> {
    match target {
        "claude" => Ok(Box::new(ClaudeLinker)),
        "copilot" => Ok(Box::new(CopilotLinker)),
        "gemini" => Ok(Box::new(GeminiLinker)),
        _ => anyhow::bail!(
            "Unknown link target: '{target}'. Supported targets: claude, copilot, gemini"
        ),
    }
}

/// Convert an agent name to a kebab-case slug suitable for filenames.
pub fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else if c == ' ' || c == '_' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|c| *c != '\0')
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

impl From<&Agent> for LinkAgent {
    fn from(agent: &Agent) -> Self {
        let description = agent
            .system_prompt
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string());

        Self {
            name: agent.name.clone(),
            system_prompt: agent.system_prompt.clone(),
            instructions: agent.instructions.clone(),
            output_format: agent.output_format.clone(),
            context: agent.context.clone(),
            description,
            tags: agent.metadata.tags.clone(),
            stacks: agent.metadata.stacks.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_simple() {
        assert_eq!(slugify("Code Reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_already_kebab() {
        assert_eq!(slugify("code-reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_underscores() {
        assert_eq!(slugify("my_test_agent"), "my-test-agent");
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
    fn test_create_linker_claude() {
        assert!(create_linker("claude").is_ok());
    }

    #[test]
    fn test_create_linker_copilot() {
        assert!(create_linker("copilot").is_ok());
    }

    #[test]
    fn test_create_linker_gemini() {
        assert!(create_linker("gemini").is_ok());
    }

    #[test]
    fn test_create_linker_unknown() {
        assert!(create_linker("unknown").is_err());
    }
}
