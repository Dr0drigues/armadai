use async_trait::async_trait;

use crate::providers::traits::*;

pub struct AnthropicProvider {
    pub api_key: String,
    pub base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.anthropic.com/v1".to_string(),
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn complete(&self, _request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        todo!("Anthropic completion")
    }

    async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
        todo!("Anthropic streaming")
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "anthropic".to_string(),
            models: vec![
                "claude-opus-4-6".to_string(),
                "claude-sonnet-4-5-20250929".to_string(),
                "claude-haiku-4-5-20251001".to_string(),
            ],
            supports_streaming: true,
        }
    }
}
