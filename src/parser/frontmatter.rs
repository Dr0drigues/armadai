/// Extract YAML frontmatter (between `---` delimiters) and the remaining body.
///
/// Returns `(frontmatter_yaml, body)`. If no frontmatter is found, returns
/// `(None, full_content)`.
pub fn extract_frontmatter(content: &str) -> (Option<&str>, &str) {
    // Frontmatter must start at the very beginning of the file
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content);
    }

    // Find the opening delimiter line end
    let after_open = match trimmed[3..].find('\n') {
        Some(pos) => 3 + pos + 1,
        None => return (None, content), // only `---` with nothing after
    };

    // Check that the opening line is just `---` (possibly with trailing whitespace)
    if !trimmed[3..after_open].trim().is_empty() {
        return (None, content);
    }

    // Find the closing `---`
    let rest = &trimmed[after_open..];
    for (i, line) in rest.lines().enumerate() {
        if line.trim() == "---" {
            let fm_end = after_open + rest.lines().take(i).map(|l| l.len() + 1).sum::<usize>();
            let frontmatter = &trimmed[after_open..fm_end];
            let body_start = fm_end + line.len();
            // Skip the optional newline right after closing ---
            let body = if trimmed[body_start..].starts_with('\n') {
                &trimmed[body_start + 1..]
            } else {
                &trimmed[body_start..]
            };
            let fm = frontmatter.trim();
            if fm.is_empty() {
                return (None, body);
            }
            return (Some(fm), body);
        }
    }

    // No closing delimiter found — treat as no frontmatter
    (None, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_frontmatter() {
        let content = "---\nname: test\ndescription: hello\n---\nBody content here.";
        let (fm, body) = extract_frontmatter(content);
        assert_eq!(fm, Some("name: test\ndescription: hello"));
        assert_eq!(body, "Body content here.");
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "Just a regular markdown file.\nNo frontmatter.";
        let (fm, body) = extract_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_empty_frontmatter() {
        let content = "---\n---\nBody after empty frontmatter.";
        let (fm, body) = extract_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, "Body after empty frontmatter.");
    }

    #[test]
    fn test_frontmatter_with_dashes_in_body() {
        let content = "---\nname: test\n---\nSome text.\n---\nMore text.";
        let (fm, body) = extract_frontmatter(content);
        assert_eq!(fm, Some("name: test"));
        assert_eq!(body, "Some text.\n---\nMore text.");
    }

    #[test]
    fn test_no_closing_delimiter() {
        let content = "---\nname: test\nno closing delimiter";
        let (fm, body) = extract_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_multiline_frontmatter() {
        let content = "---\nname: my-prompt\ndescription: A useful prompt\napply_to:\n  - agent-a\n  - agent-b\n---\n# Body\n\nContent.";
        let (fm, body) = extract_frontmatter(content);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(fm.contains("name: my-prompt"));
        assert!(fm.contains("apply_to:"));
        assert_eq!(body, "# Body\n\nContent.");
    }

    #[test]
    fn test_empty_content() {
        let (fm, body) = extract_frontmatter("");
        assert!(fm.is_none());
        assert_eq!(body, "");
    }

    #[test]
    fn test_only_dashes() {
        let (fm, body) = extract_frontmatter("---");
        assert!(fm.is_none());
        assert_eq!(body, "---");
    }
}
