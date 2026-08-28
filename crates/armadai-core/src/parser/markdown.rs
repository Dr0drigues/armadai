use std::path::Path;

use anyhow::{Context, bail};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use super::metadata::{config_line, warn_duplicate_keys};
use crate::agent::{Agent, PipelineConfig};
use crate::orchestration::{AgentRingConfig, TriggerConfig};

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

    // Build section map: for each H2, extract raw markdown from after its
    // heading to the start of the next heading of level <= H2 (#392).
    let mut sections: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (i, (level, heading_text, _heading_start, content_start)) in boundaries.iter().enumerate() {
        if *level != HeadingLevel::H2 {
            continue;
        }

        // A `##` section OWNS its sub-sections: it ends at the next heading
        // of level <= H2 (another `##`, or a new `#`), not at the next
        // heading of any level. An `###`/`####`/... is a sub-heading INSIDE
        // the section, so everything from it up to the next `#`/`##` stays
        // part of the section.
        //
        // Ending at "the next heading of any level" (the behaviour up to
        // #392) truncated every agent whose prompt used sub-headings: on the
        // shipped `debug.md` template, `## Instructions` parsed as the empty
        // string because all four `### Phase N` blocks were cut off; on a
        // 3833-byte `agent-builder.md`, `## System Prompt` parsed to 211
        // characters. `link` writes these fields verbatim and `run` sends
        // `system_prompt` to the provider, so the truncation reached the
        // model, silently and with no parse error.
        let section_end = boundaries[i + 1..]
            .iter()
            .find(|(lvl, ..)| *lvl <= HeadingLevel::H2)
            .map(|(_, _, hs, _)| *hs)
            .unwrap_or(content.len());

        let raw = content[*content_start..section_end].trim().to_string();
        sections.insert(heading_text.to_lowercase(), raw);
    }

    let metadata_raw = sections
        .get("metadata")
        .context("Missing ## Metadata section")?;
    let mut metadata = super::metadata::parse_metadata(metadata_raw, path)?;

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
                // Heading lines are structure, not agent names. A `##`
                // section now keeps its sub-headings (#392), so a stray
                // `### Phase two` inside a `## Pipeline` block would
                // otherwise be handed downstream as an agent name and fail
                // to resolve.
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .collect();
        PipelineConfig { next }
    });

    // Parse ## Triggers section (for Blackboard agents)
    if let Some(raw) = sections.get("triggers") {
        metadata.triggers = Some(parse_trigger_config(raw, path));
    }

    // Parse ## Ring Config section (for Ring agents)
    if let Some(raw) = sections.get("ring config") {
        metadata.ring_config = Some(parse_ring_config(raw, path));
    }

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

/// The canonical field a `## Triggers` key sets, or `None` when the parser
/// ignores it. Mirrors the `match` below — see [`metadata_field`] for why the
/// list is kept next to the parser that owns it.
///
/// [`metadata_field`]: super::metadata
fn trigger_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "requires" => "requires",
        "excludes" => "excludes",
        "min_round" => "min_round",
        "max_round" => "max_round",
        "priority" => "priority",
        _ => return None,
    })
}

/// Parse a ## Triggers section into TriggerConfig.
///
/// `source` only names the file in the duplicate-key warning (#396).
fn parse_trigger_config(raw: &str, source: &Path) -> TriggerConfig {
    warn_duplicate_keys(raw, source, "Triggers", trigger_field);

    let mut requires = Vec::new();
    let mut excludes = Vec::new();
    let mut min_round = 0u32;
    let mut max_round = None;
    let mut priority = 50u8;

    for line in raw.lines() {
        let Some((key, value)) = config_line(line) else {
            continue;
        };

        match key.as_str() {
            "requires" => requires = parse_string_list_inline(value),
            "excludes" => excludes = parse_string_list_inline(value),
            "min_round" => min_round = value.parse().unwrap_or(0),
            "max_round" => max_round = value.parse().ok(),
            "priority" => priority = value.parse().unwrap_or(50),
            _ => {}
        }
    }

    TriggerConfig {
        requires,
        excludes,
        min_round,
        max_round,
        priority,
    }
}

/// The canonical field a `## Ring Config` key sets, or `None` when the parser
/// ignores it.
fn ring_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "role" => "role",
        "position" => "position",
        "vote_weight" => "vote_weight",
        _ => return None,
    })
}

