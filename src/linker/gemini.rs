use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile};

/// Generates a single `GEMINI.md` file at the project root.
///
/// The Gemini CLI reads one Markdown file, so when multiple agents are
/// present they are combined into a coordinator document with a team
/// roster.
pub struct GeminiLinker;

impl Linker for GeminiLinker {
    fn name(&self) -> &str {
        "gemini"
    }

    fn default_output_dir(&self) -> &str {
        "."
    }

    fn generate(&self, agents: &[LinkAgent], _sources: &[String]) -> Vec<OutputFile> {
        if agents.is_empty() {
            return Vec::new();
        }

        let content = if agents.len() == 1 {
            generate_single(&agents[0])
        } else {
            generate_coordinator(agents)
        };

        vec![OutputFile {
            path: PathBuf::from("GEMINI.md"),
            content,
        }]
    }
}

/// Single agent: use its prompt directly.
fn generate_single(agent: &LinkAgent) -> String {
    let mut content = String::new();
    content.push_str(&agent.system_prompt);

    if let Some(ref instructions) = agent.instructions {
        ensure_blank_line(&mut content);
        content.push_str("## Instructions\n\n");
        content.push_str(instructions);
    }

    if let Some(ref output_format) = agent.output_format {
        ensure_blank_line(&mut content);
        content.push_str("## Output Format\n\n");
        content.push_str(output_format);
    }

    if !content.ends_with('\n') {
        content.push('\n');
    }

    content
}

/// Multiple agents: generate a coordinator document with a team roster.
fn generate_coordinator(agents: &[LinkAgent]) -> String {
    let mut content = String::new();

    content.push_str(
        "You are a team coordinator managing specialized agents for this project.\n\
         Analyze each request and adopt the most appropriate role.\n",
    );

    content.push_str("\n## Team\n");

    for agent in agents {
        content.push_str(&format!("\n### {}\n\n", agent.name));
        content.push_str(&agent.system_prompt);

        if let Some(ref instructions) = agent.instructions {
            ensure_blank_line(&mut content);
            content.push_str(instructions);
        }

        if !content.ends_with('\n') {
            content.push('\n');
        }
    }

    content.push_str(
        "\n## How to operate\n\n\
         1. Analyze the user's request\n\
         2. Identify which team member is best suited\n\
         3. Adopt their expertise to respond\n\
         4. For complex tasks, combine multiple members' perspectives\n",
    );

    content
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
    fn test_generate_single_agent() {
        let linker = GeminiLinker;
        let agents = vec![make_agent("Code Reviewer", "You review code for bugs.")];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("GEMINI.md"));
        assert_eq!(files[0].content, "You review code for bugs.\n");
    }

    #[test]
    fn test_generate_single_agent_with_sections() {
        let linker = GeminiLinker;
        let agents = vec![LinkAgent {
            name: "Test Agent".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            instructions: Some("Follow TDD.".to_string()),
            output_format: Some("JSON output.".to_string()),
            context: None,
            description: Some("You are a test agent.".to_string()),
            tags: vec![],
            stacks: vec![],
        }];
        let files = linker.generate(&agents, &[]);

        assert_eq!(files.len(), 1);
        assert!(files[0].content.contains("You are a test agent."));
        assert!(files[0].content.contains("## Instructions\n\nFollow TDD."));
        assert!(
            files[0]
                .content
                .contains("## Output Format\n\nJSON output.")
        );
        assert!(files[0].content.ends_with('\n'));
    }

    #[test]
    fn test_generate_multiple_agents() {
        let linker = GeminiLinker;
        let agents = vec![
            make_agent("Code Reviewer", "You review code."),
            make_agent("Test Writer", "You write tests."),
        ];
        let files = linker.generate(&agents, &[]);

        // Single GEMINI.md file
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("GEMINI.md"));

        let content = &files[0].content;
        assert!(content.contains("team coordinator"));
        assert!(content.contains("### Code Reviewer"));
        assert!(content.contains("You review code."));
        assert!(content.contains("### Test Writer"));
        assert!(content.contains("You write tests."));
        assert!(content.contains("## How to operate"));
    }

    #[test]
    fn test_generate_empty_agents() {
        let linker = GeminiLinker;
        let files = linker.generate(&[], &[]);
        assert!(files.is_empty());
    }

    #[test]
    fn test_default_output_dir() {
        let linker = GeminiLinker;
        assert_eq!(linker.default_output_dir(), ".");
    }
}
