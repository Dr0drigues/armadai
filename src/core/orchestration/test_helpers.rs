//! Shared test utilities for orchestration tests.
//!
//! Provides a common `NoopProvider` (returns fixed responses) and helper
//! functions for constructing `Board` and `RingToken` instances.

use std::sync::Arc;

use async_trait::async_trait;

use crate::providers::traits::{
    CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
};

use super::blackboard::Board;
use super::ring::RingToken;

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

/// Create a board with sensible test defaults.
pub fn test_board(task: &str, token_budget: u64) -> Board {
    Board::new(task.to_string(), token_budget)
}

/// Create a ring token with sensible test defaults.
pub fn test_ring_token(task: &str, agents: &[&str], token_budget: u64) -> RingToken {
    let order = agents.iter().map(|a| a.to_string()).collect();
    RingToken::new(task.to_string(), order, token_budget)
}
