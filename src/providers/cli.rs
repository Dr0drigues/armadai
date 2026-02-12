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
}

#[async_trait]
impl Provider for CliProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let input = request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let mut cmd = self.build_command(input);
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

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(CompletionResponse {
            content: stdout,
            model: self.command.clone(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
        })
    }

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        let input = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

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
                let _ = tx.send(Err(anyhow::anyhow!("CLI command timed out"))).await;
                let _ = child.kill().await;
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
}
