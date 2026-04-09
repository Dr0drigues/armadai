use std::path::PathBuf;

use super::{LinkAgent, Linker, OutputFile, armadai_protocol_block, slugify};

/// Generates OpenAI Codex CLI config files in `.codex/`.
///
/// Produces:
/// - `agents/{slug}.toml` — one TOML config per agent (model + developer_instructions)
/// - `config.toml` — main config referencing all agents
/// - `AGENTS.md` — coordinator context document (same pattern as Gemini linker)
pub struct CodexLinker;

impl Linker for CodexLinker {
    fn name(&self) -> &str {
        "codex"
    }

    fn default_output_dir(&self) -> &str {
        ".codex"
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

        // Individual agent TOML files
        for agent in agents {
            files.push(generate_agent_file(agent));
        }

        // config.toml — main config referencing all agents
        files.push(generate_config_toml(agents));

        // AGENTS.md — coordinator context document
        files.push(generate_agents_md(agents, coordinator));

        files
    }
}

/// Generate an individual agent TOML file at `.codex/agents/{slug}.toml`.
fn generate_agent_file(agent: &LinkAgent) -> OutputFile {
    let slug = slugify(&agent.name);
    let path = PathBuf::from(".codex/agents").join(format!("{slug}.toml"));

    let model = agent.model.as_deref().unwrap_or("o3-mini");

    // Build developer_instructions from all sections
    let mut instructions = String::new();
    instructions.push_str(&agent.system_prompt);

    if let Some(ref inst) = agent.instructions {
        ensure_blank_line(&mut instructions);
        instructions.push_str("## Instructions\n\n");
        instructions.push_str(inst);
    }

    if let Some(ref output_format) = agent.output_format {
        ensure_blank_line(&mut instructions);
        instructions.push_str("## Output Format\n\n");
        instructions.push_str(output_format);
    }

    if let Some(ref context) = agent.context {
        ensure_blank_line(&mut instructions);
        instructions.push_str("## Context\n\n");
        instructions.push_str(context);
    }

    let mut content = String::new();
    content.push_str(&format!("model = \"{model}\"\n"));
    content.push_str(&format!(
        "developer_instructions = \"\"\"\n{instructions}\n\"\"\"\n"
    ));

    OutputFile { path, content }
}

/// Generate `.codex/config.toml` with references to all agent configs.
fn generate_config_toml(agents: &[LinkAgent]) -> OutputFile {
    let mut content = String::new();

    for agent in agents {
        let slug = slugify(&agent.name);
        let desc = agent
            .description
            .as_deref()
            .or_else(|| agent.system_prompt.lines().find(|l| !l.trim().is_empty()))
            .unwrap_or(&agent.name);
        let desc_escaped = toml_escape(desc);

        content.push_str(&format!("[agents.{slug}]\n"));
        content.push_str(&format!("description = \"{desc_escaped}\"\n"));
        content.push_str(&format!("config_file = \"agents/{slug}.toml\"\n"));
        content.push('\n');
    }

    OutputFile {
        path: PathBuf::from(".codex/config.toml"),
        content,
    }
}

/// Generate the `AGENTS.md` coordinator context document.
fn generate_agents_md(agents: &[LinkAgent], coordinator: Option<&LinkAgent>) -> OutputFile {
    let mut content = String::new();

    if let Some(coord) = coordinator {
        content.push_str(&coord.system_prompt);

        if let Some(ref instructions) = coord.instructions {
            ensure_blank_line(&mut content);
            content.push_str(instructions);
        }

        if !agents.is_empty() {
            ensure_blank_line(&mut content);
            content.push_str("## Team\n\n");
            content.push_str("| Agent | Config | Description |\n");
            content.push_str("|-------|--------|-------------|\n");

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
                    "| {} | [agents/{slug}.toml](agents/{slug}.toml) | {} |\n",
                    agent.name, desc_truncated
                ));
            }
        }
    } else if agents.len() == 1 {
        let slug = slugify(&agents[0].name);
        content.push_str(&format!("# {}\n\n", agents[0].name));
        content.push_str(&format!(
            "See [agents/{slug}.toml](agents/{slug}.toml) for the full agent config.\n"
        ));
    } else {
        content.push_str(
            "You are a team coordinator managing specialized agents for this project.\n\
             Analyze each request and adopt the most appropriate role.\n\n\
             ## Team\n\n\
             | Agent | Config | Description |\n\
             |-------|--------|-------------|\n",
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
                "| {} | [agents/{slug}.toml](agents/{slug}.toml) | {} |\n",
                agent.name, desc_truncated
            ));
        }

        content.push_str(
            "\n## How to operate\n\n\
             1. Analyze the user's request\n\
             2. Identify which team member is best suited\n\
             3. Read the corresponding agent config for their full instructions\n\
             4. Adopt their expertise to respond\n\
             5. For complex tasks, combine multiple members' perspectives\n",
        );
    }

    // Inject ArmadAI response protocol
    content.push_str(armadai_protocol_block());

    OutputFile {
        path: PathBuf::from(".codex/AGENTS.md"),
        content,
    }
}

