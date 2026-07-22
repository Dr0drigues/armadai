use async_trait::async_trait;
use tokio::process::Command;

use super::traits::*;

/// Generic CLI provider that spawns any configured command.
pub struct CliProvider {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_secs: u64,
}

impl CliProvider {
    pub fn new(command: String, args: Vec<String>, timeout_secs: u64) -> Self {
        Self {
            command,
            args,
            timeout_secs,
        }
    }

    /// Compose the input string sent to the CLI command from the request's
    /// system prompt and last user message. The system prompt is the only
    /// channel carrying agent persona and delegation instructions (see
    /// `traits::CompletionRequest`); CLI providers have no separate channel
    /// for it, so it is prefixed onto the task, clearly delimited.
    ///
    /// When `system_prompt` is empty, the message is returned unchanged
    /// (no prefix, no tags) to preserve existing behavior exactly.
    fn compose_input(&self, request: &CompletionRequest) -> String {
        let msg = request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        if request.system_prompt.is_empty() {
            msg.to_string()
        } else {
            format!("<system>\n{}\n</system>\n\n{}", request.system_prompt, msg)
        }
    }

    fn build_command(&self, input: &str) -> Command {
        let mut cmd = Command::new(&self.command);
        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd.arg(input);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd
    }

    fn parse_json_stdout(&self, raw: &str) -> CompletionResponse {
        use crate::shell::json_runner::{StreamEvent, parse_stream_event};

        let mut content = String::new();
        let mut result = None;
        for line in raw.lines() {
            match parse_stream_event(&self.command, line) {
                StreamEvent::Delta(t) | StreamEvent::Message(t) => content.push_str(&t),
                StreamEvent::Result(resp) => result = Some(resp),
                _ => {}
            }
        }

        match result {
            Some(resp) => CompletionResponse {
                content,
                model: resp.model.unwrap_or_else(|| self.command.clone()),
                tokens_in: resp.tokens_in.unwrap_or(0) as u32,
                tokens_out: resp.tokens_out.unwrap_or(0) as u32,
                cost: resp.cost_usd.unwrap_or(0.0),
            },
            None => CompletionResponse {
                content: raw.to_string(),
                model: self.command.clone(),
                tokens_in: 0,
                tokens_out: 0,
                cost: 0.0,
            },
        }
    }
}

