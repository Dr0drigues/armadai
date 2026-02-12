use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates Gemini CLI config files in `.gemini/`.
///
/// Produces:
/// - `agents/{slug}.md` — one sub-agent file per agent (with YAML frontmatter)
/// - `AGENTS.md` — coordinator context document
/// - `settings.json` — Gemini CLI settings with contextFileName and enableAgents
///
/// When a coordinator is provided, its prompt is used in AGENTS.md instead of
/// the generic "team coordinator" text. The coordinator is excluded from
/// individual agent files.
pub struct GeminiLinker;

impl Linker for GeminiLinker {
    fn name(&self) -> &str {
        "gemini"
    }

    fn default_output_dir(&self) -> &str {
        ".gemini"
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

        let mut files = Vec::new();

        // Individual agent files in .gemini/agents/
        for agent in agents {
            files.push(generate_agent_file(agent));
        }

        // AGENTS.md — coordinator context document
        files.push(generate_agents_md(agents, coordinator));

        // settings.json — enable agents + set context file
        files.push(OutputFile {
            path: PathBuf::from(".gemini/settings.json"),
            content: "{\n  \"contextFileName\": \"AGENTS.md\",\n  \"experimental\": {\n    \"enableAgents\": true\n  }\n}\n".to_string(),
        });

        files
    }
}

