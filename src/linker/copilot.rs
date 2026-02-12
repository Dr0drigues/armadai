use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates GitHub Copilot agent files (`.github/agents/{slug}.agent.md`).
pub struct CopilotLinker;

impl Linker for CopilotLinker {
    fn name(&self) -> &str {
        "copilot"
    }

    fn default_output_dir(&self) -> &str {
        ".github/agents"
    }

    fn generate(&self, agents: &[LinkAgent], _sources: &[String]) -> Vec<OutputFile> {
        agents.iter().map(generate_file).collect()
    }
}

fn generate_file(agent: &LinkAgent) -> OutputFile {
    let slug = slugify(&agent.name);
    let path = PathBuf::from(".github/agents").join(format!("{slug}.agent.md"));

    let mut content = String::new();

    // Description intro (first line of system prompt or explicit description)
    if let Some(ref desc) = agent.description {
        content.push_str(desc);
        content.push_str("\n\n");
    }

    // System prompt
    content.push_str(&agent.system_prompt);

    // Instructions section
    if let Some(ref instructions) = agent.instructions {
        ensure_blank_line(&mut content);
        content.push_str("## Instructions\n\n");
        content.push_str(instructions);
    }

    // Output format section
    if let Some(ref output_format) = agent.output_format {
        ensure_blank_line(&mut content);
        content.push_str("## Output Format\n\n");
        content.push_str(output_format);
    }

    // Context section
    if let Some(ref context) = agent.context {
        ensure_blank_line(&mut content);
        content.push_str("## Context\n\n");
        content.push_str(context);
    }

    // Ensure trailing newline
    if !content.ends_with('\n') {
        content.push('\n');
    }

    OutputFile { path, content }
}

/// Ensure there are two newlines (blank line) before a new section.
fn ensure_blank_line(content: &mut String) {
    if !content.ends_with("\n\n") {
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(name: &str, system_prompt: &str) -> LinkAgent {
        LinkAgent {
            name: name.to_string(),
            system_prompt: system_prompt.to_string(),
            instructions: None,
            output_format: None,
            context: None,
            description: Some(
                system_prompt
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            ),
            tags: vec![],
            stacks: vec![],
        }
    }

    #[test]
    fn test_generate_simple_agent() {
        let linker = CopilotLinker;
        let agents = vec![make_agent("Code Reviewer", "You review code for bugs.")];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].path,
            PathBuf::from(".github/agents/code-reviewer.agent.md")
        );
        // Description intro + blank line + system prompt
        assert!(
            files[0]
                .content
                .starts_with("You review code for bugs.\n\n")
        );
        assert!(files[0].content.ends_with('\n'));
    }

    #[test]
    fn test_generate_with_all_sections() {
        let linker = CopilotLinker;
        let agents = vec![LinkAgent {
            name: "Test Agent".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            instructions: Some("Write unit tests.".to_string()),
            output_format: Some("Markdown report.".to_string()),
            context: Some("Python project.".to_string()),
            description: Some("You are a test agent.".to_string()),
            tags: vec![],
            stacks: vec![],
        }];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].path,
            PathBuf::from(".github/agents/test-agent.agent.md")
        );
        assert!(
            files[0]
                .content
                .contains("## Instructions\n\nWrite unit tests.")
        );
        assert!(
            files[0]
                .content
                .contains("## Output Format\n\nMarkdown report.")
        );
        assert!(files[0].content.contains("## Context\n\nPython project."));
    }

    #[test]
    fn test_agent_md_extension() {
        let linker = CopilotLinker;
        let agents = vec![make_agent("my-agent", "A simple agent.")];
        let files = linker.generate(&agents, &[]);

        // Must use .agent.md extension for Copilot
        assert_eq!(
            files[0].path,
            PathBuf::from(".github/agents/my-agent.agent.md")
        );
    }

    #[test]
    fn test_default_output_dir() {
        let linker = CopilotLinker;
        assert_eq!(linker.default_output_dir(), ".github/agents");
    }
}