#[async_trait]
impl Provider for CliProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let input = self.compose_input(&request);

        // Run with `self.args` verbatim (the factory already selected the right
        // args: canonical stream-json args for a default JSON-capable CLI, or
        // the agent's explicit args). Then parse stdout opportunistically:
        // `parse_json_stdout` extracts content + cost/tokens from JSONL events
        // when present, and falls back to raw stdout (zeroed metrics) otherwise.
        let mut cmd = self.build_command(&input);
        let timeout = std::time::Duration::from_secs(self.timeout_secs);

        let output = match tokio::time::timeout(timeout, cmd.output()).await {
            Ok(result) => result?,
            Err(_) => {
                anyhow::bail!("CLI command timed out after {}s", self.timeout_secs);
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("CLI command failed ({}): {stderr}", output.status);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(self.parse_json_stdout(&stdout))
    }

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        let input = self.compose_input(&request);

        let mut child = self.build_command(&input).spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;

        let timeout_secs = self.timeout_secs;
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = tokio::io::AsyncBufReadExt::lines(reader);

            let timeout = std::time::Duration::from_secs(timeout_secs);
            let result = tokio::time::timeout(timeout, async {
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx.send(Ok(line)).await.is_err() {
                        break;
                    }
                }
            })
            .await;

            if result.is_err() {
                if let Err(e) = tx.send(Err(anyhow::anyhow!("CLI command timed out"))).await {
                    tracing::debug!("Failed to send timeout error (receiver dropped): {:?}", e);
                }
                if let Err(e) = child.kill().await {
                    tracing::debug!("Failed to kill timed-out CLI command: {:?}", e);
                }
            }

            let _ = child.wait().await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: format!("cli:{}", self.command),
            models: vec![self.command.clone()],
            supports_streaming: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_request(text: &str) -> CompletionRequest {
        CompletionRequest {
            model: "echo".to_string(),
            system_prompt: String::new(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            temperature: 0.0,
            max_tokens: None,
        }
    }

    #[tokio::test]
    async fn cli_complete_echo() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let response = provider
            .complete(echo_request("hello world"))
            .await
            .unwrap();
        assert_eq!(response.content.trim(), "hello world");
        assert_eq!(response.tokens_in, 0);
        assert_eq!(response.cost, 0.0);
    }

    #[tokio::test]
    async fn cli_complete_with_args() {
        let provider = CliProvider::new("echo".to_string(), vec!["prefix".to_string()], 10);
        let response = provider.complete(echo_request("test")).await.unwrap();
        assert_eq!(response.content.trim(), "prefix test");
    }

    #[tokio::test]
    async fn cli_stream_echo() {
        use tokio_stream::StreamExt;

        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let mut stream = provider.stream(echo_request("stream test")).await.unwrap();

        let mut output = Vec::new();
        while let Some(line) = stream.next().await {
            output.push(line.unwrap());
        }
        assert_eq!(output, vec!["stream test"]);
    }

    #[tokio::test]
    async fn cli_complete_failure() {
        let provider = CliProvider::new("false".to_string(), vec![], 10);
        let result = provider.complete(echo_request("")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cli_complete_timeout() {
        let provider = CliProvider::new("sleep".to_string(), vec![], 1);
        let result = provider.complete(echo_request("30")).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "Error was: {err}");
    }

    // ── JSON-mode stdout parsing (Part C: real cost/tokens from `claude`) ──

    #[test]
    fn parse_json_stdout_claude_extracts_cost_and_tokens() {
        let provider = CliProvider::new("claude".to_string(), vec![], 10);
        // A representative claude --output-format stream-json session: an
        // init event, one assistant turn (the visible answer), then the
        // terminal result event carrying usage/cost.
        let jsonl = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-4-6","agents":[]}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello there!"}]}}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1200,"num_turns":1,"result":"Hello there!","session_id":"s1","total_cost_usd":0.0123,"usage":{"input_tokens":42,"output_tokens":17},"modelUsage":{"claude-opus-4-6":{"outputTokens":17}}}"#,
        );

        let response = provider.parse_json_stdout(jsonl);

        assert_eq!(response.content, "Hello there!");
        assert_eq!(response.tokens_in, 42);
        assert_eq!(response.tokens_out, 17);
        assert!((response.cost - 0.0123).abs() < 1e-9);
        assert_eq!(response.model, "claude-opus-4-6");
    }

    #[test]
    fn parse_json_stdout_falls_back_to_raw_when_no_result_event() {
        // Partial/interrupted stream: no terminal "result" event at all.
        let provider = CliProvider::new("claude".to_string(), vec![], 10);
        let jsonl = r#"{"type":"system","subtype":"init","session_id":"s1","tools":[]}"#;

        let response = provider.parse_json_stdout(jsonl);

        // Falls back to the current behavior: raw stdout as content, zeroed metrics.
        assert_eq!(response.content, jsonl);
        assert_eq!(response.tokens_in, 0);
        assert_eq!(response.tokens_out, 0);
        assert_eq!(response.cost, 0.0);
    }

    // ── system_prompt forwarding (fix: CliProvider was dropping system_prompt) ──

    fn request_with_system(system_prompt: &str, text: &str) -> CompletionRequest {
        CompletionRequest {
            model: "echo".to_string(),
            system_prompt: system_prompt.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            temperature: 0.0,
            max_tokens: None,
        }
    }

    #[test]
    fn compose_input_empty_system_prompt_returns_message_unchanged() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let request = request_with_system("", "fais la tache");
        assert_eq!(provider.compose_input(&request), "fais la tache");
    }

    #[test]
    fn compose_input_non_empty_system_prompt_prefixes_message() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let request =
            request_with_system("PERSONA-XYZ instructions de delegation", "fais la tache");
        let composed = provider.compose_input(&request);

        assert_eq!(
            composed,
            "<system>\nPERSONA-XYZ instructions de delegation\n</system>\n\nfais la tache"
        );
    }

    #[tokio::test]
    async fn system_prompt_non_vide_atteint_la_commande() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let request =
            request_with_system("PERSONA-XYZ instructions de delegation", "fais la tache");

        let response = provider.complete(request).await.unwrap();

        assert!(
            response.content.contains("PERSONA-XYZ"),
            "expected system prompt marker in output, got: {}",
            response.content
        );
        assert!(
            response.content.contains("fais la tache"),
            "expected task text in output, got: {}",
            response.content
        );
        let system_pos = response.content.find("PERSONA-XYZ").unwrap();
        let task_pos = response.content.find("fais la tache").unwrap();
        assert!(
            system_pos < task_pos,
            "expected system prompt before task, got: {}",
            response.content
        );
    }

    #[tokio::test]
    async fn system_prompt_vide_ne_change_rien() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let request = request_with_system("", "hello world");

        let response = provider.complete(request).await.unwrap();

        assert_eq!(response.content.trim(), "hello world");
    }
}
