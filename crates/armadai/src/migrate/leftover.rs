//! Text of a legacy agent `.md` that the legacy parser does not read.
//!
//! The parser keeps only the H1 and the sections it knows; `migrate` must never lose
//! text, so everything else is found here and appended to the agent's body.

/// What the legacy parser left out of one agent file.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Leftover {
    /// (label, verbatim text), in source order.
    pub kept: Vec<(String, String)>,
    /// `## Metadata` entries the legacy parser ignores (key, or the whole line if it has none).
    pub unknown_metadata: Vec<String>,
}

/// H2 titles the legacy parser reads (lowercase, as it compares them).
const KNOWN_SECTIONS: [&str; 8] = [
    "metadata",
    "system prompt",
    "instructions",
    "output format",
    "context",
    "pipeline",
    "triggers",
    "ring config",
];

/// `## Metadata` keys the legacy parser reads (`armadai_core::parser::metadata`).
const KNOWN_METADATA: [&str; 17] = [
    "provider",
    "model",
    "command",
    "args",
    "temperature",
    "max_tokens",
    "timeout",
    "tags",
    "stacks",
    "scope",
    "model_fallback",
    "model_fallbacks",
    "cost_limit",
    "rate_limit",
    "context_window",
    "mode",
    "orchestration",
];

pub fn leftover(content: &str) -> Leftover {
    use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

    // (level, title, heading start, content start) of every H1/H2, as the legacy parser
    // sees them: same Markdown parser, so fenced code and setext headings agree.
    let mut heads: Vec<(HeadingLevel, String, usize, usize)> = Vec::new();
    let mut current: Option<(HeadingLevel, String, usize)> = None;
    for (event, range) in Parser::new(content).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current = Some((level, String::new(), range.start))
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, title, _)) = current.as_mut() {
                    title.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, title, start)) = current.take()
                    && level <= HeadingLevel::H2
                {
                    heads.push((level, title.trim().to_string(), start, range.end));
                }
            }
            _ => {}
        }
    }

    let mut l = Leftover::default();
    let end_of = |i: usize| heads.get(i + 1).map_or(content.len(), |h| h.2);
    let text = |a: usize, b: usize| content[a..b].trim().replace("\r\n", "\n");
    let before = heads.first().map_or(content.len(), |h| h.2);
    if !text(0, before).is_empty() {
        l.kept
            .push(("text before the title".into(), text(0, before)));
    }
    let first_h1 = heads.iter().position(|h| h.0 == HeadingLevel::H1);
    for (i, (level, title, start, body)) in heads.iter().enumerate() {
        let lower = title.to_lowercase();
        if Some(i) == first_h1 {
            if !text(*body, end_of(i)).is_empty() {
                l.kept.push(("preamble".into(), text(*body, end_of(i))));
            }
        } else if *level == HeadingLevel::H1 || !KNOWN_SECTIONS.contains(&lower.as_str()) {
            let mark = if *level == HeadingLevel::H1 {
                "#"
            } else {
                "##"
            };
            l.kept
                .push((format!("{mark} {title}"), text(*start, end_of(i))));
        } else if heads[i + 1..]
            .iter()
            .any(|h| h.0 == HeadingLevel::H2 && h.1.to_lowercase() == lower)
        {
            // the legacy parser keeps the last section of a given title only
            l.kept.push((
                format!("## {title} (shadowed by a later one)"),
                text(*start, end_of(i)),
            ));
        } else if lower == "metadata" {
            for line in content[*body..end_of(i)].lines() {
                let line = line.trim().trim_start_matches('-').trim();
                if line.is_empty() {
                    continue;
                }
                let entry = match line.split_once(':') {
                    Some((k, _)) => k.trim().to_lowercase(),
                    None => line.to_string(),
                };
                if !KNOWN_METADATA.contains(&entry.as_str()) {
                    l.unknown_metadata.push(entry);
                }
            }
        }
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(l: &Leftover) -> Vec<&str> {
        l.kept.iter().map(|(k, _)| k.as_str()).collect()
    }

    const FULL: &str = "\
Before the title.

# Dev

A preamble line.

### Preamble sub-heading
Still preamble.

## Metadata
- provider: anthropic
- flavour: spicy
- just a note

## System Prompt

You code.

### Phase 1
Inside the system prompt.

## Delegation Protocol

Delegate.

```md
## Not a heading
```

### Sub
Still delegation.

## Instructions

First instructions.

## Pipeline
- next

## Examples

An example.

## instructions

Second instructions win.
";

    #[test]
    fn unknown_sections_preamble_and_shadowed_duplicates_are_kept_in_order() {
        let l = leftover(FULL);
        assert_eq!(
            labels(&l),
            [
                "text before the title",
                "preamble",
                "## Delegation Protocol",
                "## Instructions (shadowed by a later one)",
                "## Examples",
            ]
        );
        assert_eq!(l.kept[0].1, "Before the title.");
        assert_eq!(
            l.kept[1].1,
            "A preamble line.\n\n### Preamble sub-heading\nStill preamble."
        );
        assert_eq!(
            l.kept[2].1,
            "## Delegation Protocol\n\nDelegate.\n\n```md\n## Not a heading\n```\n\n### Sub\nStill delegation."
        );
        assert_eq!(l.kept[3].1, "## Instructions\n\nFirst instructions.");
    }

    #[test]
    fn unknown_metadata_entries_are_listed() {
        assert_eq!(leftover(FULL).unknown_metadata, ["flavour", "just a note"]);
    }

    #[test]
    fn a_file_with_only_known_sections_leaves_nothing() {
        let l = leftover(
            "# Dev\n\n## Metadata\n- provider: x\n- model_fallbacks: [a]\n\n## System Prompt\n\nHi.\n\n### Sub\nx\n\n## Output Format\n\nY\n",
        );
        assert_eq!(l, Leftover::default());
    }

    #[test]
    fn setext_and_crlf_headings_are_seen() {
        let l = leftover(
            "# Dev\r\n\r\n## Metadata\r\n- provider: x\r\n\r\nNotes\r\n-----\r\n\r\nN.\r\n",
        );
        assert_eq!(labels(&l), ["## Notes"]);
    }
}
