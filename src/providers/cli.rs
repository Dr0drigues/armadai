use async_trait::async_trait;
use tokio::process::Command;
use tokio_stream::StreamExt;

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
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            cmd.output(),
        )
        .await??;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            anyhow::bail!("CLI command failed ({}): {stderr}", output.status);
        }

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

        let mut cmd = self.build_command(&input);
        let mut child = cmd.spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;

        let reader = tokio::io::BufReader::new(stdout);
        let lines = tokio::io::AsyncBufReadExt::lines(reader);
        let stream = tokio_stream::wrappers::LinesStream::new(lines);

        Ok(Box::pin(stream.map(|line: Result<String, std::io::Error>| {
            line.map_err(|e| anyhow::anyhow!(e))
        })))
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: format!("cli:{}", self.command),
            models: vec![self.command.clone()],
            supports_streaming: true,
        }
    }
}