/// Parse a ## Ring Config section into AgentRingConfig.
///
/// `source` only names the file in the duplicate-key warning (#396).
fn parse_ring_config(raw: &str, source: &Path) -> AgentRingConfig {
    warn_duplicate_keys(raw, source, "Ring Config", ring_field);

    let mut role = "specialist".to_string();
    let mut position = None;
    let mut vote_weight = 1.0f32;

    for line in raw.lines() {
        let Some((key, value)) = config_line(line) else {
            continue;
        };

        match key.as_str() {
            "role" => role = value.to_string(),
            "position" => position = value.parse().ok(),
            "vote_weight" => vote_weight = value.parse().unwrap_or(1.0),
            _ => {}
        }
    }

    AgentRingConfig {
        role,
        position,
        vote_weight,
    }
}

/// Parse bracket-delimited list inline (reused for trigger fields).
fn parse_string_list_inline(value: &str) -> Vec<String> {
    let trimmed = value.trim().trim_start_matches('[').trim_end_matches(']');
    trimmed
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
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

    // ── #392: an H2 section owns its H3+ sub-sections ────────────────────
    //
    // `parse_agent_content` used to end an H2 section at the *next heading
    // of any level*, so the first `###` inside `## System Prompt` truncated
    // the prompt. Measured before the fix on the shipped `debug.md` template:
    // `## Instructions` came out empty, and `agent-builder.md` went from
    // 3833 source bytes to 1717 linked bytes. The tests below pin the
    // boundary at "next heading of level <= H2".

    #[test]
    fn h2_section_keeps_its_h3_subsections() {
        let f = write_temp_agent(
            r#"# Nested Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

An agent file has these required sections:

### Required Structure

- `# Name` — the H1
- `## Metadata` — the fields

### Optional Sections

- `## Instructions`
- `## Output Format`

## Instructions

Do the thing.
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();

        // The intro survives (it always did).
        assert!(
            agent
                .system_prompt
                .contains("An agent file has these required sections:"),
            "intro missing from system_prompt: {:?}",
            agent.system_prompt
        );
        // The sub-sections the intro announces survive too (they did not).
        assert!(
            agent.system_prompt.contains("### Required Structure"),
            "H3 sub-heading truncated away: {:?}",
            agent.system_prompt
        );
        assert!(
            agent.system_prompt.contains("`# Name` — the H1"),
            "H3 body truncated away: {:?}",
            agent.system_prompt
        );
        assert!(
            agent.system_prompt.contains("### Optional Sections"),
            "second H3 sub-heading truncated away: {:?}",
            agent.system_prompt
        );
        assert!(
            agent.system_prompt.contains("- `## Output Format`"),
            "last line of the last sub-section truncated away: {:?}",
            agent.system_prompt
        );

        // And the section still STOPS at the next H2: no leak downwards.
        assert!(
            !agent.system_prompt.contains("Do the thing."),
            "next H2's content leaked into system_prompt: {:?}",
            agent.system_prompt
        );
        assert_eq!(agent.instructions.as_deref(), Some("Do the thing."));
    }

    /// The same defect hit every `##` section, not just System Prompt. The
    /// shipped `debug.md` template puts all of its content under
    /// `### Phase N` sub-headings, so `## Instructions` parsed as the empty
    /// string; `planning.md`/`tech-debt.md`/`security-review.md` do the same.
    #[test]
    fn h3_subsections_survive_in_instructions_and_output_format() {
        let f = write_temp_agent(
            r#"# Phased Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

You debug systematically.

## Instructions

### Phase 1: Assessment
1. Read the error message
2. Reproduce the mental model

### Phase 2: Investigation
1. Trace the execution path

## Output Format

### Root Cause
<One sentence>

### Evidence
- Expected: <what should happen>
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();

        let instructions = agent.instructions.expect("## Instructions section");
        assert!(
            instructions.contains("### Phase 1: Assessment"),
            "Phase 1 heading lost: {instructions:?}"
        );
        assert!(
            instructions.contains("1. Read the error message"),
            "Phase 1 body lost: {instructions:?}"
        );
        assert!(
            instructions.contains("### Phase 2: Investigation"),
            "Phase 2 heading lost: {instructions:?}"
        );
        assert!(
            instructions.contains("1. Trace the execution path"),
            "Phase 2 body lost: {instructions:?}"
        );
        // Instructions still stop at the next H2.
        assert!(
            !instructions.contains("### Root Cause"),
            "Output Format leaked into Instructions: {instructions:?}"
        );

        let output_format = agent.output_format.expect("## Output Format section");
        assert!(
            output_format.contains("### Root Cause"),
            "Root Cause heading lost: {output_format:?}"
        );
        assert!(
            output_format.contains("### Evidence"),
            "Evidence heading lost: {output_format:?}"
        );
        assert!(
            output_format.contains("- Expected: <what should happen>"),
            "last line of the last sub-section lost: {output_format:?}"
        );
    }

    /// `## Metadata` is parsed field-by-field, so the truncation silently
    /// dropped every field declared after a sub-heading — the agent then ran
    /// with default temperature, no tags, no model. The new boundary keeps
    /// them, and `parse_metadata` skips the `###` line itself (no colon).
    #[test]
    fn metadata_fields_after_an_h3_subsection_are_parsed() {
        let f = write_temp_agent(
            r#"# Grouped Metadata Agent

## Metadata

### Provider
- provider: anthropic
- model: claude-sonnet-4-5-20250929

### Tuning
- temperature: 0.9
- max_tokens: 4096
- tags: [dev, deep]

## System Prompt

test
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        assert_eq!(agent.metadata.provider, "anthropic");
        assert_eq!(
            agent.metadata.model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert!(
            (agent.metadata.temperature - 0.9).abs() < f32::EPSILON,
            "temperature declared under a second H3 was dropped: {}",
            agent.metadata.temperature
        );
        assert_eq!(agent.metadata.max_tokens, Some(4096));
        assert_eq!(agent.metadata.tags, vec!["dev", "deep"]);
    }

    /// The boundary is "level <= H2", so H4/H5/H6 stay inside their section
    /// exactly like H3 — a nested outline is not a section terminator.
    #[test]
    fn h4_and_deeper_headings_stay_inside_their_h2_section() {
        let f = write_temp_agent(
            r#"# Deep Outline Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

Top level text.

### Level three

Three body.

#### Level four

Four body.

##### Level five

Five body.

###### Level six

Six body.

## Instructions

Next section.
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        for needle in [
            "### Level three",
            "Three body.",
            "#### Level four",
            "Four body.",
            "##### Level five",
            "Five body.",
            "###### Level six",
            "Six body.",
        ] {
            assert!(
                agent.system_prompt.contains(needle),
                "{needle:?} lost from system_prompt: {:?}",
                agent.system_prompt
            );
        }
        assert!(
            !agent.system_prompt.contains("Next section."),
            "next H2 leaked in: {:?}",
            agent.system_prompt
        );
    }

    /// A later H1 is level 1, so it IS <= H2 and must still close the
    /// section — a document that starts a second top-level part must not
    /// have that part swallowed into the previous `##`.
    #[test]
    fn an_h2_section_ends_at_a_later_h1() {
        let f = write_temp_agent(
            r#"# First Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

Owned by the first H1.

### A sub-section

Still owned by the first H1.

# Appendix

Not part of the system prompt.
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        assert!(
            agent.system_prompt.contains("### A sub-section"),
            "sub-section lost: {:?}",
            agent.system_prompt
        );
        assert!(
            agent.system_prompt.contains("Still owned by the first H1."),
            "sub-section body lost: {:?}",
            agent.system_prompt
        );
        assert!(
            !agent
                .system_prompt
                .contains("Not part of the system prompt."),
            "content after a later H1 leaked into system_prompt: {:?}",
            agent.system_prompt
        );
        assert!(
            !agent.system_prompt.contains("Appendix"),
            "a later H1 heading leaked into system_prompt: {:?}",
            agent.system_prompt
        );
    }

    /// Degenerate shapes the new boundary must survive: an `###` before the
    /// first `##` (owned by no section), an `###` as the very last thing in
    /// the file (no following boundary at all), and two `##` in a row with
    /// nothing between them (empty section, not a panic or a slice inversion).
    #[test]
    fn degenerate_heading_shapes_parse_sanely() {
        let f = write_temp_agent(
            r#"# Edge Agent

### Orphan sub-section before any H2

Orphan body.

## Metadata
- provider: anthropic
- model: test

## Context
## System Prompt

Prompt intro.

### Trailing sub-section at end of file

Trailing body.
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();

        // The orphan `###` belongs to no `##` section: it must not be
        // adopted by Metadata (which comes after it) nor by System Prompt.
        assert!(
            !agent.system_prompt.contains("Orphan body."),
            "orphan pre-H2 content leaked into system_prompt: {:?}",
            agent.system_prompt
        );

        // Two consecutive H2 with nothing between: empty, not missing.
        assert_eq!(
            agent.context.as_deref(),
            Some(""),
            "an empty `## Context` must parse as an empty string"
        );

        // Trailing `###` with no following boundary: runs to end of file.
        assert!(
            agent
                .system_prompt
                .contains("### Trailing sub-section at end of file"),
            "trailing sub-heading lost: {:?}",
            agent.system_prompt
        );
        assert!(
            agent.system_prompt.contains("Trailing body."),
            "trailing sub-section body lost: {:?}",
            agent.system_prompt
        );
    }

    /// `## Pipeline` reads every non-empty line as an agent name. Now that a
    /// `###` no longer terminates the section, a stray sub-heading inside a
    /// Pipeline block would otherwise be handed downstream as an agent named
    /// `### Phase two` and fail to resolve. Heading lines are skipped.
    #[test]
    fn pipeline_section_ignores_heading_lines() {
        let f = write_temp_agent(
            r#"# Pipeline Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

test

## Pipeline
- agent-b

### Phase two
- agent-c
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let pipeline = agent.pipeline.unwrap();
        assert_eq!(
            pipeline.next,
            vec!["agent-b", "agent-c"],
            "a heading line must not be read as an agent name"
        );
    }

    #[test]
    fn parse_real_agent_files() {
        let agents_dir = Path::new("agents");
        if !agents_dir.exists() {
            return;
        }
        let agents = crate::agent::Agent::load_all(agents_dir).unwrap();
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

    #[test]
    fn parse_triggers_section() {
        let f = write_temp_agent(
            r#"# Trigger Agent

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- orchestration: blackboard

## System Prompt

You react to findings.

## Triggers
- requires: [finding, question]
- excludes: [synthesis]
- min_round: 1
- max_round: 4
- priority: 75
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let triggers = agent.metadata.triggers.unwrap();
        assert_eq!(triggers.requires, vec!["finding", "question"]);
        assert_eq!(triggers.excludes, vec!["synthesis"]);
        assert_eq!(triggers.min_round, 1);
        assert_eq!(triggers.max_round, Some(4));
        assert_eq!(triggers.priority, 75);
    }

    #[test]
    fn parse_triggers_section_defaults() {
        let f = write_temp_agent(
            r#"# Trigger Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

test

## Triggers
- requires: [finding]
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let triggers = agent.metadata.triggers.unwrap();
        assert_eq!(triggers.requires, vec!["finding"]);
        assert!(triggers.excludes.is_empty());
        assert_eq!(triggers.min_round, 0);
        assert!(triggers.max_round.is_none());
        assert_eq!(triggers.priority, 50);
    }

    /// The same table-completeness check `## Metadata` gets, for the two
    /// sections whose tables live here.
    ///
    /// Measured before this existed: dropping `excludes`, `min_round` and
    /// `max_round` from `trigger_field` left the whole suite green, and so did
    /// dropping `position` from `ring_field`. Three of five trigger spellings
    /// and two of three ring spellings were exercised by any fixture; the rest
    /// were promised only by a comment.
    #[test]
    fn every_trigger_and_ring_key_the_tables_know_is_reported_when_set_twice() {
        for (section, keys, field) in [
            (
                "Triggers",
                ["requires", "excludes", "min_round", "max_round", "priority"].as_slice(),
                trigger_field as fn(&str) -> Option<&'static str>,
            ),
            (
                "Ring Config",
                ["role", "position", "vote_weight"].as_slice(),
                ring_field as fn(&str) -> Option<&'static str>,
            ),
        ] {
            for key in keys {
                let raw = format!("- {key}: first\n- {key}: second\n");
                let found = super::super::metadata::duplicate_keys(&raw, field);
                assert_eq!(
                    found.len(),
                    1,
                    "## {section} knows '{key}' but setting it twice reported {found:?}"
                );
            }
        }
    }

    /// Control for the loop above: a key neither table knows must stay silent,
    /// so the loop cannot be satisfied by a table that says yes to everything.
    #[test]
    fn a_key_neither_table_knows_is_never_reported() {
        for key in ["unrelated", "note", "owner"] {
            let raw = format!("- {key}: first\n- {key}: second\n");
            assert!(
                super::super::metadata::duplicate_keys(&raw, trigger_field).is_empty(),
                "## Triggers does not read '{key}'"
            );
            assert!(
                super::super::metadata::duplicate_keys(&raw, ring_field).is_empty(),
                "## Ring Config does not read '{key}'"
            );
        }
    }

    /// #396: `## Triggers` and `## Ring Config` overwrite in silence exactly
    /// like `## Metadata`, and a `###` sub-block inside them is read since
    /// #392 — so a "not in use" alternative changes Blackboard activation and
    /// Ring vote weights. The printing of these warnings is measured on the
    /// binary in `crates/armadai/tests/duplicate_metadata_key_warns.rs`.
    #[test]
    fn duplicate_trigger_keys_are_reported_with_their_values() {
        let raw = "\
- requires: [alpha]
- priority: 10

### Alternative triggers (not in use)
- requires: [beta]
- priority: 99
- unrelated: repeated
- unrelated: again
";
        assert_eq!(
            super::super::metadata::duplicate_keys(raw, trigger_field),
            vec![
                ("requires", "[alpha]".to_string(), "[beta]".to_string()),
                ("priority", "10".to_string(), "99".to_string()),
            ]
        );
    }

    #[test]
    fn duplicate_ring_keys_are_reported_with_their_values() {
        let raw = "\
- role: specialist
- vote_weight: 1.0

### Alternative ring (not in use)
- role: coordinator
- vote_weight: 9.0
";
        assert_eq!(
            super::super::metadata::duplicate_keys(raw, ring_field),
            vec![
                ("role", "specialist".to_string(), "coordinator".to_string()),
                ("vote_weight", "1.0".to_string(), "9.0".to_string()),
            ]
        );
    }

    /// The precedence in both sections is untouched: the last value still wins,
    /// through the real `parse_agent_file` path.
    #[test]
    fn a_duplicated_trigger_and_ring_key_still_lets_the_last_value_win() {
        let f = write_temp_agent(
            r#"# Dup Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

test

## Triggers
- requires: [alpha]
- priority: 10

### Alternative triggers (not in use)
- requires: [beta]
- priority: 99

## Ring Config
- role: specialist
- vote_weight: 1.0

### Alternative ring (not in use)
- role: coordinator
- vote_weight: 9.0
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let triggers = agent.metadata.triggers.unwrap();
        assert_eq!(triggers.requires, vec!["beta"]);
        assert_eq!(triggers.priority, 99);
        let ring = agent.metadata.ring_config.unwrap();
        assert_eq!(ring.role, "coordinator");
        assert!((ring.vote_weight - 9.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_ring_config_section() {
        let f = write_temp_agent(
            r#"# Ring Agent

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- orchestration: ring

## System Prompt

You participate in ring reviews.

## Ring Config
- role: challenger
- position: 2
- vote_weight: 1.5
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let ring = agent.metadata.ring_config.unwrap();
        assert_eq!(ring.role, "challenger");
        assert_eq!(ring.position, Some(2));
        assert!((ring.vote_weight - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_ring_config_section_defaults() {
        let f = write_temp_agent(
            r#"# Ring Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

test

## Ring Config
- role: synthesizer
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        let ring = agent.metadata.ring_config.unwrap();
        assert_eq!(ring.role, "synthesizer");
        assert!(ring.position.is_none());
        assert!((ring.vote_weight - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_no_triggers_or_ring_config() {
        let f = write_temp_agent(
            r#"# Plain Agent

## Metadata
- provider: anthropic
- model: test

## System Prompt

test
"#,
        );
        let agent = parse_agent_file(f.path()).unwrap();
        assert!(agent.metadata.triggers.is_none());
        assert!(agent.metadata.ring_config.is_none());
    }
}
