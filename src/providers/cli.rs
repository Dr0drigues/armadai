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
        // Ensure the child is SIGKILLed if the `Child`/future is dropped
        // without being awaited to completion — e.g. when a caller aborts
        // the `tokio::spawn`'d task driving `complete()`/`stream()` (see
        // `shell::run_view::run_loop`'s Ctrl+C/`q` handling). Without this,
        // an aborted run leaves the CLI subprocess (e.g. `claude`) running
        // and orphaned, still writing to the inherited stdout/stderr after
        // the TUI has torn down the alternate screen — looking like "the
        // run keeps going" even though ArmadAI itself has exited (#274).
        cmd.kill_on_drop(true);
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

    /// Run `fut` while holding `ENV_MUTEX`, serialising the child-process spawn's
    /// read of `environ` against tests that mutate it via `std::env::set_var`
    /// (otherwise a data race — the reason `set_var` is `unsafe`). Holding the
    /// guard across `.await` is safe here: no other async task contends for
    /// `ENV_MUTEX`, so there is no deadlock risk.
    #[allow(clippy::await_holding_lock)]
    async fn with_env_lock<T>(fut: impl std::future::Future<Output = T>) -> T {
        let _guard = crate::core::config::ENV_MUTEX.lock().unwrap();
        fut.await
    }

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
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            provider
                .complete(echo_request("hello world"))
                .await
                .unwrap()
        })
        .await;
        assert_eq!(response.content.trim(), "hello world");
        assert_eq!(response.tokens_in, 0);
        assert_eq!(response.cost, 0.0);
    }

    #[tokio::test]
    async fn cli_complete_with_args() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec!["prefix".to_string()], 10);
            provider.complete(echo_request("test")).await.unwrap()
        })
        .await;
        assert_eq!(response.content.trim(), "prefix test");
    }

    #[tokio::test]
    async fn cli_stream_echo() {
        use tokio_stream::StreamExt;

        let output = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            let mut stream = provider.stream(echo_request("stream test")).await.unwrap();

            let mut output = Vec::new();
            while let Some(line) = stream.next().await {
                output.push(line.unwrap());
            }
            output
        })
        .await;
        assert_eq!(output, vec!["stream test"]);
    }

    #[tokio::test]
    async fn cli_complete_failure() {
        let result = with_env_lock(async {
            let provider = CliProvider::new("false".to_string(), vec![], 10);
            provider.complete(echo_request("")).await
        })
        .await;
        assert!(result.is_err());
    }

    // ── kill_on_drop (fix for #274: abort must not orphan the subprocess) ──

    /// Best-effort liveness check via `kill -0 <pid>` (POSIX; both the CI
    /// Linux runners and macOS dev machines have `kill`). A zombie still
    /// occupies the process table until its parent reaps it, so this
    /// correctly reports "alive" until the OS/tokio's orphan-queue reaper
    /// has actually collected it.
    fn process_alive(pid: i32) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn dropping_child_kills_orphaned_process_via_kill_on_drop() {
        // Regression test for #274: the Workroom's `run_loop` aborts a
        // running orchestration by dropping the `tokio::spawn`'d task
        // future. Without `kill_on_drop(true)` on the `Command` built in
        // `build_command`, dropping the future (and its owned `Child`)
        // leaves the CLI subprocess (e.g. `claude`) running and orphaned —
        // it keeps writing to the inherited stdout/stderr after the TUI has
        // torn down, which looks like "the run keeps going" even though
        // ArmadAI has exited. This asserts the child is actually gone
        // shortly after the `Child` handle is dropped, unawaited.
        let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 10);
        let mut cmd = provider.build_command("sleep 30");
        let child = cmd.spawn().expect("failed to spawn `sh -c sleep 30`");
        let pid = child.id().expect("spawned child should have a pid") as i32;

        assert!(
            process_alive(pid),
            "sanity check: child {pid} should be running right after spawn"
        );

        // Simulate `handle.abort()`: drop the future (and the `Child` it
        // owns) without ever calling `.wait()`/`.output()` on it.
        drop(child);

        // `kill_on_drop`'s `Drop` impl sends SIGKILL synchronously, but the
        // process table entry is only cleared once tokio's orphan-queue
        // reaper collects it (driven by SIGCHLD) — poll briefly instead of
        // asserting instantaneously.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline && process_alive(pid) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        assert!(
            !process_alive(pid),
            "child process {pid} should have been killed+reaped when the Child was \
             dropped (kill_on_drop); a still-running process here reproduces the #274 orphan"
        );
    }

    #[tokio::test]
    async fn cli_complete_timeout() {
        let result = with_env_lock(async {
            let provider = CliProvider::new("sleep".to_string(), vec![], 1);
            provider.complete(echo_request("30")).await
        })
        .await;
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
    async fn non_empty_system_prompt_reaches_the_command() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            let request =
                request_with_system("PERSONA-XYZ instructions de delegation", "fais la tache");

            provider.complete(request).await.unwrap()
        })
        .await;

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
    async fn empty_system_prompt_changes_nothing() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            let request = request_with_system("", "hello world");

            provider.complete(request).await.unwrap()
        })
        .await;

        assert_eq!(response.content.trim(), "hello world");
    }
}
