use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates Claude Code sub-agent files (`.claude/agents/{slug}.md`).
///
/// Each file includes a YAML frontmatter block with `name`, `description`,
/// and optional `model` fields, followed by the agent's system prompt as
/// the Markdown body.
///
/// When a coordinator is provided, also generates `.claude/CLAUDE.md` with
/// the coordinator's prompt and a team roster. The coordinator is excluded
/// from sub-agent files.
pub struct ClaudeLinker;

impl Linker for ClaudeLinker {
    fn name(&self) -> &str {
        "claude"
    }

    fn default_output_dir(&self) -> &str {
        ".claude"
    }

    fn generate(
        &self,
        agents: &[LinkAgent],
        coordinator: Option<&LinkAgent>,
        _sources: &[String],
    ) -> Vec<OutputFile> {
        let mut files: Vec<OutputFile> = agents.iter().map(generate_agent_file).collect();

        if let Some(coord) = coordinator {
            files.push(generate_claude_md(coord, agents));
        }

        files
    }
}

fn generate_agent_file(agent: &LinkAgent) -> OutputFile {
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

/// Generate `.claude/CLAUDE.md` with the coordinator's prompt and a team roster.
fn generate_claude_md(coordinator: &LinkAgent, agents: &[LinkAgent]) -> OutputFile {
    let mut content = String::new();

    content.push_str(&coordinator.system_prompt);

    if let Some(ref instructions) = coordinator.instructions {
        ensure_blank_line(&mut content);
        content.push_str(instructions);
    }

    if !agents.is_empty() {
        ensure_blank_line(&mut content);
        content.push_str("## Team\n\n");
        content.push_str("| Agent | Description |\n");
        content.push_str("|-------|-------------|\n");

        for agent in agents {
            let desc = agent
                .description
                .as_deref()
                .or_else(|| agent.system_prompt.lines().find(|l| !l.trim().is_empty()))
                .unwrap_or("");
            let desc_truncated = if desc.len() > 80 {
                format!("{}...", &desc[..77])
            } else {
                desc.to_string()
            };
            content.push_str(&format!("| {} | {} |\n", agent.name, desc_truncated));
        }

        content.push_str(
            "\nTo delegate to a specialized agent, use `/agents` and select the appropriate one.\n",
        );
    }

    OutputFile {
        path: PathBuf::from(".claude/CLAUDE.md"),
        content,
    }
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
            scope: vec![],
            model: None,
            model_fallback: vec![],
            temperature: 0.7,
        }
    }

    #[test]
    fn test_generate_simple_agent() {
        let linker = ClaudeLinker;
        let agents = vec![make_agent("Code Reviewer", "You are a code reviewer.")];
        let files = linker.generate(&agents, None, &[]);

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
            scope: vec![],
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            model_fallback: vec![],
            temperature: 0.5,
        }];
        let files = linker.generate(&agents, None, &[]);

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
        let files = linker.generate(&agents, None, &[]);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from(".claude/agents/agent-one.md"));
        assert_eq!(files[1].path, PathBuf::from(".claude/agents/agent-two.md"));
    }

    #[test]
    fn test_default_output_dir() {
        let linker = ClaudeLinker;
        assert_eq!(linker.default_output_dir(), ".claude");
    }

    #[test]
    fn test_frontmatter_escaping() {
        let linker = ClaudeLinker;
        let agents = vec![make_agent(
            "Escaper",
            "Agent with \"quotes\" and \\backslash.",
        )];
        let files = linker.generate(&agents, None, &[]);
        assert!(
            files[0]
                .content
                .contains(r#"description: "Agent with \"quotes\" and \\backslash.""#)
        );
    }

    #[test]
    fn test_generate_with_coordinator() {
        let linker = ClaudeLinker;
        let coordinator = make_agent("Capitaine", "You are the captain of the crew.");
        let agents = vec![
            make_agent("Vigie", "You watch from the mast."),
            make_agent("Charpentier", "You repair the hull."),
        ];
        let files = linker.generate(&agents, Some(&coordinator), &[]);

        // 2 agent files + CLAUDE.md
        assert_eq!(files.len(), 3);

        // CLAUDE.md should exist
        let claude_md = files
            .iter()
            .find(|f| f.path.ends_with("CLAUDE.md"))
            .unwrap();
        assert_eq!(claude_md.path, PathBuf::from(".claude/CLAUDE.md"));
        assert!(
            claude_md
                .content
                .contains("You are the captain of the crew.")
        );
        assert!(claude_md.content.contains("## Team"));
        assert!(claude_md.content.contains("| Vigie |"));
        assert!(claude_md.content.contains("| Charpentier |"));
        assert!(claude_md.content.contains("/agents"));

        // No agent file for the coordinator
        assert!(
            !files
                .iter()
                .any(|f| f.path.to_string_lossy().contains("capitaine"))
        );
    }

    #[test]
    fn test_generate_coordinator_with_instructions() {
        let coordinator = LinkAgent {
            name: "Leader".to_string(),
            system_prompt: "You lead the team.".to_string(),
            instructions: Some("Always be kind.".to_string()),
            output_format: None,
            context: None,
            description: Some("You lead the team.".to_string()),
            tags: vec![],
            stacks: vec![],
            scope: vec![],
            model: None,
            model_fallback: vec![],
            temperature: 0.7,
        };
        let agents = vec![make_agent("Worker", "You do the work.")];

        let file = generate_claude_md(&coordinator, &agents);
        assert!(file.content.contains("You lead the team."));
        assert!(file.content.contains("Always be kind."));
        assert!(file.content.contains("## Team"));
    }
}
