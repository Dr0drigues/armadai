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
        let next: Vec<String> = raw.lines().filter_map(|l| {
            let trimmed = l.trim().trim_start_matches('-').trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        }).collect();
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
