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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// One `tool_result` content block from a `user` message.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub text: String,
}

/// A transcript entry the mapper acts on. Everything else is dropped.
#[derive(Debug, Clone, PartialEq)]
pub enum RelevantEntry {
    Assistant {
        model: String,
        blocks: Vec<Block>,
        usage: Usage,
        /// The assistant message's `stop_reason` (`"end_turn"`, `"tool_use"`,
        /// `"stop_sequence"`, `"max_tokens"`, …). `None` if absent or JSON
        /// null — which the follow-mode driver treats as "still going".
        stop_reason: Option<String>,
    },
    /// Anthropic batches ALL `tool_result` blocks for a turn into a single
    /// `user` message (e.g. parallel `Agent` spawns resolving together), so
    /// this carries every block found in that message, not just the first.
    ToolResults(Vec<ToolResult>),
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
    // `None` when absent or JSON null (`as_str` yields `None` for null) — the
    // driver reads that as "turn still in progress".
    let stop_reason = msg
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
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
    Some(RelevantEntry::Assistant {
        model,
        blocks,
        usage,
        stop_reason,
    })
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
    }
}

fn parse_user_tool_result(msg: &Value) -> Option<RelevantEntry> {
    let mut results = Vec::new();
    for b in msg
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if b.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else {
            continue; // defensive: malformed block, skip it, keep the rest
        };
        let text = tool_result_text(b.get("content"));
        results.push(ToolResult {
            tool_use_id: id.to_string(),
            text,
        });
    }
    if results.is_empty() {
        None
    } else {
        Some(RelevantEntry::ToolResults(results))
    }
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
                stop_reason,
            } => {
                assert_eq!(model, "claude-x");
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
                assert!(matches!(blocks.as_slice(), [Block::Text(t)] if t == "hello"));
                // Absent in this line → None.
                assert_eq!(stop_reason, None);
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_assistant_stop_reason_when_present() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","model":"m","stop_reason":"end_turn","content":[{"type":"text","text":"bye"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { stop_reason, .. } => {
                assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_assistant_null_stop_reason_as_none() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","model":"m","stop_reason":null,"content":[{"type":"text","text":"mid"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { stop_reason, .. } => {
                assert_eq!(stop_reason, None, "JSON null stop_reason must map to None");
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_agent_spawn_tool_use() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"tu1","name":"Agent","input":{"subagent_type":"core-specialist","description":"Architecture du workspace","prompt":"x"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => {
                assert!(matches!(blocks.as_slice(),
                    [Block::AgentSpawn { tool_use_id, subagent_type, description }]
                    if tool_use_id == "tu1"
                        && subagent_type == "core-specialist"
                        && description == "Architecture du workspace"));
            }
            _ => panic!("expected Assistant"),
        }
    }

    /// A `tool_use{name:"Agent"}` without an `input.description` must parse
    /// with an empty `description` (mapper falls back to `subagent_type`).
    #[test]
    fn parses_agent_spawn_without_description_defaults_empty() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"tu9","name":"Agent","input":{"subagent_type":"Explore","prompt":"x"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => {
                assert!(matches!(blocks.as_slice(),
                    [Block::AgentSpawn { subagent_type, description, .. }]
                    if subagent_type == "Explore" && description.is_empty()));
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn parses_tool_result_from_user() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":[{"type":"text","text":"done"}]}]}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::ToolResults(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].tool_use_id, "tu1");
                assert_eq!(results[0].text, "done");
            }
            _ => panic!("expected ToolResults"),
        }
    }

    /// I3: a single `user` message batches ALL `tool_result` blocks for a
    /// turn (e.g. two parallel `Agent` spawns resolving together) — every
    /// block must survive parsing, not just the first.
    #[test]
    fn parses_all_tool_result_blocks_in_one_user_message() {
        let line = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"tu1","content":[{"type":"text","text":"first done"}]},
            {"type":"tool_result","tool_use_id":"tu2","content":[{"type":"text","text":"second done"}]}
        ]}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::ToolResults(results) => {
                assert_eq!(results.len(), 2, "both tool_result blocks must be kept");
                assert_eq!(results[0].tool_use_id, "tu1");
                assert_eq!(results[0].text, "first done");
                assert_eq!(results[1].tool_use_id, "tu2");
                assert_eq!(results[1].text, "second done");
            }
            _ => panic!("expected ToolResults"),
        }
    }

    #[test]
    fn non_agent_tool_use_yields_other_block() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).unwrap() {
            RelevantEntry::Assistant { blocks, .. } => {
                assert!(matches!(blocks.as_slice(), [Block::Tool { .. }]))
            }
            _ => panic!("expected Assistant"),
        }
    }

    #[test]
    fn non_agent_tool_use_keeps_its_name() {
        let line = r#"{"type":"assistant","message":{"model":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        match parse_line(line).expect("assistant entry") {
            RelevantEntry::Assistant { blocks, .. } => {
                assert_eq!(
                    blocks.as_slice(),
                    [Block::Tool {
                        name: "Bash".to_string()
                    }],
                    "a non-Agent tool_use must carry its tool name"
                );
            }
            _ => panic!("expected Assistant"),
        }
    }
}
