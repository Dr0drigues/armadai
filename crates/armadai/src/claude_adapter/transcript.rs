use serde_json::Value;

/// One content block we care about within an assistant message.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    AgentSpawn {
        tool_use_id: String,
        subagent_type: String,
    },
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// A transcript entry the mapper acts on. Everything else is dropped.
#[derive(Debug, Clone, PartialEq)]
pub enum RelevantEntry {
    Assistant {
        model: String,
        blocks: Vec<Block>,
        usage: Usage,
    },
    ToolResult {
        tool_use_id: String,
        text: String,
    },
}

/// Defensive parse of one transcript JSONL line. Returns `None` for malformed
/// lines and for any entry type the adapter does not model (ai-title, mode,
/// pr-link, system, attachment, …) — never panics.
pub fn parse_line(line: &str) -> Option<RelevantEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "assistant" => parse_assistant(v.get("message")?),
        "user" => parse_user_tool_result(v.get("message")?),
        _ => None,
    }
}

fn parse_assistant(msg: &Value) -> Option<RelevantEntry> {
    let model = msg
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let usage = msg.get("usage").map(parse_usage).unwrap_or_default();
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
                blocks.push(Block::AgentSpawn {
                    tool_use_id: id,
                    subagent_type: sub,
                });
            }
            Some("tool_use") => blocks.push(Block::Other),
            _ => {} // thinking, redacted_thinking, etc. — dropped
        }
    }
    Some(RelevantEntry::Assistant {
        model,
        blocks,
        usage,
    })
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
    }
}

fn parse_user_tool_result(msg: &Value) -> Option<RelevantEntry> {
    for b in msg
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if b.get("type").and_then(Value::as_str) == Some("tool_result") {
            let id = b.get("tool_use_id").and_then(Value::as_str)?.to_string();
            let text = tool_result_text(b.get("content"));
            return Some(RelevantEntry::ToolResult {
                tool_use_id: id,
                text,
            });
        }
    }
    None
}

/// A tool_result `content` may be a string or an array of `{type:text,text}`.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_app_specific_and_malformed() {
        assert!(parse_line(r#"{"type":"ai-title","aiTitle":"x"}"#).is_none());
        assert!(parse_line(r#"{"type":"mode","mode":"x"}"#).is_none());
        assert!(parse_line("not json").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn parses_assistant_text_and_usage() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","model":"claude-x","content":[{"type":"text","text":"hello"}],"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant {
                model,
                blocks,
                usage,
            } => {
                assert_eq!(model, "claude-x");
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
                assert!(matches!(blocks.as_slice(), [Block::Text(t)] if t == "hello"));
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_agent_spawn_tool_use() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"tu1","name":"Agent","input":{"subagent_type":"core-specialist","prompt":"x"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => {
                assert!(matches!(blocks.as_slice(),
                    [Block::AgentSpawn { tool_use_id, subagent_type }]
                    if tool_use_id == "tu1" && subagent_type == "core-specialist"));
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_tool_result_from_user() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":[{"type":"text","text":"done"}]}]}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::ToolResult { tool_use_id, text } => {
                assert_eq!(tool_use_id, "tu1");
                assert_eq!(text, "done");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn non_agent_tool_use_yields_other_block() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => {
                assert!(matches!(blocks.as_slice(), [Block::Other]))
            }
            _ => panic!("expected Assistant"),
        }
    }
}
