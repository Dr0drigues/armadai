use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates Claude Code sub-agent files (`.claude/agents/{slug}.md`).
///
/// Each file includes a YAML frontmatter block with `name`, `description`,
/// and optional `model` fields, followed by the agent's system prompt as
/// the Markdown body.
pub struct ClaudeLinker;

impl Linker for ClaudeLinker {
    fn name(&self) -> &str {
        "claude"
    }

    fn default_output_dir(&self) -> &str {
        ".claude/agents"
    }

    fn generate(&self, agents: &[LinkAgent], _sources: &[String]) -> Vec<OutputFile> {
        agents.iter().map(generate_file).collect()
    }
}

fn generate_file(agent: &LinkAgent) -> OutputFile {
    let slug = slugify(&agent.name);
    let path = PathBuf::from(".claude/agents").join(format!("{slug}.md"));

    let mut content = String::new();

    // YAML frontmatter required by Claude Code
    content.push_str("---\n");
    content.push_str(&format!("name: {slug}\n"));

    let description = agent
        .description
        .as_deref()
        .or_else(|| agent.system_prompt.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or(&agent.name);
    // Escape YAML string
    content.push_str(&format!("description: \"{}\"\n", yaml_escape(description)));
    content.push_str("---\n\n");

    // System prompt (main content)
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

/// Escape a string for use as a YAML double-quoted value.
fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
            description: Some(system_prompt.lines().next().unwrap_or("").to_string()),
            tags: vec![],
            stacks: vec![],
        }
    }

    #[test]
    fn test_generate_simple_agent() {
        let linker = ClaudeLinker;
        let agents = vec![make_agent("Code Reviewer", "You are a code reviewer.")];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].path,
            PathBuf::from(".claude/agents/code-reviewer.md")
        );
        assert!(files[0].content.starts_with("---\n"));
        assert!(files[0].content.contains("name: code-reviewer\n"));
        assert!(
            files[0]
                .content
                .contains("description: \"You are a code reviewer.\"")
        );
        assert!(files[0].content.contains("---\n\nYou are a code reviewer."));
    }

    #[test]
    fn test_generate_with_all_sections() {
        let linker = ClaudeLinker;
        let agents = vec![LinkAgent {
            name: "Test Agent".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            instructions: Some("Follow TDD practices.".to_string()),
            output_format: Some("JSON output.".to_string()),
            context: Some("Rust project.".to_string()),
            description: Some("You are a test agent.".to_string()),
            tags: vec!["test".to_string()],
            stacks: vec!["rust".to_string()],
        }];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from(".claude/agents/test-agent.md"));
        assert!(files[0].content.contains("name: test-agent\n"));
        assert!(files[0].content.contains("You are a test agent."));
        assert!(
            files[0]
                .content
                .contains("## Instructions\n\nFollow TDD practices.")
        );
        assert!(
            files[0]
                .content
                .contains("## Output Format\n\nJSON output.")
        );
        assert!(files[0].content.contains("## Context\n\nRust project."));
        assert!(files[0].content.ends_with('\n'));
    }

    #[test]
    fn test_generate_multiple_agents() {
        let linker = ClaudeLinker;
        let agents = vec![
            make_agent("Agent One", "First agent."),
            make_agent("Agent Two", "Second agent."),
        ];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from(".claude/agents/agent-one.md"));
        assert_eq!(files[1].path, PathBuf::from(".claude/agents/agent-two.md"));
    }

    #[test]
    fn test_default_output_dir() {
        let linker = ClaudeLinker;
        assert_eq!(linker.default_output_dir(), ".claude/agents");
    }

    #[test]
    fn test_frontmatter_escaping() {
        let linker = ClaudeLinker;
        let agents = vec![make_agent(
            "Escaper",
            "Agent with \"quotes\" and \\backslash.",
        )];
        let files = linker.generate(&agents, &[]);
        assert!(
            files[0]
                .content
                .contains(r#"description: "Agent with \"quotes\" and \\backslash.""#)
        );
    }
}
