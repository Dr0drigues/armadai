use async_trait::async_trait;

use crate::providers::traits::*;

pub struct OpenAiProvider {
    pub api_key: String,
    pub base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn complete(&self, _request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        todo!("OpenAI completion")
    }

    async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
        todo!("OpenAI streaming")
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "openai".to_string(),
            models: vec![
                "gpt-4o".to_string(),
                "gpt-4o-mini".to_string(),
                "o1".to_string(),
            ],
            supports_streaming: true,
        }
    }
}
