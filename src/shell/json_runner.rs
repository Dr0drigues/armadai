//! Universal JSON runner — parses structured JSON output from CLI tools.
//!
//! Supports: Claude Code, Gemini CLI, Codex, Copilot CLI, OpenCode.
//! Falls back to text parsing for CLIs without JSON support (Aider).

use serde_json::Value;

/// Unified response from any CLI, parsed from JSON or text.
#[derive(Debug, Clone)]
pub struct CliResponse {
    /// The response text content
    pub content: String,
    /// Actual tokens in (from CLI metrics, not estimated)
    pub tokens_in: Option<u64>,
    /// Actual tokens out
    pub tokens_out: Option<u64>,
    /// Actual cost in USD
    pub cost_usd: Option<f64>,
    /// Duration reported by the CLI
    pub duration_ms: Option<u64>,
    /// Model actually used
    pub model: Option<String>,
    /// Session ID from the CLI
    pub session_id: Option<String>,
    /// Whether the response was parsed from JSON (true) or text fallback (false)
    pub from_json: bool,
}

/// Provider-specific JSON output flags.
pub struct JsonFlags {
    /// The CLI flag to enable JSON output
    pub flag: &'static str,
    /// The value for the flag (if needed)
    pub value: Option<&'static str>,
}

/// Get the JSON output flags for a provider.
/// Returns None if the provider doesn't support JSON output.
pub fn json_output_flags(provider: &str) -> Option<Vec<String>> {
    match provider {
        "claude" => Some(vec!["--output-format".to_string(), "json".to_string()]),
        "gemini" => Some(vec!["-o".to_string(), "json".to_string()]),
        "codex" => Some(vec!["--json".to_string()]),
        "copilot" => Some(vec!["--output-format".to_string(), "json".to_string()]),
        "opencode" => Some(vec!["--format".to_string(), "json".to_string()]),
        // Aider and unknown CLIs: no JSON support
        _ => None,
    }
}

/// Get the base CLI args for a provider in stream-JSON mode.
/// Uses stream-json when available for real-time JSONL event streaming.
pub fn json_mode_args(provider: &str) -> Vec<String> {
    match provider {
        "claude" => vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ],
        // NOTE: For Gemini, -p must be followed immediately by the prompt value.
        // The prompt is appended as the last arg by the caller, so we put -o BEFORE -p.
        "gemini" => vec![
            "-o".to_string(),
            "stream-json".to_string(),
            "-p".to_string(),
        ],
        "codex" => vec!["exec".to_string(), "--json".to_string()],
        // Copilot: -p takes a value, so put other flags before -p
        "copilot" => vec![
            "--output-format".to_string(),
            "json".to_string(),
            "-p".to_string(),
        ],
        "opencode" => vec![
            "run".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ],
        // Aider: text mode fallback
        "aider" => vec!["--yes".to_string(), "--message".to_string()],
        _ => vec![],
    }
}

/// Check if a provider supports JSON output.
pub fn supports_json(provider: &str) -> bool {
    json_output_flags(provider).is_some()
}

/// A streaming event parsed from a single JSONL line.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Init event with metadata (agents, model, etc.)
    Init {
        model: Option<String>,
        agents: Vec<String>,
    },
    /// Text delta — append to the current response
    Delta(String),
    /// Complete message text (non-delta)
    Message(String),
    /// Result/completion with metrics
    Result(CliResponse),
    /// Error event
    Error(String),
    /// Unknown/ignored event
    Ignored,
}

/// Parse a single JSONL line into a StreamEvent.
pub fn parse_stream_event(provider: &str, line: &str) -> StreamEvent {
    let line = line.trim();
    if line.is_empty() {
        return StreamEvent::Ignored;
    }
    let Ok(json) = serde_json::from_str::<Value>(line) else {
        return StreamEvent::Ignored;
    };

    let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match provider {
        "claude" => parse_claude_stream_event(event_type, &json),
        "gemini" => parse_gemini_stream_event(event_type, &json),
        "codex" => parse_codex_stream_event(event_type, &json),
        "copilot" => parse_copilot_stream_event(event_type, &json),
        "opencode" => parse_copilot_stream_event(event_type, &json), // similar format
        _ => StreamEvent::Ignored,
    }
}

