//! Marker parser for ArmadAI response protocol
//!
//! Parses HTML comment markers in LLM responses to extract:
//! - End-of-response signal (<!--ARMADAI_END-->)
//! - Delegation instructions (<!--ARMADAI_DELEGATE:agent-name-->)
//! - Metadata key-value pairs (<!--ARMADAI_META:key1=value1,key2=value2-->)

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Compiled regex patterns for marker detection
static END_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--ARMADAI_END-->").expect("valid regex"));

static DELEGATE_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--ARMADAI_DELEGATE:(.+?)-->").expect("valid regex"));

static META_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!--ARMADAI_META:(.+?)-->").expect("valid regex"));

/// A parsed LLM response with markers extracted
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedResponse {
    /// The clean content without markers
    pub content: String,
    /// Whether the END marker was found
    pub complete: bool,
    /// Delegations found (agent names)
    pub delegations: Vec<String>,
    /// Metadata key-value pairs
    pub metadata: HashMap<String, String>,
}

/// Parse a raw LLM response and extract markers
///
/// # Example
///
/// ```
/// use armadai::shell::parser::parse_response;
///
/// let raw = r#"
/// Here is my response.
///
/// <!--ARMADAI_META:status=complete-->
/// <!--ARMADAI_END-->
/// "#;
///
/// let parsed = parse_response(raw);
/// assert!(parsed.complete);
/// assert_eq!(parsed.metadata.get("status"), Some(&"complete".to_string()));
/// ```
pub fn parse_response(raw: &str) -> ParsedResponse {
    let mut content = raw.to_string();
    let mut delegations = Vec::new();
    let mut metadata = HashMap::new();

    // Extract delegations
    for cap in DELEGATE_MARKER.captures_iter(raw) {
        if let Some(agent) = cap.get(1) {
            delegations.push(agent.as_str().trim().to_string());
        }
    }

    // Extract metadata
    for cap in META_MARKER.captures_iter(raw) {
        if let Some(meta_str) = cap.get(1) {
            for pair in meta_str.as_str().split(',') {
                let pair = pair.trim();
                if let Some((key, value)) = pair.split_once('=') {
                    metadata.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
    }

    // Check for end marker
    let complete = END_MARKER.is_match(raw);

    // Strip all markers from content
    content = DELEGATE_MARKER.replace_all(&content, "").to_string();
    content = META_MARKER.replace_all(&content, "").to_string();
    content = END_MARKER.replace_all(&content, "").to_string();

    // Clean up extra whitespace
    content = content.trim().to_string();

    ParsedResponse {
        content,
        complete,
        delegations,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complete_response_with_all_markers() {
        let raw = r#"
I analyzed the code and found two issues:
1. Missing error handling on line 42
2. Unused variable on line 15

<!--ARMADAI_META:status=complete,tokens=150-->
<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        assert!(parsed.complete);
        assert_eq!(parsed.delegations.len(), 0);
        assert_eq!(parsed.metadata.get("status"), Some(&"complete".to_string()));
        assert_eq!(parsed.metadata.get("tokens"), Some(&"150".to_string()));
        assert!(parsed.content.contains("Missing error handling"));
        assert!(!parsed.content.contains("ARMADAI"));
    }

    #[test]
    fn test_response_with_no_markers() {
        let raw = "Just a plain response with no markers.";

        let parsed = parse_response(raw);

        assert!(!parsed.complete);
        assert_eq!(parsed.delegations.len(), 0);
        assert!(parsed.metadata.is_empty());
        assert_eq!(parsed.content, "Just a plain response with no markers.");
    }

    #[test]
    fn test_response_with_only_end_marker() {
        let raw = r#"
Simple response.

<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        assert!(parsed.complete);
        assert_eq!(parsed.delegations.len(), 0);
        assert!(parsed.metadata.is_empty());
        assert_eq!(parsed.content, "Simple response.");
    }

    #[test]
    fn test_multiple_delegate_markers() {
        let raw = r#"
I'll delegate to two agents:

<!--ARMADAI_DELEGATE:qa-specialist-->
Please review the tests.

<!--ARMADAI_DELEGATE:ui-specialist-->
Update the dashboard.

<!--ARMADAI_META:status=delegated-->
<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        assert!(parsed.complete);
        assert_eq!(parsed.delegations.len(), 2);
        assert_eq!(parsed.delegations[0], "qa-specialist");
        assert_eq!(parsed.delegations[1], "ui-specialist");
        assert_eq!(
            parsed.metadata.get("status"),
            Some(&"delegated".to_string())
        );
    }

    #[test]
    fn test_meta_with_multiple_key_value_pairs() {
        let raw = r#"
Response with metadata.

<!--ARMADAI_META:status=complete,tokens=200,model=claude-3-opus-->
<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        assert!(parsed.complete);
        assert_eq!(parsed.metadata.len(), 3);
        assert_eq!(parsed.metadata.get("status"), Some(&"complete".to_string()));
        assert_eq!(parsed.metadata.get("tokens"), Some(&"200".to_string()));
        assert_eq!(
            parsed.metadata.get("model"),
            Some(&"claude-3-opus".to_string())
        );
    }

    #[test]
    fn test_markers_embedded_in_code_blocks() {
        let raw = r#"
Here's how to use the protocol:

```markdown
Your response here.

<!--ARMADAI_META:status=complete-->
<!--ARMADAI_END-->
```

Above is an example.

<!--ARMADAI_META:status=complete-->
<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        // Should detect both the marker in the code block AND the real one
        assert!(parsed.complete);
        // The code block marker will also be stripped, which is expected behavior
        // since we're doing simple regex matching
        assert!(parsed.content.contains("Here's how to use the protocol"));
    }

    #[test]
    fn test_partial_malformed_markers() {
        let raw = r#"
Response with malformed markers.

<!--ARMADAI_DELEGATE-->
<!--ARMADAI_META-->
<!--ARMADAI-->

<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        // Only the END marker should be recognized
        assert!(parsed.complete);
        assert_eq!(parsed.delegations.len(), 0);
        assert!(parsed.metadata.is_empty());
    }

    #[test]
    fn test_whitespace_handling() {
        let raw = r#"


Response with lots of whitespace.


<!--ARMADAI_META:  status = complete  ,  tokens = 100  -->
<!--ARMADAI_END-->


"#;

        let parsed = parse_response(raw);

        assert!(parsed.complete);
        assert_eq!(parsed.metadata.get("status"), Some(&"complete".to_string()));
        assert_eq!(parsed.metadata.get("tokens"), Some(&"100".to_string()));
        assert_eq!(parsed.content, "Response with lots of whitespace.");
    }

    #[test]
    fn test_delegate_with_spaces() {
        let raw = r#"
<!--ARMADAI_DELEGATE:  qa-specialist  -->

<!--ARMADAI_END-->
"#;

        let parsed = parse_response(raw);

        assert!(parsed.complete);
        assert_eq!(parsed.delegations.len(), 1);
        assert_eq!(parsed.delegations[0], "qa-specialist");
    }

    #[test]
    fn test_empty_response() {
        let raw = "";

        let parsed = parse_response(raw);

        assert!(!parsed.complete);
        assert_eq!(parsed.delegations.len(), 0);
        assert!(parsed.metadata.is_empty());
        assert_eq!(parsed.content, "");
    }

    #[test]
    fn test_end_marker_not_at_end() {
        let raw = r#"
<!--ARMADAI_END-->

Wait, I have more to say!
"#;

        let parsed = parse_response(raw);

        // Marker is still detected even if not at the end
        assert!(parsed.complete);
        assert!(parsed.content.contains("Wait, I have more to say!"));
    }
}