/// Generate an individual agent file at `.gemini/agents/{slug}.md` with YAML frontmatter.
fn generate_agent_file(agent: &LinkAgent) -> OutputFile {
    let slug = slugify(&agent.name);
    let path = PathBuf::from(".gemini/agents").join(format!("{slug}.md"));

    let mut content = String::new();

    // YAML frontmatter required by Gemini CLI
    content.push_str("---\n");
    content.push_str(&format!("name: {slug}\n"));

    let description = agent
        .description
        .as_deref()
        .or_else(|| agent.system_prompt.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or(&agent.name);
    content.push_str(&format!("description: \"{}\"\n", yaml_escape(description)));

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

/// Generate the `AGENTS.md` coordinator context document.
fn generate_agents_md(agents: &[LinkAgent], coordinator: Option<&LinkAgent>) -> OutputFile {
    let mut content = String::new();

    if let Some(coord) = coordinator {
        // Use coordinator's prompt as the main content
        content.push_str(&coord.system_prompt);

        if let Some(ref instructions) = coord.instructions {
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
    } else if agents.len() == 1 {
        let slug = slugify(&agents[0].name);
        content.push_str(&format!("# {}\n\n", agents[0].name));
        content.push_str(&format!(
            "See [agents/{slug}.md](agents/{slug}.md) for the full agent prompt.\n"
        ));
    } else {
        content.push_str(
            "You are a team coordinator managing specialized agents for this project.\n\
             Analyze each request and adopt the most appropriate role.\n\n\
             ## Team\n\n\
             | Agent | File | Description |\n\
             |-------|------|-------------|\n",
        );

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

        content.push_str(
            "\n## How to operate\n\n\
             1. Analyze the user's request\n\
             2. Identify which team member is best suited\n\
             3. Read the corresponding agent file for their full instructions\n\
             4. Adopt their expertise to respond\n\
             5. For complex tasks, combine multiple members' perspectives\n",
        );
    }

    OutputFile {
        path: PathBuf::from(".gemini/AGENTS.md"),
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
            model: None,
            temperature: 0.7,
        }
    }

    #[test]
    fn test_generate_single_agent() {
        let linker = GeminiLinker;
        let agents = vec![make_agent("Code Reviewer", "You review code for bugs.")];
        let files = linker.generate(&agents, None, &[]);

        // agent file + AGENTS.md + settings.json
        assert_eq!(files.len(), 3);

        let agent_file = files
            .iter()
            .find(|f| f.path.ends_with("code-reviewer.md"))
            .unwrap();
        assert!(agent_file.content.starts_with("---\n"));
        assert!(agent_file.content.contains("name: code-reviewer\n"));
        assert!(agent_file.content.contains("You review code for bugs."));
        assert_eq!(
            agent_file.path,
            PathBuf::from(".gemini/agents/code-reviewer.md")
        );

        let agents_md = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .unwrap();
        assert!(agents_md.content.contains("agents/code-reviewer.md"));

        let settings = files
            .iter()
            .find(|f| f.path.ends_with("settings.json"))
            .unwrap();
        assert!(
            settings
                .content
                .contains("\"contextFileName\": \"AGENTS.md\"")
        );
        assert!(settings.content.contains("\"enableAgents\": true"));
    }

    #[test]
    fn test_generate_agent_with_model() {
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
            model: Some("gemini-2.5-pro".to_string()),
            temperature: 0.3,
        }];
        let files = linker.generate(&agents, None, &[]);

        let agent_file = files
            .iter()
            .find(|f| f.path.ends_with("test-agent.md"))
            .unwrap();
        assert!(agent_file.content.contains("model: gemini-2.5-pro\n"));
        assert!(agent_file.content.contains("temperature: 0.3\n"));
        assert!(agent_file.content.contains("You are a test agent."));
        assert!(
            agent_file
                .content
                .contains("## Instructions\n\nFollow TDD.")
        );
        assert!(
            agent_file
                .content
                .contains("## Output Format\n\nJSON output.")
        );
        assert!(agent_file.content.ends_with('\n'));
    }

    #[test]
    fn test_generate_multiple_agents() {
        let linker = GeminiLinker;
        let agents = vec![
            make_agent("Code Reviewer", "You review code."),
            make_agent("Test Writer", "You write tests."),
        ];
        let files = linker.generate(&agents, None, &[]);

        // 2 agent files + AGENTS.md + settings.json
        assert_eq!(files.len(), 4);

        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".gemini/agents/code-reviewer.md")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".gemini/agents/test-writer.md")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".gemini/AGENTS.md")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".gemini/settings.json")
        );

        let agents_md = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .unwrap();
        assert!(agents_md.content.contains("team coordinator"));
        assert!(agents_md.content.contains("agents/code-reviewer.md"));
        assert!(agents_md.content.contains("agents/test-writer.md"));
        assert!(agents_md.content.contains("## How to operate"));
    }

    #[test]
    fn test_generate_empty_agents() {
        let linker = GeminiLinker;
        let files = linker.generate(&[], None, &[]);
        assert!(files.is_empty());
    }

    #[test]
    fn test_default_output_dir() {
        let linker = GeminiLinker;
        assert_eq!(linker.default_output_dir(), ".gemini");
    }

    #[test]
    fn test_settings_json_content() {
        let linker = GeminiLinker;
        let agents = vec![make_agent("Agent", "Prompt.")];
        let files = linker.generate(&agents, None, &[]);

        let settings = files
            .iter()
            .find(|f| f.path.ends_with("settings.json"))
            .unwrap();
        assert!(settings.content.contains("\"enableAgents\": true"));
        assert!(
            settings
                .content
                .contains("\"contextFileName\": \"AGENTS.md\"")
        );
    }

    #[test]
    fn test_generate_with_coordinator() {
        let linker = GeminiLinker;
        let coordinator = make_agent("Capitaine", "You are the captain of the crew.");
        let agents = vec![
            make_agent("Vigie", "You watch from the mast."),
            make_agent("Charpentier", "You repair the hull."),
        ];
        let files = linker.generate(&agents, Some(&coordinator), &[]);

        // 2 agent files + AGENTS.md + settings.json
        assert_eq!(files.len(), 4);

        let agents_md = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .unwrap();
        // Should contain coordinator's prompt, not generic text
        assert!(
            agents_md
                .content
                .contains("You are the captain of the crew.")
        );
        assert!(!agents_md.content.contains("team coordinator"));
        // Should have the team roster
        assert!(agents_md.content.contains("## Team"));
        assert!(agents_md.content.contains("agents/vigie.md"));
        assert!(agents_md.content.contains("agents/charpentier.md"));

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
            model: None,
            temperature: 0.7,
        };
        let agents = vec![make_agent("Worker", "You do the work.")];

        let file = generate_agents_md(&agents, Some(&coordinator));
        assert!(file.content.contains("You lead the team."));
        assert!(file.content.contains("Always be kind."));
        assert!(file.content.contains("## Team"));
        assert!(file.content.contains("agents/worker.md"));
    }
}
