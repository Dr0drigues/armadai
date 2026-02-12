use std::path::Path;

use anyhow::{Context, bail};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::core::agent::{Agent, PipelineConfig};

/// Parse a Markdown agent definition file into an Agent struct.
///
/// Uses pulldown-cmark offset iterator to identify section boundaries, then
/// slices the raw Markdown content so that formatting (bold, lists, etc.) is
/// preserved verbatim.
pub fn parse_agent_file(path: &Path) -> anyhow::Result<Agent> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    parse_agent_content(&content, path)
}

/// Inner parser that works on a content string (testable without files).
fn parse_agent_content(content: &str, path: &Path) -> anyhow::Result<Agent> {
    // Collect section boundaries: (level, heading_text, heading_byte_start, content_byte_start)
    let mut boundaries: Vec<(HeadingLevel, String, usize, usize)> = Vec::new();
    let mut in_heading = false;
    let mut heading_level = HeadingLevel::H1;
    let mut heading_start = 0usize;
    let mut heading_name = String::new();

    let parser = Parser::new(content).into_offset_iter();

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = true;
                heading_level = level;
                heading_start = range.start;
                heading_name.clear();
            }
            Event::Text(text) if in_heading => {
                heading_name.push_str(&text);
            }
            Event::Code(text) if in_heading => {
                heading_name.push_str(&text);
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                boundaries.push((
                    heading_level,
                    heading_name.trim().to_string(),
                    heading_start,
                    range.end,
                ));
            }
            _ => {}
        }
    }

    // Extract name from H1
    let name = boundaries
        .iter()
        .find(|(level, ..)| *level == HeadingLevel::H1)
        .map(|(_, n, ..)| n.clone())
        .unwrap_or_default();

    if name.is_empty() {
        bail!("Agent file {} is missing an H1 title", path.display());
    }

    // Build section map: for each H2, extract raw markdown from after its heading
    // to the start of the next heading (any level).
    let mut sections: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (i, (level, heading_text, _heading_start, content_start)) in boundaries.iter().enumerate() {
        if *level != HeadingLevel::H2 {
            continue;
        }

        // Section content ends at the start of the next heading (any level)
        let section_end = boundaries
            .get(i + 1)
            .map(|(_, _, hs, _)| *hs)
            .unwrap_or(content.len());

        let raw = content[*content_start..section_end].trim().to_string();
        sections.insert(heading_text.to_lowercase(), raw);
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
    fn parse_preserves_markdown_formatting() {
        let f = write_temp_agent(
            r#"# Formatted Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

You inspect code for issues:

- **Bugs** — logic errors, edge cases
- **Security** — injections, data leaks
- **Performance** — N+1 queries, allocations

Use `grep` to search and **bold** for emphasis.

## Instructions

1. Read the code carefully
2. Classify each finding by severity
3. Propose a concrete fix
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();

        // Markdown list markers preserved
        assert!(agent.system_prompt.contains("- **Bugs**"));
        assert!(agent.system_prompt.contains("- **Security**"));
        assert!(agent.system_prompt.contains("- **Performance**"));

        // Inline code and bold preserved
        assert!(agent.system_prompt.contains("`grep`"));
        assert!(agent.system_prompt.contains("**bold**"));

        // Numbered list in instructions preserved
        let instructions = agent.instructions.unwrap();
        assert!(instructions.contains("1. Read"));
        assert!(instructions.contains("2. Classify"));
        assert!(instructions.contains("3. Propose"));
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
