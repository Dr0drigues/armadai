use async_trait::async_trait;

use super::traits::*;

/// Proxy provider that routes through LiteLLM or OpenRouter (OpenAI-compatible API).
pub struct ProxyProvider {
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ProxyProvider {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self { base_url, api_key }
    }
}

#[async_trait]
impl Provider for ProxyProvider {
    async fn complete(&self, _request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        todo!("Proxy completion (OpenAI-compatible)")
    }

    async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
        todo!("Proxy streaming (OpenAI-compatible)")
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "proxy".to_string(),
            models: vec![],
            supports_streaming: true,
        }
    }
}
