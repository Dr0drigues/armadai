//! OpenAI's own chat-completions endpoint.
//!
//! The protocol lives in [`super::openai_compatible`] and is shared
//! verbatim with [`crate::proxy`]; this file only carries OpenAI's
//! defaults (public base URL, a required API key, the advertised model
//! list). See that module for what is and isn't tolerated on the wire, and
//! for why Azure OpenAI is deliberately not handled here.

use async_trait::async_trait;
use reqwest::Client;

use armadai_core::provider::*;

use super::openai_compatible::Endpoint;

pub struct OpenAiProvider {
    pub api_key: String,
    pub base_url: String,
    client: Client,
}

impl OpenAiProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            client: Client::new(),
        }
    }

    fn endpoint(&self) -> Endpoint<'_> {
        Endpoint {
            client: &self.client,
            base_url: &self.base_url,
            api_key: Some(&self.api_key),
            label: "OpenAI",
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        self.endpoint().complete(request).await
    }

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        self.endpoint().stream(request).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_server::{ScriptedResponse, ScriptedServer};
    use tokio_stream::StreamExt;

    fn provider_at(server: &ScriptedServer) -> OpenAiProvider {
        let mut p = OpenAiProvider::new("sk-test-key".to_string());
        p.base_url = server.url();
        p
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "gpt-4o-mini".to_string(),
            system_prompt: "You are terse.".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
            }],
            temperature: 0.3,
            max_tokens: Some(256),
        }
    }

    async fn collect(stream: TokenStream) -> Vec<String> {
        let mut out = Vec::new();
        let mut stream = stream;
        while let Some(item) = stream.next().await {
            out.push(item.expect("stream item"));
        }
        out
    }

    // --- complete() ---

    #[tokio::test]
    async fn complete_parses_a_nominal_chat_completion() {
        let body = r#"{
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "model": "gpt-4o-mini-2024-07-18",
            "choices": [
                {"index": 0, "message": {"role": "assistant", "content": "Bonjour"},
                 "finish_reason": "stop"}
            ],
            "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}
        }"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, body)]);

        let resp = provider_at(&server)
            .complete(request())
            .await
            .expect("nominal completion");

        assert_eq!(resp.content, "Bonjour");
        assert_eq!(resp.model, "gpt-4o-mini-2024-07-18");
        assert_eq!(resp.tokens_in, 11);
        assert_eq!(resp.tokens_out, 7);
        // gpt-4o-mini: $0.15/M in, $0.60/M out.
        let expected = (11.0 * 0.15 + 7.0 * 0.60) / 1_000_000.0;
        assert!(
            (resp.cost - expected).abs() < f64::EPSILON,
            "cost {} != {expected}",
            resp.cost
        );
    }

    /// Many OpenAI-compatible gateways and local runtimes omit `usage`
    /// entirely (or send `usage: null`). That must cost the caller nothing
    /// but the counters — never the whole call.
    #[tokio::test]
    async fn complete_without_a_usage_block_still_returns_the_content() {
        let body = r#"{
            "model": "llama3.1",
            "choices": [{"message": {"content": "local answer"}}]
        }"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, body)]);

        let resp = provider_at(&server)
            .complete(request())
            .await
            .expect("a response with no usage block must still succeed");

        assert_eq!(resp.content, "local answer");
        assert_eq!(resp.tokens_in, 0);
        assert_eq!(resp.tokens_out, 0);
        assert_eq!(resp.cost, 0.0);
    }

    /// `usage: null`, no `id`, no `finish_reason`, no `system_fingerprint`:
    /// the shapes gateways actually send. Also proves an unknown model id
    /// reports a zero cost rather than a fabricated one.
    #[tokio::test]
    async fn complete_tolerates_null_usage_and_missing_optional_fields() {
        let body = r#"{
            "model": "mistral-small-latest",
            "usage": null,
            "choices": [{"message": {"role": "assistant", "content": "ok"}}]
        }"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, body)]);

        let resp = provider_at(&server).complete(request()).await.expect("ok");
        assert_eq!(resp.content, "ok");
        assert_eq!(resp.model, "mistral-small-latest");
        assert_eq!(resp.cost, 0.0);
    }

    /// A 200 with no `choices` at all (a filtered answer, a gateway
    /// hiccup): empty content, not a failed call.
    #[tokio::test]
    async fn complete_with_no_choices_returns_empty_content_not_an_error() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(
            200,
            r#"{"model":"gpt-4o","usage":{"prompt_tokens":4,"completion_tokens":0}}"#,
        )]);

        let resp = provider_at(&server).complete(request()).await.expect("ok");
        assert_eq!(resp.content, "");
        assert_eq!(resp.tokens_in, 4);
    }

    /// A server that omits `model` altogether: fall back to what we asked
    /// for rather than reporting an empty model.
    #[tokio::test]
    async fn complete_falls_back_to_the_requested_model_when_none_is_echoed() {
        let body = r#"{"choices": [{"message": {"content": "x"}}]}"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, body)]);

        let resp = provider_at(&server).complete(request()).await.expect("ok");
        assert_eq!(resp.model, "gpt-4o-mini");
    }

    #[tokio::test]
    async fn an_http_error_surfaces_the_servers_own_message() {
        let body = r#"{"error": {"message": "Unsupported parameter: 'max_tokens'",
                                  "type": "invalid_request_error", "code": null}}"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(400, body)]);

        let err = provider_at(&server)
            .complete(request())
            .await
            .expect_err("a 400 must be an error, not a panic");
        let msg = err.to_string();
        assert!(
            msg.contains("Unsupported parameter: 'max_tokens'"),
            "error must carry the server's message, got: {msg}"
        );
        assert!(
            msg.contains("400"),
            "error must name the status, got: {msg}"
        );
    }

    /// The request actually put on the wire: a `Bearer` key, the system
    /// prompt as the leading `system` message, and no `stream` flag.
    #[tokio::test]
    async fn the_request_carries_a_bearer_key_and_the_system_prompt() {
        let body = r#"{"choices": [{"message": {"content": "x"}}]}"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, body)]);

        provider_at(&server).complete(request()).await.expect("ok");

        let raw = server.request(0).expect("one request received");
        assert!(
            raw.contains("authorization: Bearer sk-test-key")
                || raw.contains("Authorization: Bearer sk-test-key"),
            "missing bearer header in:\n{raw}"
        );
        assert!(
            raw.contains("POST /chat/completions "),
            "wrong path in:\n{raw}"
        );
        assert!(
            raw.contains(r#""role":"system","content":"You are terse.""#),
            "system prompt not sent as a system message in:\n{raw}"
        );
        assert!(
            raw.contains(r#""max_tokens":256"#),
            "max_tokens not forwarded in:\n{raw}"
        );
    }

    /// `max_tokens` is rejected outright by some models (o1/o3) and some
    /// gateways, so it must only travel when the agent actually asked for it.
    #[tokio::test]
    async fn an_unset_max_tokens_is_not_sent_at_all() {
        let body = r#"{"choices": [{"message": {"content": "x"}}]}"#;
        let server = ScriptedServer::start(vec![ScriptedResponse::body(200, body)]);

        let mut req = request();
        req.max_tokens = None;
        provider_at(&server).complete(req).await.expect("ok");

        let raw = server.request(0).expect("one request received");
        assert!(
            !raw.contains("max_tokens"),
            "max_tokens must be absent when unset, found in:\n{raw}"
        );
    }

    /// The 429/503/529 handling from `api::retry` (#358) must apply here
    /// too — this path talks to far more servers than the two first-party
    /// APIs, so it meets rate limits more often, not less. The server
    /// answers 429 once and then 200; a single request count would mean the
    /// call never went through `send_with_retry`.
    #[tokio::test]
    async fn complete_retries_a_rate_limited_response_through_the_shared_policy() {
        let server = ScriptedServer::start(vec![
            ScriptedResponse::body(429, r#"{"error":{"message":"slow down"}}"#),
            ScriptedResponse::body(
                200,
                r#"{"choices":[{"message":{"content":"after retry"}}]}"#,
            ),
        ]);

        let resp = provider_at(&server)
            .complete(request())
            .await
            .expect("the retried attempt must succeed");

        assert_eq!(resp.content, "after retry");
        assert_eq!(
            server.request_count(),
            2,
            "a 429 must be retried, not surfaced as an error"
        );
    }

    // --- stream() ---

    /// The case hand-written SSE readers get wrong: one `data:` line split
    /// across two TCP packets. The server writes each chunk separately, so
    /// the halves genuinely arrive as separate reads.
    #[tokio::test]
    async fn stream_reassembles_a_data_line_split_across_two_packets() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel",
                "lo\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\ndata: [DONE]\n\n",
            ],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert_eq!(collect(stream).await, vec!["Hello", " world"]);
    }

    /// A multi-byte UTF-8 character split across two TCP packets. Decoding
    /// each chunk as it arrives (what `anthropic.rs`/`google.rs` do) turns
    /// the halves into replacement characters; buffering bytes until a whole
    /// event is in hand does not. French output makes this a routine case,
    /// not an exotic one.
    #[tokio::test]
    async fn stream_reassembles_a_multibyte_character_split_across_packets() {
        // "é" is 0xC3 0xA9 — the packet boundary falls between the two bytes.
        let first_half = b"data: {\"choices\":[{\"delta\":{\"content\":\"caf\xc3";
        let second_half = b"\xa9\"}}]}\n\ndata: [DONE]\n\n";
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![
                // Safety of the lossy conversion here is irrelevant: the
                // server writes the bytes back out verbatim, and what is
                // being measured is the CLIENT's reassembly.
                unsafe { String::from_utf8_unchecked(first_half.to_vec()) },
                unsafe { String::from_utf8_unchecked(second_half.to_vec()) },
            ],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert_eq!(collect(stream).await, vec!["café"]);
    }

    /// `data: [DONE]` ends the stream. Anything a chatty gateway appends
    /// after it must not reach the caller.
    #[tokio::test]
    async fn stream_stops_at_the_terminal_done_marker() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
                "data: [DONE]\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"AFTER\"}}]}\n\n",
            )],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert_eq!(collect(stream).await, vec!["a"]);
    }

    /// Empty `choices` on a chunk (Azure's first frame, several gateways'
    /// usage-only trailer) must be skipped, not indexed into.
    #[tokio::test]
    async fn stream_skips_chunks_with_no_choices_and_null_content() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![concat!(
                "data: {\"choices\":[]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":null}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"only this\"}}]}\n\n",
                "data: [DONE]\n\n",
            )],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert_eq!(collect(stream).await, vec!["only this"]);
    }

    /// A final event with no trailing blank line before the connection
    /// closes — several servers end that way — plus the `data:` spelling
    /// with no space after the colon.
    #[tokio::test]
    async fn stream_delivers_a_last_event_that_has_no_trailing_blank_line() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\n\n",
                "data:{\"choices\":[{\"delta\":{\"content\":\"two\"}}]}",
            )],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert_eq!(collect(stream).await, vec!["one", "two"]);
    }

    /// CRLF-framed events must be recognised **as they arrive**, not
    /// rescued at EOF by the end-of-stream flush.
    ///
    /// This has to be a timing assertion, and the bound comes from the
    /// defect rather than from the nominal time: searching only for `\n\n`
    /// (what `anthropic.rs`/`google.rs` do) leaves a CRLF stream buffered
    /// until the connection closes, so the first token would arrive only
    /// after the whole 21-chunk tail — the server pauses 30ms between
    /// chunks, so ~630ms. Correct framing emits it as soon as chunk 0
    /// lands. The 250ms bound sits well below the defect's floor and well
    /// above the correct behaviour's cost. Asserting the *content* alone
    /// cannot fail here: with the framing broken the same tokens still come
    /// out at EOF, only later — measured, see the report for #368.
    #[tokio::test]
    async fn stream_emits_a_crlf_framed_event_before_the_connection_closes() {
        let mut chunks =
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\r\n\r\n".to_string()];
        // SSE comments: valid frames carrying no token, so only the timing
        // of the first token is under test.
        chunks.extend(std::iter::repeat_n(": keep-alive\r\n\r\n".to_string(), 20));
        chunks.push("data: [DONE]\r\n\r\n".to_string());
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(200, chunks)]);

        let mut stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        let started = std::time::Instant::now();
        let first = stream.next().await.expect("a first token").expect("ok");
        let elapsed = started.elapsed();

        assert_eq!(first, "one");
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "first token took {elapsed:?}: a CRLF-framed event was buffered until EOF"
        );
    }

    #[tokio::test]
    async fn stream_reports_an_http_error_instead_of_streaming() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(
            401,
            r#"{"error": {"message": "Incorrect API key provided"}}"#,
        )]);

        let err = match provider_at(&server).stream(request()).await {
            Ok(_) => panic!("a 401 must fail before any token is produced"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("Incorrect API key provided"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn stream_sets_the_stream_flag_on_the_request() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec!["data: [DONE]\n\n"],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert!(collect(stream).await.is_empty());

        let raw = server.request(0).expect("one request received");
        assert!(raw.contains(r#""stream":true"#), "in:\n{raw}");
    }

    #[test]
    fn metadata_declares_streaming_and_it_is_true() {
        let meta = OpenAiProvider::new("k".to_string()).metadata();
        assert_eq!(meta.name, "openai");
        assert!(meta.supports_streaming);
    }
}
