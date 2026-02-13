use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates OpenCode agent files in `.opencode/`.
///
/// Produces:
/// - `agents/{slug}.md` — one sub-agent file per agent (with YAML frontmatter)
/// - `instructions.md` — coordinator instructions (only when a coordinator is provided)
///
/// OpenCode agents use `mode: subagent` in frontmatter and derive their name
/// from the filename (no `name` field in frontmatter). When a coordinator is
/// provided, its prompt is written to `instructions.md` with a team roster.
pub struct OpencodeLinker;

impl Linker for OpencodeLinker {
    fn name(&self) -> &str {
        "opencode"
    }

    fn default_output_dir(&self) -> &str {
        ".opencode"
    }

    fn generate(
        &self,
        agents: &[LinkAgent],
        coordinator: Option<&LinkAgent>,
        _sources: &[String],
    ) -> Vec<OutputFile> {
        if agents.is_empty() && coordinator.is_none() {
            return Vec::new();
        }

        let mut files: Vec<OutputFile> = agents.iter().map(generate_agent_file).collect();

        if let Some(coord) = coordinator {
            files.push(generate_instructions_md(coord, agents));
        }

        files
    }
}

/// Generate an individual agent file at `.opencode/agents/{slug}.md` with YAML frontmatter.
fn generate_agent_file(agent: &LinkAgent) -> OutputFile {
    let slug = slugify(&agent.name);
    let path = PathBuf::from(".opencode/agents").join(format!("{slug}.md"));

    let mut content = String::new();

    // YAML frontmatter — no `name` field (slug is the filename)
    content.push_str("---\n");

    let description = agent
        .description
        .as_deref()
        .or_else(|| agent.system_prompt.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or(&agent.name);
    content.push_str(&format!("description: \"{}\"\n", yaml_escape(description)));

    content.push_str("mode: subagent\n");

    if let Some(ref model) = agent.model {
        content.push_str(&format!("model: {model}\n"));
    }
    if (agent.temperature - 0.7).abs() > f32::EPSILON {
        content.push_str(&format!("temperature: {}\n", agent.temperature));
    }

    content.push_str("---\n\n");

    // Body: system prompt
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

    if let Some(ref context) = agent.context {
        ensure_blank_line(&mut content);
        content.push_str("## Context\n\n");
        content.push_str(context);
    }

    if !content.ends_with('\n') {
        content.push('\n');
    }

    OutputFile { path, content }
}

/// Generate `.opencode/instructions.md` with the coordinator's prompt and a team roster.
fn generate_instructions_md(coordinator: &LinkAgent, agents: &[LinkAgent]) -> OutputFile {
    let mut content = String::new();

    content.push_str(&coordinator.system_prompt);

    if let Some(ref instructions) = coordinator.instructions {
        ensure_blank_line(&mut content);
        content.push_str(instructions);
    }

    if !agents.is_empty() {
        ensure_blank_line(&mut content);
        content.push_str("## Team\n\n");
        content.push_str("| Agent | File | Description |\n");
        content.push_str("|-------|------|-------------|\n");

        for agent in agents {
            let slug = slugify(&agent.name);
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
            content.push_str(&format!(
                "| {} | [agents/{slug}.md](agents/{slug}.md) | {} |\n",
                agent.name, desc_truncated
            ));
        }
    }

    OutputFile {
        path: PathBuf::from(".opencode/instructions.md"),
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
            temperature: 0.7,
        }
    }

    #[test]
    fn test_name_and_default_output_dir() {
        let linker = OpencodeLinker;
        assert_eq!(linker.name(), "opencode");
        assert_eq!(linker.default_output_dir(), ".opencode");
    }

    #[test]
    fn test_generate_simple_agent() {
        let linker = OpencodeLinker;
        let agents = vec![make_agent("Code Reviewer", "You review code for bugs.")];
        let files = linker.generate(&agents, None, &[]);

        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, PathBuf::from(".opencode/agents/code-reviewer.md"));
        assert!(f.content.starts_with("---\n"));
        assert!(f.content.contains("mode: subagent\n"));
        assert!(
            f.content
                .contains("description: \"You review code for bugs.\"\n")
        );
        // No `name:` field in frontmatter
        assert!(!f.content.contains("name:"));
        assert!(f.content.contains("You review code for bugs."));
        assert!(f.content.ends_with('\n'));
    }

    #[test]
    fn test_generate_agent_with_model_and_temperature() {
        let linker = OpencodeLinker;
        let agents = vec![LinkAgent {
            name: "Test Agent".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            instructions: None,
            output_format: None,
            context: None,
            description: Some("You are a test agent.".to_string()),
            tags: vec![],
            stacks: vec![],
            scope: vec![],
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            temperature: 0.3,
        }];
        let files = linker.generate(&agents, None, &[]);

        let f = &files[0];
        assert!(f.content.contains("model: anthropic/claude-sonnet-4-5\n"));
        assert!(f.content.contains("temperature: 0.3\n"));
    }

    #[test]
    fn test_generate_agent_default_temperature_omitted() {
        let linker = OpencodeLinker;
        let agents = vec![make_agent("Agent", "Prompt.")];
        let files = linker.generate(&agents, None, &[]);

        assert!(!files[0].content.contains("temperature:"));
    }

    #[test]
    fn test_generate_agent_with_all_sections() {
        let linker = OpencodeLinker;
        let agents = vec![LinkAgent {
            name: "Full Agent".to_string(),
            system_prompt: "You are a full agent.".to_string(),
            instructions: Some("Follow TDD.".to_string()),
            output_format: Some("JSON output.".to_string()),
            context: Some("Rust project.".to_string()),
            description: Some("You are a full agent.".to_string()),
            tags: vec!["test".to_string()],
            stacks: vec!["rust".to_string()],
            scope: vec![],
            model: Some("anthropic/claude-sonnet-4-5".to_string()),
            temperature: 0.5,
        }];
        let files = linker.generate(&agents, None, &[]);

        let f = &files[0];
        assert!(f.content.contains("mode: subagent\n"));
        assert!(f.content.contains("You are a full agent."));
        assert!(f.content.contains("## Instructions\n\nFollow TDD."));
        assert!(f.content.contains("## Output Format\n\nJSON output."));
        assert!(f.content.contains("## Context\n\nRust project."));
        assert!(f.content.ends_with('\n'));
    }

    #[test]
    fn test_generate_multiple_agents() {
        let linker = OpencodeLinker;
        let agents = vec![
            make_agent("Code Reviewer", "You review code."),
            make_agent("Test Writer", "You write tests."),
        ];
        let files = linker.generate(&agents, None, &[]);

        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".opencode/agents/code-reviewer.md")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".opencode/agents/test-writer.md")
        );
    }

    #[test]
    fn test_generate_empty_agents() {
        let linker = OpencodeLinker;
        let files = linker.generate(&[], None, &[]);
        assert!(files.is_empty());
    }

    #[test]
    fn test_generate_with_coordinator() {
        let linker = OpencodeLinker;
        let coordinator = make_agent("Capitaine", "You are the captain of the crew.");
        let agents = vec![
            make_agent("Vigie", "You watch from the mast."),
            make_agent("Charpentier", "You repair the hull."),
        ];
        let files = linker.generate(&agents, Some(&coordinator), &[]);

        // 2 agent files + instructions.md
        assert_eq!(files.len(), 3);

        let instructions = files
            .iter()
            .find(|f| f.path.ends_with("instructions.md"))
            .unwrap();
        assert_eq!(
            instructions.path,
            PathBuf::from(".opencode/instructions.md")
        );
        assert!(
            instructions
                .content
                .contains("You are the captain of the crew.")
        );
        assert!(instructions.content.contains("## Team"));
        assert!(instructions.content.contains("agents/vigie.md"));
        assert!(instructions.content.contains("agents/charpentier.md"));

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

        let file = generate_instructions_md(&coordinator, &agents);
        assert!(file.content.contains("You lead the team."));
        assert!(file.content.contains("Always be kind."));
        assert!(file.content.contains("## Team"));
        assert!(file.content.contains("agents/worker.md"));
    }

    #[test]
    fn test_frontmatter_escaping() {
        let linker = OpencodeLinker;
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
}
