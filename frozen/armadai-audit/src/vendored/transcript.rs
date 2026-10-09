//! FROZEN copy of `crates/armadai/src/claude_adapter/transcript.rs` at commit
//! c486959 (moved here: the audit's usage scan was its last user), reduced to
//! what the scan reads: assistant entries, their model and content blocks. Not
//! kept in sync with anything.
//!
//! Defensive parser for Claude Code transcript JSONL entries.

use serde_json::Value;

/// One content block we care about within an assistant message.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    AgentSpawn {
        tool_use_id: String,
        subagent_type: String,
        description: String,
    },
    /// Any other `tool_use`, keyed by its tool name (`Bash`, `Read`, `Skill`, …).
    Tool {
        name: String,
    },
}

/// A transcript entry the mapper acts on. Everything else is dropped.
#[derive(Debug, Clone, PartialEq)]
pub enum RelevantEntry {
    Assistant { model: String, blocks: Vec<Block> },
}

/// Parse one transcript entry the caller already parsed as JSON. Returns
/// `None` for any entry the audit does not model (user, ai-title, mode,
/// system, attachment, …) — never panics. The audit
/// scan reads the entry envelope (timestamp, isSidechain, attribution…) from
/// the same `Value`, so parsing the line twice would double its cost.
pub fn parse_value(v: &Value) -> Option<RelevantEntry> {
    match v.get("type")?.as_str()? {
        "assistant" => parse_assistant(v.get("message")?),
        _ => None,
    }
}

fn parse_assistant(msg: &Value) -> Option<RelevantEntry> {
    let model = msg
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut blocks = Vec::new();
    for b in msg
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = b.get("text").and_then(Value::as_str) {
                    blocks.push(Block::Text(t.to_string()));
                }
            }
            Some("tool_use") if b.get("name").and_then(Value::as_str) == Some("Agent") => {
                let id = b
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let sub = b
                    .get("input")
                    .and_then(|i| i.get("subagent_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .to_string();
                let description = b
                    .get("input")
                    .and_then(|i| i.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                blocks.push(Block::AgentSpawn {
                    tool_use_id: id,
                    subagent_type: sub,
                    description,
                });
            }
            Some("tool_use") => {
                let name = b
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                blocks.push(Block::Tool { name });
            }
            _ => {} // thinking, redacted_thinking, etc. — dropped
        }
    }
    Some(RelevantEntry::Assistant { model, blocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &str) -> Option<RelevantEntry> {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        parse_value(&v)
    }

    #[test]
    fn ignores_app_specific_user_and_malformed() {
        assert!(parse_line(r#"{"type":"ai-title","aiTitle":"x"}"#).is_none());
        assert!(parse_line(r#"{"type":"mode","mode":"x"}"#).is_none());
        assert!(parse_line(r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"done"}]}}"#).is_none());
        assert!(parse_line("not json").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn parses_assistant_text_and_model() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-x","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { model, blocks } => {
                assert_eq!(model, "claude-x");
                assert!(matches!(blocks.as_slice(), [Block::Text(t)] if t == "hello"));
            }
        }
    }

    #[test]
    fn parses_agent_spawn_tool_use() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"tu1","name":"Agent","input":{"subagent_type":"core-specialist","description":"Architecture du workspace","prompt":"x"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let RelevantEntry::Assistant { blocks, .. } = parse_line(line).unwrap();
        assert!(matches!(blocks.as_slice(),
            [Block::AgentSpawn { tool_use_id, subagent_type, description }]
            if tool_use_id == "tu1"
                && subagent_type == "core-specialist"
                && description == "Architecture du workspace"));
    }

    /// A `tool_use{name:"Agent"}` without an `input.description` must parse
    /// with an empty `description`.
    #[test]
    fn parses_agent_spawn_without_description_defaults_empty() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"tu9","name":"Agent","input":{"subagent_type":"Explore","prompt":"x"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let RelevantEntry::Assistant { blocks, .. } = parse_line(line).unwrap();
        assert!(matches!(blocks.as_slice(),
            [Block::AgentSpawn { subagent_type, description, .. }]
            if subagent_type == "Explore" && description.is_empty()));
    }

    #[test]
    fn non_agent_tool_use_keeps_its_name() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let RelevantEntry::Assistant { blocks, .. } = parse_line(line).expect("assistant entry");
        assert_eq!(
            blocks.as_slice(),
            [Block::Tool {
                name: "Bash".to_string()
            }],
            "a non-Agent tool_use must carry its tool name"
        );
    }

    #[test]
    fn parse_value_reads_the_message_of_an_envelope_the_scan_also_reads() {
        let v: Value = serde_json::from_str(
            r#"{"type":"assistant","isSidechain":true,"uuid":"u1","timestamp":"2026-08-13T10:00:00Z","sessionId":"s1","cwd":"/tmp","message":{"role":"assistant","model":"claude-opus","content":[{"type":"text","text":"response"}],"usage":{"input_tokens":10,"output_tokens":5}}}"#,
        )
        .unwrap();
        // The envelope stays readable alongside the message — this is why the
        // scan holds the Value instead of re-parsing the line.
        assert_eq!(v["uuid"], "u1");
        assert_eq!(v["isSidechain"], true);
        match parse_value(&v) {
            Some(RelevantEntry::Assistant { model, blocks }) => {
                assert_eq!(model, "claude-opus");
                assert!(matches!(blocks.as_slice(), [Block::Text(t)] if t == "response"));
            }
            other => panic!("expected Assistant, got {:?}", other),
        }
    }
}
