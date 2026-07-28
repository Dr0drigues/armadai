//! Shared test utilities for orchestration tests.
//!
//! Provides a common `NoopProvider` (returns fixed responses).

use std::sync::Arc;

use async_trait::async_trait;

use crate::core::provider::{
    CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
};

/// A no-op provider that returns fixed responses with zero cost.
///
/// Used in unit / integration tests where provider behaviour is irrelevant.
pub struct NoopProvider;

#[async_trait]
impl Provider for NoopProvider {
    async fn complete(&self, _: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        Ok(CompletionResponse {
            content: "ok".to_string(),
            model: "noop".to_string(),
            tokens_in: 10,
            tokens_out: 10,
            cost: 0.0,
        })
    }
    async fn stream(&self, _: CompletionRequest) -> anyhow::Result<TokenStream> {
        unimplemented!()
    }
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "noop".to_string(),
            models: vec![],
            supports_streaming: false,
        }
    }
}

/// Create a single-provider vec ready for orchestration tests.
pub fn noop_providers() -> Vec<Arc<dyn Provider>> {
    vec![Arc::new(NoopProvider)]
}
