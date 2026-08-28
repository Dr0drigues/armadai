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

// Fields are constructed by every `Provider::metadata()` impl but only read
// in tests today (`cli/run.rs`). Previously silent because `mod providers;`
// in `main.rs` carries a blanket `#[allow(dead_code)]`; `core` has no such
// blanket allow, so the move surfaces this pre-existing dead code. Scoped
// here rather than widening `core`'s allow — same effective behavior as
// before the move (OH7 #252 Task 1d, pure refactor, no allow scope change).
#[allow(dead_code)]
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

    /// Whether [`CompletionRequest::model`] actually selects the model this
    /// provider answers with.
    ///
    /// `true` for everything that speaks an HTTP API: the field is the model
    /// name in the request body or URL. `false` for a provider that relays a
    /// command-line tool, which owns its own model choice — `CliProvider`
    /// spawns the binary and never passes the field on, and reports back the
    /// command name (or whatever the tool's own JSON says it used).
    ///
    /// Only a preview needs to ask. `armadai run --dry-run` was answering
    /// "which model will I pay for" with the agent's resolved model id for
    /// every agent, including CLI-relayed ones where that id is never sent
    /// and never billed (#398 review, F2) — and a CLI-relayed agent is
    /// ArmadAI's reference configuration. Defaulted to `true` so an API
    /// provider states nothing, and the one exception is where it belongs.
    fn honors_request_model(&self) -> bool {
        true
    }
}
