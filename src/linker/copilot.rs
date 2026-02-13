use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates GitHub Copilot agent files (`.github/agents/{slug}.agent.md`).
///
/// When a coordinator is provided, also generates `.github/copilot-instructions.md`
/// with the coordinator's prompt and a team roster. The coordinator is excluded
/// from sub-agent files.
pub struct CopilotLinker;

impl Linker for CopilotLinker {
    fn name(&self) -> &str {
        "copilot"
    }

    fn default_output_dir(&self) -> &str {
        ".github"
    }

    fn generate(
        &self,
        agents: &[LinkAgent],
        coordinator: Option<&LinkAgent>,
        _sources: &[String],
    ) -> Vec<OutputFile> {
        let mut files: Vec<OutputFile> = agents.iter().map(generate_agent_file).collect();

        if let Some(coord) = coordinator {
            files.push(generate_copilot_instructions(coord, agents));
        }

        files
    }
}

fn generate_agent_file(agent: &LinkAgent) -> OutputFile {
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

/// Generate `.github/copilot-instructions.md` with coordinator prompt and team roster.
fn generate_copilot_instructions(coordinator: &LinkAgent, agents: &[LinkAgent]) -> OutputFile {
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
            let slug = slugify(&agent.name);
            content.push_str(&format!("| {} | {} |\n", slug, desc_truncated));
        }

        content.push_str(
            "\nTo delegate to a specialized agent, mention `@<agent-name>` in your prompt.\n",
        );
    }

    // Ensure trailing newline
    if !content.ends_with('\n') {
        content.push('\n');
    }

    OutputFile {
        path: PathBuf::from(".github/copilot-instructions.md"),
        content,
    }
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
            scope: vec![],
            model: None,
            temperature: 0.7,
        }
    }

    #[test]
    fn test_generate_simple_agent() {
        let linker = CopilotLinker;
        let agents = vec![make_agent("Code Reviewer", "You review code for bugs.")];
        let files = linker.generate(&agents, None, &[]);

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
            scope: vec![],
            model: None,
            temperature: 0.7,
        }];
        let files = linker.generate(&agents, None, &[]);

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
        let files = linker.generate(&agents, None, &[]);

        // Must use .agent.md extension for Copilot
        assert_eq!(
            files[0].path,
            PathBuf::from(".github/agents/my-agent.agent.md")
        );
    }

    #[test]
    fn test_default_output_dir() {
        let linker = CopilotLinker;
        assert_eq!(linker.default_output_dir(), ".github");
    }

    #[test]
    fn test_generate_with_coordinator() {
        let linker = CopilotLinker;
        let coordinator = make_agent("Capitaine", "You are the captain of the crew.");
        let agents = vec![
            make_agent("Vigie", "You watch from the mast."),
            make_agent("Charpentier", "You repair the hull."),
        ];
        let files = linker.generate(&agents, Some(&coordinator), &[]);

        // 2 agent files + copilot-instructions.md
        assert_eq!(files.len(), 3);

        let instructions = files
            .iter()
            .find(|f| f.path.ends_with("copilot-instructions.md"))
            .unwrap();
        assert_eq!(
            instructions.path,
            PathBuf::from(".github/copilot-instructions.md")
        );
        assert!(
            instructions
                .content
                .contains("You are the captain of the crew.")
        );
        assert!(instructions.content.contains("## Team"));
        assert!(instructions.content.contains("| vigie |"));
        assert!(instructions.content.contains("| charpentier |"));
        assert!(instructions.content.contains("@<agent-name>"));

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
            temperature: 0.7,
        };
        let agents = vec![make_agent("Worker", "You do the work.")];

        let file = generate_copilot_instructions(&coordinator, &agents);
        assert!(file.content.contains("You lead the team."));
        assert!(file.content.contains("Always be kind."));
        assert!(file.content.contains("## Team"));
    }
}
