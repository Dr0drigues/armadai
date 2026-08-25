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

    /// The defect this guards: a `200` whose body is an error envelope.
    /// Every field of the response type is optional, so such a body parses
    /// into an empty completion and the run is recorded as successful, empty
    /// and free — `Ok(CompletionResponse { content: "", tokens_in: 0, cost:
    /// 0.0 })`. Ollama and LiteLLM in pass-through mode both answer this way.
    ///
    /// `anthropic`/`google` are protected here by accident: their required
    /// `content`/`candidates` fields make the same body a parse failure. This
    /// path had to be told.
    #[tokio::test]
    async fn complete_rejects_a_success_status_carrying_an_error_envelope() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(
            200,
            r#"{"error":{"message":"Rate limit exceeded","type":"rate_limit"}}"#,
        )]);

        let err = match provider_at(&server).complete(request()).await {
            Ok(resp) => panic!(
                "an error envelope was reported as a successful empty run: \
                 content={:?} tokens_in={} cost={}",
                resp.content, resp.tokens_in, resp.cost
            ),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("Rate limit exceeded"), "got: {err}");
    }

    /// The other side of the same check: a `200` that legitimately carries
    /// no `choices` (a filtered answer, a usage-only body) must stay a
    /// success. `is_error_body` is what separates the two, so this is the
    /// test that would catch it being widened into "any empty response is an
    /// error".
    #[tokio::test]
    async fn complete_still_accepts_an_empty_body_that_is_not_an_error() {
        let server = ScriptedServer::start(vec![ScriptedResponse::body(
            200,
            r#"{"model":"gpt-4o","error":null,"usage":{"prompt_tokens":4,"completion_tokens":0}}"#,
        )]);

        let resp = provider_at(&server)
            .complete(request())
            .await
            .expect("an empty-but-successful body must not be rejected");
        assert_eq!(resp.content, "");
        assert_eq!(resp.tokens_in, 4);
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

    /// CRLF-framed events must be recognised **as they arrive**, not rescued
    /// at EOF by the end-of-stream flush.
    ///
    /// Categorical, not a timing measurement: the server writes the first
    /// event and then *holds the connection open*, sending neither more data
    /// nor EOF until `release()`. A reader that searches only for `\n\n`
    /// (what `anthropic.rs`/`google.rs` do) therefore emits nothing at all
    /// rather than the same tokens a little later, and the `timeout` below
    /// is an anti-hang guard rather than a threshold to tune. The previous
    /// version of this test asserted `elapsed < 250ms` against a 21-chunk
    /// tail; it held (worst case 161ms over 1150 runs) but spent 64% of its
    /// budget, and a bound that can be approached is a bound that can be
    /// crossed on a loaded machine.
    #[tokio::test]
    async fn stream_emits_a_crlf_framed_event_while_the_connection_is_open() {
        let server = ScriptedServer::start_gated(vec![ScriptedResponse::streamed(
            200,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"one\"}}]}\r\n\r\n",
                "data: [DONE]\r\n\r\n",
            ],
        )]);

        let mut stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("a CRLF-framed event was buffered instead of emitted on arrival")
            .expect("a first token")
            .expect("ok");
        assert_eq!(first, "one");
        server.release();
    }

    /// A CR-only stream: legal SSE, and a third framing a reader can be
    /// blind to. Same categorical shape as the CRLF case above — while the
    /// gate is shut there is no EOF to rescue an unrecognised boundary.
    #[tokio::test]
    async fn stream_emits_a_cr_framed_event_while_the_connection_is_open() {
        let server = ScriptedServer::start_gated(vec![ScriptedResponse::streamed(
            200,
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"cr\"}}]}\r\r",
                "data: [DONE]\r\r",
            ],
        )]);

        let mut stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("a CR-framed event was never recognised as an event")
            .expect("a first token")
            .expect("ok");
        assert_eq!(first, "cr");
        server.release();
    }

    /// One `data` field spread over two lines — the spec's own spelling for
    /// a multi-line payload, joined with `\n`. Read line by line, each half
    /// is invalid JSON and both are dropped in silence.
    #[tokio::test]
    async fn stream_joins_a_data_field_split_across_two_lines() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![concat!(
                "data: {\"choices\":[{\"delta\":\n",
                "data: {\"content\":\"joined\"}}]}\n\n",
                "data: [DONE]\n\n",
            )],
        )]);

        let stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        assert_eq!(collect(stream).await, vec!["joined"]);
    }

    /// The defect this guards: an error frame arriving **after** the stream
    /// has started. The status line said 200 long ago, so SSE is the only
    /// channel left — this is OpenAI's documented behaviour for anything
    /// that fails mid-stream. Dropped silently, it delivers a truncated
    /// answer as a complete one.
    #[tokio::test]
    async fn stream_surfaces_an_error_frame_instead_of_ending_early() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"partial ans\"}}]}\n\n",
                "data: {\"error\":{\"message\":\"upstream timed out\",\"type\":\"server_error\"}}\n\n",
            )],
        )]);

        let mut stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");

        let first = stream
            .next()
            .await
            .expect("a first token")
            .expect("the first delta is fine");
        assert_eq!(first, "partial ans");

        let second = stream
            .next()
            .await
            .expect("the error frame must reach the caller, not end the stream");
        let err = second.expect_err("the error frame must arrive as an Err");
        assert!(err.to_string().contains("upstream timed out"), "got: {err}");
    }

    /// A streaming request answered with a plain JSON error body and a 200
    /// status — no SSE framing at all. Ollama and LiteLLM in pass-through do
    /// this. With no `data:` line there is nothing to parse, so the stream
    /// would simply end empty and successful.
    #[tokio::test]
    async fn stream_surfaces_an_unframed_error_body_sent_with_a_200() {
        let server = ScriptedServer::start(vec![ScriptedResponse::streamed(
            200,
            vec![r#"{"error":{"message":"model 'nope' not found"}}"#],
        )]);

        let mut stream = provider_at(&server)
            .stream(request())
            .await
            .expect("stream");
        let item = stream
            .next()
            .await
            .expect("an error body must not read as an empty answer");
        let err = item.expect_err("must be an Err");
        assert!(
            err.to_string().contains("model 'nope' not found"),
            "got: {err}"
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
