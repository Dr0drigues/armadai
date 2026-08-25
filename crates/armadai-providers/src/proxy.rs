//! Any OpenAI-compatible endpoint that isn't `api.openai.com`.
//!
//! Gateways (LiteLLM, OpenRouter, Groq, Together, Fireworks, DeepSeek,
//! Mistral) and local runtimes (Ollama, vLLM, LM Studio, llama.cpp) all
//! speak the same chat-completions dialect, so this provider is exactly
//! [`crate::api::openai`] with two differences: the base URL is supplied by
//! the user (it has no sensible universal default), and the API key is
//! **optional** — a gateway on localhost usually has none, and sending an
//! empty `Bearer ` is worse than sending no header at all.
//!
//! The protocol itself lives in [`crate::api::openai_compatible`] and is
//! shared verbatim; see that module for the wire-level tolerances and for
//! what is deliberately unsupported (Azure OpenAI's `api-key` +
//! `api-version` shape — reachable through a gateway instead).

use async_trait::async_trait;
use reqwest::Client;

use armadai_core::provider::*;

use crate::api::openai_compatible::Endpoint;

/// Proxy provider that routes through any OpenAI-compatible server.
pub struct ProxyProvider {
    pub base_url: String,
    pub api_key: Option<String>,
    client: Client,
}

impl ProxyProvider {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            base_url,
            api_key,
            client: Client::new(),
        }
    }

    fn endpoint(&self) -> Endpoint<'_> {
        Endpoint {
            client: &self.client,
            base_url: &self.base_url,
            api_key: self.api_key.as_deref(),
            label: "Proxy",
        }
    }
}

#[async_trait]
impl Provider for ProxyProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        self.endpoint().complete(request).await
    }

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        self.endpoint().stream(request).await
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: "proxy".to_string(),
            models: vec![],
            supports_streaming: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_server::{ScriptedResponse, ScriptedServer};
    use tokio_stream::StreamExt;

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "openai/gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
            }],
            temperature: 0.2,
            max_tokens: None,
        }
    }

    const OK_BODY: &str = r#"{"model":"openai/gpt-4o-mini",
        "choices":[{"message":{"content":"routed"}}],
        "usage":{"prompt_tokens":3,"completion_tokens":2}}"#;

    /// A keyless gateway (a local LiteLLM, an Ollama server) must receive NO
    /// `Authorization` header at all — an empty `Bearer ` is rejected by
    /// several servers and is worse than sending nothing.
    #[tokio::test]
    async fn a_keyless_proxy_sends_no_authorization_header() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, OK_BODY)]);
        let provider = ProxyProvider::new(server.url(), None);

        let resp = provider.complete(request()).await.expect("keyless call");
        assert_eq!(resp.content, "routed");

        let raw = server.request(0).expect("one request received");
        assert!(
            !raw.to_lowercase().contains("authorization"),
            "a keyless proxy must send no Authorization header, got:\n{raw}"
        );
    }

    #[tokio::test]
    async fn a_proxy_with_a_key_sends_it_as_a_bearer_token() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, OK_BODY)]);
        let provider = ProxyProvider::new(server.url(), Some("sk-gateway".to_string()));

        provider.complete(request()).await.expect("keyed call");

        let raw = server.request(0).expect("one request received");
        assert!(
            raw.to_lowercase()
                .contains("authorization: bearer sk-gateway"),
            "missing bearer header in:\n{raw}"
        );
    }

    /// The proxy speaks the same protocol as `openai` — same endpoint, same
    /// parsing, same token/cost extraction — because it runs the same code.
    #[tokio::test]
    async fn proxy_speaks_the_same_openai_protocol() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, OK_BODY)]);
        let provider = ProxyProvider::new(server.url(), None);

        let resp = provider.complete(request()).await.expect("call");
        assert_eq!(resp.model, "openai/gpt-4o-mini");
        assert_eq!(resp.tokens_in, 3);
        assert_eq!(resp.tokens_out, 2);
        // A vendor-prefixed OpenRouter-style id still prices as gpt-4o-mini.
        let expected = (3.0 * 0.15 + 2.0 * 0.60) / 1_000_000.0;
        assert!((resp.cost - expected).abs() < f64::EPSILON, "{}", resp.cost);

        let raw = server.request(0).expect("one request received");
        assert!(raw.contains("POST /chat/completions "), "in:\n{raw}");
    }

    #[tokio::test]
    async fn proxy_streams_sse_like_openai_does() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"pro\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"xied\"}}]}\n\n",
                "data: [DONE]\n\n",
            )],
        )]);
        let provider = ProxyProvider::new(server.url(), None);

        let mut stream = provider.stream(request()).await.expect("stream");
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            out.push(item.expect("stream item"));
        }
        assert_eq!(out, vec!["pro", "xied"]);
    }

    #[tokio::test]
    async fn a_gateway_error_body_is_reported_verbatim() {
        // vLLM / FastAPI-style error shape: no nested `error` object.
        let server = ScriptedServer::start(vec![ScriptedResponse::body(
            404,
            r#"{"object":"error","message":"The model `nope` does not exist.","type":"NotFoundError"}"#,
        )]);
        let provider = ProxyProvider::new(server.url(), None);

        let err = provider.complete(request()).await.expect_err("404");
        assert!(
            err.to_string().contains("The model `nope` does not exist."),
            "got: {err}"
        );
    }

    #[test]
    fn metadata_declares_streaming_and_it_is_true() {
        let meta = ProxyProvider::new("http://localhost:4000/v1".to_string(), None).metadata();
        assert_eq!(meta.name, "proxy");
        assert!(meta.supports_streaming);
    }
}
