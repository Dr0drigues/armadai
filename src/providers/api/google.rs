use async_trait::async_trait;

use crate::providers::traits::*;

pub struct GoogleProvider {
    pub api_key: String,
    pub base_url: String,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
        }
    }
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn complete(&self, _request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        todo!("Google AI completion")
    }

    async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
        todo!("Google AI streaming")
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "google".to_string(),
            models: vec![
                "gemini-2.0-flash".to_string(),
                "gemini-2.0-pro".to_string(),
            ],
            supports_streaming: true,
        }
    }
}
