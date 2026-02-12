use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, slugify};

/// Generates Gemini CLI config files in `.gemini/`.
///
/// Produces:
/// - `AGENTS.md` — coordinator document referencing all agents
/// - `{slug}.md` — one file per agent with its full prompt
/// - `settings.json` — Gemini CLI settings pointing to AGENTS.md
pub struct GeminiLinker;

impl Linker for GeminiLinker {
    fn name(&self) -> &str {
        "gemini"
    }

    fn default_output_dir(&self) -> &str {
        ".gemini"
    }

    fn generate(&self, agents: &[LinkAgent], _sources: &[String]) -> Vec<OutputFile> {
        if agents.is_empty() {
            return Vec::new();
        }

        let mut files = Vec::new();

        // Individual agent files
        for agent in agents {
            files.push(generate_agent_file(agent));
        }

        // AGENTS.md — coordinator that references individual files
        files.push(generate_agents_md(agents));

        // settings.json
        files.push(OutputFile {
            path: PathBuf::from(".gemini/settings.json"),
            content: "{\n  \"contextFileName\": \"AGENTS.md\"\n}\n".to_string(),
        });

        files
    }
}

/// Generate an individual agent file at `.gemini/{slug}.md`.
fn generate_agent_file(agent: &LinkAgent) -> OutputFile {
    let slug = slugify(&agent.name);
    let path = PathBuf::from(".gemini").join(format!("{slug}.md"));

    let mut content = String::new();

    content.push_str(&format!("# {}\n\n", agent.name));
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

/// Generate the `AGENTS.md` coordinator document.
fn generate_agents_md(agents: &[LinkAgent]) -> OutputFile {
    let mut content = String::new();

    if agents.len() == 1 {
        let slug = slugify(&agents[0].name);
        content.push_str(&format!("# {}\n\n", agents[0].name));
        content.push_str(&format!(
            "See [{slug}.md]({slug}.md) for the full agent prompt.\n"
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
                "| {} | [{slug}.md]({slug}.md) | {} |\n",
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

        // agent file + AGENTS.md + settings.json
        assert_eq!(files.len(), 3);

        let agent_file = files
            .iter()
            .find(|f| f.path.ends_with("code-reviewer.md"))
            .unwrap();
        assert!(agent_file.content.starts_with("# Code Reviewer\n\n"));
        assert!(agent_file.content.contains("You review code for bugs."));

        let agents_md = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .unwrap();
        assert!(agents_md.content.contains("code-reviewer.md"));

        let settings = files
            .iter()
            .find(|f| f.path.ends_with("settings.json"))
            .unwrap();
        assert!(
            settings
                .content
                .contains("\"contextFileName\": \"AGENTS.md\"")
        );
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

        let agent_file = files
            .iter()
            .find(|f| f.path.ends_with("test-agent.md"))
            .unwrap();
        assert!(agent_file.content.starts_with("# Test Agent\n\n"));
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
        let files = linker.generate(&agents, &[]);

        // 2 agent files + AGENTS.md + settings.json
        assert_eq!(files.len(), 4);

        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".gemini/code-reviewer.md")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".gemini/test-writer.md")
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
        assert!(agents_md.content.contains("code-reviewer.md"));
        assert!(agents_md.content.contains("test-writer.md"));
        assert!(agents_md.content.contains("## How to operate"));
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
        assert_eq!(linker.default_output_dir(), ".gemini");
    }

    #[test]
    fn test_settings_json_content() {
        let linker = GeminiLinker;
        let agents = vec![make_agent("Agent", "Prompt.")];
        let files = linker.generate(&agents, &[]);

        let settings = files
            .iter()
            .find(|f| f.path.ends_with("settings.json"))
            .unwrap();
        assert_eq!(
            settings.content,
            "{\n  \"contextFileName\": \"AGENTS.md\"\n}\n"
        );
    }
}