/// Escape a string for use in a TOML double-quoted value.
fn toml_escape(s: &str) -> String {
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
            provider: None,
        }
    }

    #[test]
    fn test_generate_simple_agent() {
        let linker = CodexLinker;
        let agents = vec![make_agent("Code Reviewer", "You review code for bugs.")];
        let files = linker.generate(&agents, None, &[]);

        // agent file + config.toml + AGENTS.md
        assert_eq!(files.len(), 3);

        let agent_file = files
            .iter()
            .find(|f| f.path.ends_with("code-reviewer.toml"))
            .unwrap();
        assert_eq!(
            agent_file.path,
            PathBuf::from(".codex/agents/code-reviewer.toml")
        );
        assert!(agent_file.content.contains("model = \"o3-mini\""));
        assert!(
            agent_file
                .content
                .contains("developer_instructions = \"\"\"")
        );
        assert!(agent_file.content.contains("You review code for bugs."));
    }

    #[test]
    fn test_generate_agent_with_model() {
        let linker = CodexLinker;
        let agents = vec![LinkAgent {
            name: "Test Agent".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            instructions: Some("Follow TDD.".to_string()),
            output_format: Some("JSON output.".to_string()),
            context: Some("Rust project.".to_string()),
            description: Some("You are a test agent.".to_string()),
            tags: vec![],
            stacks: vec![],
            scope: vec![],
            model: Some("gpt-4o".to_string()),
            model_fallback: vec![],
            temperature: 0.3,
            provider: None,
        }];
        let files = linker.generate(&agents, None, &[]);

        let agent_file = files
            .iter()
            .find(|f| f.path.ends_with("test-agent.toml"))
            .unwrap();
        assert!(agent_file.content.contains("model = \"gpt-4o\""));
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
        assert!(agent_file.content.contains("## Context\n\nRust project."));
    }

    #[test]
    fn test_generate_multiple_agents() {
        let linker = CodexLinker;
        let agents = vec![
            make_agent("Code Reviewer", "You review code."),
            make_agent("Test Writer", "You write tests."),
        ];
        let files = linker.generate(&agents, None, &[]);

        // 2 agent files + config.toml + AGENTS.md
        assert_eq!(files.len(), 4);

        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".codex/agents/code-reviewer.toml")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".codex/agents/test-writer.toml")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".codex/config.toml")
        );
        assert!(
            files
                .iter()
                .any(|f| f.path.as_os_str() == ".codex/AGENTS.md")
        );

        let agents_md = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .unwrap();
        assert!(agents_md.content.contains("team coordinator"));
        assert!(agents_md.content.contains("agents/code-reviewer.toml"));
        assert!(agents_md.content.contains("agents/test-writer.toml"));
        assert!(agents_md.content.contains("## How to operate"));
    }

    #[test]
    fn test_generate_empty_agents() {
        let linker = CodexLinker;
        let files = linker.generate(&[], None, &[]);
        assert!(files.is_empty());
    }

    #[test]
    fn test_default_output_dir() {
        let linker = CodexLinker;
        assert_eq!(linker.default_output_dir(), ".codex");
    }

    #[test]
    fn test_generate_with_coordinator() {
        let linker = CodexLinker;
        let coordinator = make_agent("Capitaine", "You are the captain of the crew.");
        let agents = vec![
            make_agent("Vigie", "You watch from the mast."),
            make_agent("Charpentier", "You repair the hull."),
        ];
        let files = linker.generate(&agents, Some(&coordinator), &[]);

        // 2 agent files + config.toml + AGENTS.md
        assert_eq!(files.len(), 4);

        let agents_md = files
            .iter()
            .find(|f| f.path.ends_with("AGENTS.md"))
            .unwrap();
        assert!(
            agents_md
                .content
                .contains("You are the captain of the crew.")
        );
        assert!(!agents_md.content.contains("team coordinator"));
        assert!(agents_md.content.contains("## Team"));
        assert!(agents_md.content.contains("agents/vigie.toml"));
        assert!(agents_md.content.contains("agents/charpentier.toml"));

        // No agent file for the coordinator
        assert!(
            !files
                .iter()
                .any(|f| f.path.to_string_lossy().contains("capitaine"))
        );
    }

    #[test]
    fn test_config_toml_structure() {
        let linker = CodexLinker;
        let agents = vec![
            make_agent("Code Reviewer", "You review code."),
            make_agent("Test Writer", "You write tests."),
        ];
        let files = linker.generate(&agents, None, &[]);

        let config = files
            .iter()
            .find(|f| f.path.ends_with("config.toml"))
            .unwrap();
        assert!(config.content.contains("[agents.code-reviewer]"));
        assert!(
            config
                .content
                .contains("config_file = \"agents/code-reviewer.toml\"")
        );
        assert!(config.content.contains("[agents.test-writer]"));
        assert!(
            config
                .content
                .contains("config_file = \"agents/test-writer.toml\"")
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
            provider: None,
        };
        let agents = vec![make_agent("Worker", "You do the work.")];

        let file = generate_agents_md(&agents, Some(&coordinator));
        assert!(file.content.contains("You lead the team."));
        assert!(file.content.contains("Always be kind."));
        assert!(file.content.contains("## Team"));
        assert!(file.content.contains("agents/worker.toml"));

        // Protocol block must be present
        assert!(file.content.contains("## ArmadAI Response Protocol"));
        assert!(file.content.contains("<!--ARMADAI_END-->"));
        assert!(file.content.contains("<!--ARMADAI_DELEGATE:agent-name-->"));
        assert!(file.content.contains("<!--ARMADAI_META:status=complete-->"));
    }
}
