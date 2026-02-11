use std::path::Path;

use anyhow::{Context, bail};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::core::agent::{Agent, PipelineConfig};

/// Parse a Markdown agent definition file into an Agent struct.
pub fn parse_agent_file(path: &Path) -> anyhow::Result<Agent> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut name = String::new();
    let mut current_section: Option<String> = None;
    let mut sections: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut current_text = String::new();

    let parser = Parser::new(&content);

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                // Flush previous section
                if let Some(ref section) = current_section {
                    sections.insert(section.clone(), current_text.trim().to_string());
                    current_text.clear();
                }

                if level == HeadingLevel::H1 {
                    current_section = Some("__title__".to_string());
                } else if level == HeadingLevel::H2 {
                    current_section = Some(String::new());
                }
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                name = current_text.trim().to_string();
                current_text.clear();
                current_section = None;
            }
            Event::End(TagEnd::Heading(HeadingLevel::H2)) => {
                let heading = current_text.trim().to_lowercase();
                current_text.clear();
                current_section = Some(heading);
            }
            Event::Text(text) | Event::Code(text) => {
                current_text.push_str(&text);
            }
            Event::SoftBreak | Event::HardBreak => {
                current_text.push('\n');
            }
            Event::End(TagEnd::Item | TagEnd::Paragraph) => {
                current_text.push('\n');
            }
            _ => {}
        }
    }

    // Flush last section
    if let Some(ref section) = current_section {
        sections.insert(section.clone(), current_text.trim().to_string());
    }

    if name.is_empty() {
        bail!("Agent file {} is missing an H1 title", path.display());
    }

    let metadata_raw = sections
        .get("metadata")
        .context("Missing ## Metadata section")?;
    let metadata = super::metadata::parse_metadata(metadata_raw)?;

    let system_prompt = sections
        .get("system prompt")
        .context("Missing ## System Prompt section")?
        .clone();

    let instructions = sections.get("instructions").cloned();
    let output_format = sections.get("output format").cloned();
    let context = sections.get("context").cloned();

    let pipeline = sections.get("pipeline").map(|raw| {
        let next: Vec<String> = raw
            .lines()
            .filter_map(|l| {
                let trimmed = l.trim().trim_start_matches('-').trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect();
        PipelineConfig { next }
    });

    Ok(Agent {
        name,
        source: path.to_path_buf(),
        metadata,
        system_prompt,
        instructions,
        output_format,
        pipeline,
        context,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_agent(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::with_suffix(".md").unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parse_basic_agent() {
        let f = write_temp_agent(
            r#"# Test Agent

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.5
- tags: [dev, test]
- stacks: [rust]

## System Prompt

You are a test agent.

## Instructions

Do the thing.
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        assert_eq!(agent.name, "Test Agent");
        assert_eq!(agent.metadata.provider, "anthropic");
        assert_eq!(
            agent.metadata.model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert!((agent.metadata.temperature - 0.5).abs() < f32::EPSILON);
        assert_eq!(agent.metadata.tags, vec!["dev", "test"]);
        assert_eq!(agent.metadata.stacks, vec!["rust"]);
        assert_eq!(agent.system_prompt, "You are a test agent.");
        assert_eq!(agent.instructions.as_deref(), Some("Do the thing."));
        assert!(agent.output_format.is_none());
        assert!(agent.pipeline.is_none());
    }

    #[test]
    fn parse_cli_agent() {
        let f = write_temp_agent(
            r#"# CLI Agent

## Metadata
- provider: cli
- command: echo
- args: [hello, world]
- timeout: 60

## System Prompt

You are a cli wrapper.
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        assert_eq!(agent.name, "CLI Agent");
        assert_eq!(agent.metadata.provider, "cli");
        assert_eq!(agent.metadata.command.as_deref(), Some("echo"));
        assert_eq!(
            agent.metadata.args.as_deref(),
            Some(&["hello".to_string(), "world".to_string()][..])
        );
        assert_eq!(agent.metadata.timeout, Some(60));
    }

    #[test]
    fn parse_missing_title_fails() {
        let f = write_temp_agent(
            r#"## Metadata
- provider: anthropic

## System Prompt

test
"#,
        );
        assert!(parse_agent_file(f.path()).is_err());
    }

    #[test]
    fn parse_missing_metadata_fails() {
        let f = write_temp_agent(
            r#"# Agent

## System Prompt

test
"#,
        );
        assert!(parse_agent_file(f.path()).is_err());
    }

    #[test]
    fn parse_missing_system_prompt_fails() {
        let f = write_temp_agent(
            r#"# Agent

## Metadata
- provider: anthropic
- model: test
"#,
        );
        assert!(parse_agent_file(f.path()).is_err());
    }

    #[test]
    fn parse_agent_with_pipeline() {
        let f = write_temp_agent(
            r#"# Pipeline Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

test

## Pipeline
- agent-b
- agent-c
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let pipeline = agent.pipeline.unwrap();
        assert_eq!(pipeline.next, vec!["agent-b", "agent-c"]);
    }

    #[test]
    fn parse_real_agent_files() {
        let agents_dir = Path::new("agents");
        if !agents_dir.exists() {
            return;
        }
        let agents = crate::core::agent::Agent::load_all(agents_dir).unwrap();
        assert!(
            !agents.is_empty(),
            "Should parse at least one agent from agents/"
        );
        for agent in &agents {
            assert!(!agent.name.is_empty());
            assert!(!agent.metadata.provider.is_empty());
            assert!(!agent.system_prompt.is_empty());
        }
    }
}