fn parse_claude_stream_event(event_type: &str, json: &Value) -> StreamEvent {
    match event_type {
        "system" if json.get("subtype").and_then(|v| v.as_str()) == Some("init") => {
            let model = json
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let agents = json
                .get("agents")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            StreamEvent::Init { model, agents }
        }
        "assistant" => {
            if let Some(message) = json.get("message")
                && let Some(content) = message.get("content").and_then(|v| v.as_array())
            {
                let text: String = content
                    .iter()
                    .filter_map(|c| {
                        if c.get("type").and_then(|t| t.as_str()) == Some("text") {
                            c.get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    return StreamEvent::Delta(text);
                }
            }
            StreamEvent::Ignored
        }
        "result" => {
            let resp = parse_claude_json(json);
            StreamEvent::Result(resp)
        }
        "error" => {
            let msg = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            StreamEvent::Error(msg)
        }
        _ => StreamEvent::Ignored,
    }
}

fn parse_gemini_stream_event(event_type: &str, json: &Value) -> StreamEvent {
    match event_type {
        "init" => {
            let model = json
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            StreamEvent::Init {
                model,
                agents: vec![],
            }
        }
        "message" if json.get("role").and_then(|v| v.as_str()) == Some("assistant") => {
            let content = json
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_delta = json.get("delta").and_then(|v| v.as_bool()).unwrap_or(false);
            if is_delta {
                StreamEvent::Delta(content)
            } else {
                StreamEvent::Message(content)
            }
        }
        "result" => {
            let resp = parse_gemini_result(json);
            StreamEvent::Result(resp)
        }
        "error" => {
            let msg = json
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            StreamEvent::Error(msg)
        }
        _ => StreamEvent::Ignored,
    }
}

fn parse_gemini_result(json: &Value) -> CliResponse {
    let stats = json.get("stats");
    let tokens_in = stats
        .and_then(|s| s.get("input_tokens"))
        .and_then(|v| v.as_u64());
    let tokens_out = stats
        .and_then(|s| s.get("output_tokens"))
        .and_then(|v| v.as_u64());
    let duration_ms = stats
        .and_then(|s| s.get("duration_ms"))
        .and_then(|v| v.as_u64());

    let model = stats
        .and_then(|s| s.get("models"))
        .and_then(|m| m.as_object())
        .and_then(|obj| obj.keys().next().cloned());

    CliResponse {
        content: String::new(), // content already streamed via deltas
        tokens_in,
        tokens_out,
        cost_usd: None,
        duration_ms,
        model,
        session_id: json
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        from_json: true,
    }
}

fn parse_codex_stream_event(event_type: &str, json: &Value) -> StreamEvent {
    match event_type {
        "thread.started" => StreamEvent::Init {
            model: None,
            agents: vec![],
        },
        "item.completed" => {
            let text = json
                .get("item")
                .and_then(|i| i.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            StreamEvent::Message(text)
        }
        "turn.completed" => {
            let usage = json.get("usage");
            StreamEvent::Result(CliResponse {
                content: String::new(),
                tokens_in: usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64()),
                tokens_out: usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_u64()),
                cost_usd: None,
                duration_ms: None,
                model: None,
                session_id: None,
                from_json: true,
            })
        }
        "error" => {
            let msg = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            StreamEvent::Error(msg)
        }
        _ => StreamEvent::Ignored,
    }
}

fn parse_copilot_stream_event(event_type: &str, json: &Value) -> StreamEvent {
    match event_type {
        "session.tools_updated" => {
            let model = json
                .get("data")
                .and_then(|d| d.get("model"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            StreamEvent::Init {
                model,
                agents: vec![],
            }
        }
        "assistant.message_delta" => {
            let delta = json
                .get("data")
                .and_then(|d| d.get("deltaContent"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            StreamEvent::Delta(delta)
        }
        "assistant.message" => {
            let content = json
                .get("data")
                .and_then(|d| d.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            StreamEvent::Message(content)
        }
        "result" => {
            let usage = json.get("usage");
            let duration = usage
                .and_then(|u| u.get("totalApiDurationMs"))
                .and_then(|v| v.as_u64());
            StreamEvent::Result(CliResponse {
                content: String::new(),
                tokens_in: None,
                tokens_out: None,
                cost_usd: None,
                duration_ms: duration,
                model: None,
                session_id: json
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                from_json: true,
            })
        }
        "error" => {
            let msg = json
                .get("data")
                .and_then(|d| d.get("message"))
                .or(json.get("error").and_then(|e| e.get("message")))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            StreamEvent::Error(msg)
        }
        _ => StreamEvent::Ignored,
    }
}

/// Parse CLI JSON output into a unified CliResponse.
pub fn parse_json_response(provider: &str, raw: &str) -> CliResponse {
    // Try to parse as JSON first
    if let Ok(json) = serde_json::from_str::<Value>(raw) {
        match provider {
            "claude" => parse_claude_json(&json),
            "gemini" => parse_gemini_json(&json),
            "codex" => parse_codex_json(raw),
            "copilot" => parse_copilot_json(raw),
            "opencode" => parse_opencode_json(raw),
            _ => text_fallback(raw),
        }
    } else if provider == "codex" || provider == "copilot" || provider == "opencode" {
        // These use JSONL (one JSON per line) — parse last meaningful line
        parse_jsonl_response(provider, raw)
    } else {
        text_fallback(raw)
    }
}

/// Parse Claude Code JSON response.
fn parse_claude_json(json: &Value) -> CliResponse {
    let content = json
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let cost_usd = json.get("total_cost_usd").and_then(|v| v.as_f64());
    let duration_ms = json.get("duration_ms").and_then(|v| v.as_u64());
    let session_id = json
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract tokens from usage
    let usage = json.get("usage");
    let tokens_in = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(|v| v.as_u64());
    let tokens_out = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64());

    // Extract model from modelUsage keys
    let model = json
        .get("modelUsage")
        .and_then(|v| v.as_object())
        .and_then(|obj| {
            // Pick the model with the most output tokens (main model)
            obj.iter()
                .max_by_key(|(_, v)| v.get("outputTokens").and_then(|t| t.as_u64()).unwrap_or(0))
                .map(|(k, _)| k.clone())
        });

    CliResponse {
        content,
        tokens_in,
        tokens_out,
        cost_usd,
        duration_ms,
        model,
        session_id,
        from_json: true,
    }
}

/// Parse Gemini CLI JSON response.
fn parse_gemini_json(json: &Value) -> CliResponse {
    let content = json
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let session_id = json
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract from stats.models (first model entry)
    let stats = json.get("stats").and_then(|s| s.get("models"));
    let (tokens_in, tokens_out, duration_ms, model) =
        if let Some(models) = stats.and_then(|m| m.as_object()) {
            if let Some((model_name, model_data)) = models.iter().next() {
                let tokens = model_data.get("tokens");
                let api = model_data.get("api");
                (
                    tokens.and_then(|t| t.get("input")).and_then(|v| v.as_u64()),
                    tokens
                        .and_then(|t| t.get("candidates"))
                        .and_then(|v| v.as_u64()),
                    api.and_then(|a| a.get("totalLatencyMs"))
                        .and_then(|v| v.as_u64()),
                    Some(model_name.clone()),
                )
            } else {
                (None, None, None, None)
            }
        } else {
            (None, None, None, None)
        };

    CliResponse {
        content,
        tokens_in,
        tokens_out,
        cost_usd: None, // Gemini doesn't report cost directly
        duration_ms,
        model,
        session_id,
        from_json: true,
    }
}

/// Parse JSONL output (Codex, Copilot, OpenCode) — each line is a JSON event.
fn parse_jsonl_response(provider: &str, raw: &str) -> CliResponse {
    let mut content = String::new();
    let mut tokens_in: Option<u64> = None;
    let mut tokens_out: Option<u64> = None;
    let mut cost_usd: Option<f64> = None;
    let duration_ms: Option<u64> = None;
    let mut model: Option<String> = None;
    let mut session_id: Option<String> = None;

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        match provider {
            "codex" => {
                if let Some(msg_type) = event.get("type").and_then(|v| v.as_str()) {
                    if msg_type == "message"
                        && let Some(text) = event.get("content").and_then(|v| v.as_str())
                    {
                        content.push_str(text);
                    }
                    if msg_type == "usage" || msg_type == "stats" {
                        tokens_in = event
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .or(tokens_in);
                        tokens_out = event
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .or(tokens_out);
                    }
                }
                if session_id.is_none() {
                    session_id = event
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            "copilot" | "opencode" => {
                if let Some(msg_type) = event.get("type").and_then(|v| v.as_str()) {
                    if (msg_type == "result" || msg_type == "response")
                        && let Some(text) = event
                            .get("result")
                            .or(event.get("response"))
                            .and_then(|v| v.as_str())
                    {
                        content = text.to_string();
                    }
                    if msg_type == "usage" {
                        tokens_in = event
                            .get("inputTokens")
                            .and_then(|v| v.as_u64())
                            .or(event.get("input_tokens").and_then(|v| v.as_u64()))
                            .or(tokens_in);
                        tokens_out = event
                            .get("outputTokens")
                            .and_then(|v| v.as_u64())
                            .or(event.get("output_tokens").and_then(|v| v.as_u64()))
                            .or(tokens_out);
                        cost_usd = event
                            .get("cost")
                            .and_then(|v| v.as_f64())
                            .or(event.get("total_cost_usd").and_then(|v| v.as_f64()))
                            .or(cost_usd);
                    }
                }
                if session_id.is_none() {
                    session_id = event
                        .get("sessionID")
                        .or(event.get("session_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                if model.is_none() {
                    model = event
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            _ => {}
        }
    }

    // If no content found via events, use last non-empty line as fallback
    if content.is_empty() {
        content = raw
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_string();
    }

    CliResponse {
        content,
        tokens_in,
        tokens_out,
        cost_usd,
        duration_ms,
        model,
        session_id,
        from_json: !raw.is_empty(),
    }
}

fn parse_codex_json(raw: &str) -> CliResponse {
    parse_jsonl_response("codex", raw)
}

fn parse_copilot_json(raw: &str) -> CliResponse {
    parse_jsonl_response("copilot", raw)
}

fn parse_opencode_json(raw: &str) -> CliResponse {
    parse_jsonl_response("opencode", raw)
}

/// Text fallback for CLIs without JSON support.
fn text_fallback(raw: &str) -> CliResponse {
    let parsed = super::parser::parse_response(raw);
    CliResponse {
        content: parsed.content,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        duration_ms: None,
        model: None,
        session_id: None,
        from_json: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_claude_json() {
        let json = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":5100,"num_turns":1,"result":"Hello!","session_id":"abc-123","total_cost_usd":0.076,"usage":{"input_tokens":100,"output_tokens":10},"modelUsage":{"claude-opus-4-6":{"outputTokens":10}}}"#;
        let resp = parse_json_response("claude", json);
        assert_eq!(resp.content, "Hello!");
        assert_eq!(resp.tokens_in, Some(100));
        assert_eq!(resp.tokens_out, Some(10));
        assert_eq!(resp.cost_usd, Some(0.076));
        assert_eq!(resp.duration_ms, Some(5100));
        assert_eq!(resp.session_id, Some("abc-123".to_string()));
        assert_eq!(resp.model, Some("claude-opus-4-6".to_string()));
        assert!(resp.from_json);
    }

    #[test]
    fn test_parse_gemini_json() {
        let json = r#"{"session_id":"sess-1","response":"Hi there!","stats":{"models":{"gemini-2.5-flash":{"api":{"totalLatencyMs":3000},"tokens":{"input":500,"candidates":20}}}}}"#;
        let resp = parse_json_response("gemini", json);
        assert_eq!(resp.content, "Hi there!");
        assert_eq!(resp.tokens_in, Some(500));
        assert_eq!(resp.tokens_out, Some(20));
        assert_eq!(resp.duration_ms, Some(3000));
        assert_eq!(resp.model, Some("gemini-2.5-flash".to_string()));
        assert!(resp.from_json);
    }

    #[test]
    fn test_text_fallback() {
        let resp = parse_json_response("aider", "Just some text response");
        assert_eq!(resp.content, "Just some text response");
        assert!(resp.tokens_in.is_none());
        assert!(!resp.from_json);
    }

    #[test]
    fn test_supports_json() {
        assert!(supports_json("claude"));
        assert!(supports_json("gemini"));
        assert!(supports_json("codex"));
        assert!(supports_json("copilot"));
        assert!(supports_json("opencode"));
        assert!(!supports_json("aider"));
        assert!(!supports_json("unknown"));
    }

    #[test]
    fn test_json_mode_args() {
        let args = json_mode_args("claude");
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"stream-json".to_string()));
        assert!(args.contains(&"--verbose".to_string()));

        let args = json_mode_args("gemini");
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"stream-json".to_string()));

        let args = json_mode_args("aider");
        assert!(args.contains(&"--message".to_string())); // text fallback
    }
}
