use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_stream::Stream;

/// A stream of tokens from a model response.
pub type TokenStream = std::pin::Pin<Box<dyn Stream<Item = anyhow::Result<String>> + Send>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost: f64,
}

#[derive(Debug, Clone)]
pub struct ProviderMetadata {
    pub name: String,
    pub models: Vec<String>,
    pub supports_streaming: bool,
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a prompt and return the full response.
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse>;

    /// Send a prompt and return a stream of tokens.
    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream>;

    /// Return provider metadata.
    fn metadata(&self) -> ProviderMetadata;
}
