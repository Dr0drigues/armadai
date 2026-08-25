//! The OpenAI chat-completions protocol, implemented once.
//!
//! `POST {base_url}/chat/completions` — plus Server-Sent Events for the
//! streaming variant — is not one vendor's API but the de-facto lingua
//! franca of LLM serving. The same wire format is spoken by OpenAI itself,
//! by gateways (LiteLLM, OpenRouter, Groq, Together, Fireworks, DeepSeek,
//! Mistral) and by local runtimes (Ollama, vLLM, LM Studio, llama.cpp).
//! `api::openai` and `proxy` differ only in their defaults and in whether
//! an API key exists at all, so they share this module rather than each
//! growing a copy — two implementations of one protocol is the defect class
//! this repo has closed repeatedly.
//!
//! ## Written for the dialect, not for `api.openai.com`
//!
//! Every optional field is treated as optional, because in practice it is:
//!
//! - `usage` is routinely **absent**, or present as `null`, on gateways and
//!   local runtimes. That costs the caller its token counters, never the
//!   call: the response still parses and reports zeros.
//! - `id`, `object`, `system_fingerprint`, `finish_reason`, `created` are
//!   never read, so a server that omits them is fine.
//! - `model` may be absent (llama.cpp) — we then report the model that was
//!   *asked for* rather than an empty string.
//! - `choices` may be **empty** on a streaming chunk (Azure's leading
//!   content-filter frame; the usage-only trailer several gateways append),
//!   and `delta.content` may be `null`. Both are skipped, never indexed.
//! - SSE events may be separated by `\n\n` **or** `\r\n\r\n`, the `data:`
//!   prefix may or may not be followed by a space, and the final event may
//!   arrive with no trailing blank line before the connection closes.
//! - The response body is buffered as **bytes** and only decoded once a
//!   whole event is in hand, so a multi-byte UTF-8 character split across
//!   two TCP packets is reassembled instead of being mangled into
//!   replacement characters.
//! - `max_tokens` is sent **only** when the caller set one. Reasoning
//!   models (`o1`, `o3`) reject it outright, and some gateways reject
//!   unknown or unsupported parameters wholesale, so an unrequested
//!   parameter is a compatibility liability, not a harmless default.
//! - `stream_options: {include_usage: true}` is deliberately **not** sent
//!   for the same reason: it would buy usage numbers on OpenAI proper at
//!   the price of a 400 from every server that doesn't know the field.
//!
//! ## Deliberately not supported
//!
//! **Azure OpenAI** authenticates with an `api-key` header (not
//! `Authorization: Bearer`), addresses deployments rather than models
//! (`/openai/deployments/{deployment}/chat/completions`) and requires an
//! `api-version` query parameter. That is a different enough shape that
//! bolting it onto this path would mean branching on the vendor in three
//! places to serve one of them. It is documented as unsupported in
//! `docs/wiki/providers.md` rather than half-wired and left to fail at
//! runtime. Azure users can still reach ArmadAI through a gateway
//! (LiteLLM) that presents an OpenAI-shaped front — that is exactly what
//! `proxy` is for.
//!
//! Tool/function calling, structured outputs, images and audio are out of
//! scope: `CompletionRequest`/`CompletionResponse` carry plain text, so
//! there is nothing in the domain model for them to map onto.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tokio_stream::StreamExt;

use armadai_core::provider::*;

use super::retry::{RetryPolicy, send_with_retry};

/// Terminal marker of an OpenAI-style SSE stream.
const DONE_MARKER: &str = "[DONE]";

/// Longest raw (unparsed) error body echoed back to the user. A gateway
/// that answers with an HTML error page or a stack trace should not paste
/// the whole thing into a terminal.
const MAX_RAW_ERROR_CHARS: usize = 500;

/// What a caller sees when the server sends an error status and no body at
/// all — better than an error line that trails off after the status code.
const NO_BODY: &str = "no response body";

/// One configured OpenAI-compatible endpoint. Borrowed, not owned: the
/// provider structs (`OpenAiProvider`, `ProxyProvider`) stay the owners of
/// their key/URL/client so the factory can keep mutating `base_url` the way
/// it already does for `anthropic` and `google`.
pub(crate) struct Endpoint<'a> {
    pub(crate) client: &'a Client,
    pub(crate) base_url: &'a str,
    /// `None` means *send no `Authorization` header at all*. A keyless
    /// gateway (a local LiteLLM, an Ollama server) is a first-class case,
    /// and an empty `Bearer ` is actively worse than nothing — several
    /// servers reject it as malformed instead of ignoring it.
    pub(crate) api_key: Option<&'a str>,
    /// Name used in error messages (`"OpenAI"`, `"Proxy"`).
    pub(crate) label: &'a str,
}

// --- API request/response types ---

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatRequestMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize)]
struct ChatRequestMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    /// `#[serde(default)]` is load-bearing here and only here: a `Vec` is a
    /// required field without it, so a server that omits `choices`
    /// altogether would fail the whole response. (`Option` fields below are
    /// already optional to serde, and already accept an explicit `null`;
    /// the attribute on them is documentation, not behaviour.)
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatChoiceMessage>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    /// `null` whenever the model answered with tool calls instead of text.
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

// --- Cost calculation ---

/// Public list prices per million tokens, keyed on the model id with any
/// vendor prefix stripped (`openai/gpt-4o` on OpenRouter is still
/// `gpt-4o`).
///
/// An **unknown** id yields `None`, and the caller then reports a cost of
/// `0.0`. That is deliberate and different from `anthropic.rs`/`google.rs`,
/// which fall back to their mid-tier price: those two only ever talk to one
/// vendor, so a guess is bounded. This path can be pointed at Ollama
/// serving Llama, or at a gateway serving anything at all — pricing an
/// unknown model as if it were GPT-4o would invent dollars that were never
/// spent and feed them into `armadai costs`. A visible `$0.00` is a
/// recognisable "not priced", a wrong number is not.
fn rates_for_model(model: &str) -> Option<(f64, f64)> {
    let id = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    // Most specific prefix first: `gpt-4o-mini` also starts with `gpt-4o`.
    Some(match id.as_str() {
        m if m.starts_with("gpt-4o-mini") => (0.15, 0.60),
        m if m.starts_with("gpt-4o") => (2.50, 10.00),
        m if m.starts_with("gpt-4.1-nano") => (0.10, 0.40),
        m if m.starts_with("gpt-4.1-mini") => (0.40, 1.60),
        m if m.starts_with("gpt-4.1") => (2.00, 8.00),
        m if m.starts_with("o1-mini") => (1.10, 4.40),
        m if m.starts_with("o1") => (15.00, 60.00),
        m if m.starts_with("o3-mini") => (1.10, 4.40),
        _ => return None,
    })
}

fn cost_for_model(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    match rates_for_model(model) {
        Some((input_rate, output_rate)) => {
            (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0
        }
        None => 0.0,
    }
}

// --- Error bodies ---

/// Pull a human-readable message out of an error body, across the shapes
/// this dialect's servers actually produce:
///
/// - `{"error": {"message": "..."}}` — OpenAI, LiteLLM, OpenRouter, Groq
/// - `{"error": "..."}` — Ollama
/// - `{"message": "...", "object": "error"}` — vLLM
/// - `{"detail": "..."}` — bare FastAPI wrappers
///
/// Anything else falls back to the raw body, truncated, because a
/// truncated real answer still tells the user more than "unknown error".
fn error_message_from_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return NO_BODY.to_string();
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let candidates = [
            value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str()),
            value.get("error").and_then(|e| e.as_str()),
            value.get("message").and_then(|m| m.as_str()),
            value.get("detail").and_then(|m| m.as_str()),
        ];
        for candidate in candidates.into_iter().flatten() {
            if !candidate.trim().is_empty() {
                return candidate.to_string();
            }
        }
    }

    truncate_chars(trimmed, MAX_RAW_ERROR_CHARS)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}… (truncated)")
}

// --- SSE parsing ---

/// Offset and separator length of the first complete SSE event boundary,
/// or `None` while the buffer still holds a partial event.
///
/// Both `\n\n` and `\r\n\r\n` are accepted: the SSE spec allows either, and
/// searching only for `\n\n` (what `anthropic.rs`/`google.rs` do — safe
/// against those two servers) would buffer a CRLF stream forever.
fn find_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = find_subslice(buffer, b"\r\n\r\n");
    let lf = find_subslice(buffer, b"\n\n");
    match (crlf, lf) {
        (Some(c), Some(l)) => {
            if c <= l {
                Some((c, 4))
            } else {
                Some((l, 2))
            }
        }
        (Some(c), None) => Some((c, 4)),
        (None, Some(l)) => Some((l, 2)),
        (None, None) => None,
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The payload of a `data:` line, or `None` for any other SSE field
/// (`event:`, `id:`, `retry:`, comments starting with `:`).
///
/// Per the SSE spec a single space after the colon is part of the framing
/// and is stripped; anything beyond that is payload. Some gateways emit
/// `data:{...}` with no space at all, which is equally legal.
fn sse_data_payload(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("data:")?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// The text delta carried by one streaming chunk, if it carries any.
///
/// Returns `None` — never panics, never indexes — for the shapes that
/// legitimately carry no text: `choices: []`, a `delta` with only a `role`,
/// `content: null`, an empty `content`, or a body that isn't JSON at all.
fn parse_content_delta(data: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let content = value
        .get("choices")?
        .as_array()?
        .first()?
        .get("delta")?
        .get("content")?
        .as_str()?;
    if content.is_empty() {
        return None;
    }
    Some(content.to_string())
}

/// Whether the reader should keep going after an event.
enum Flow {
    Continue,
    Stop,
}

/// Emit every text delta in one SSE event block. Stops the stream on the
/// terminal `[DONE]` marker (anything a chatty gateway appends after it is
/// not the model's answer) or when the receiver has gone away.
async fn emit_event(event: &str, tx: &Sender<anyhow::Result<String>>) -> Flow {
    for line in event.lines() {
        let Some(payload) = sse_data_payload(line) else {
            continue;
        };
        if payload.trim() == DONE_MARKER {
            return Flow::Stop;
        }
        let Some(text) = parse_content_delta(payload) else {
            continue;
        };
        if tx.send(Ok(text)).await.is_err() {
            return Flow::Stop;
        }
    }
    Flow::Continue
}

// --- The protocol itself ---

impl Endpoint<'_> {
    fn url(&self) -> String {
        // A user who writes `base_url: http://localhost:4000/v1/` should
        // not get `//chat/completions`; some gateways route that to a 404.
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn body(&self, request: &CompletionRequest, stream: bool) -> ChatRequest {
        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        if !request.system_prompt.is_empty() {
            messages.push(ChatRequestMessage {
                role: "system".to_string(),
                content: request.system_prompt.clone(),
            });
        }
        messages.extend(request.messages.iter().map(|m| ChatRequestMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        }));

        ChatRequest {
            model: request.model.clone(),
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: stream.then_some(true),
        }
    }

    /// Send the request through the shared retry/backoff policy and turn a
    /// non-success status into a useful `anyhow` error.
    async fn send(&self, body: &ChatRequest) -> anyhow::Result<reqwest::Response> {
        let url = self.url();
        let response = send_with_retry(&RetryPolicy::default(), || {
            let builder = self.client.post(&url).json(body);
            match self.api_key {
                Some(key) => builder.bearer_auth(key),
                None => builder,
            }
        })
        .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            let message = error_message_from_body(&text);
            anyhow::bail!("{} API error ({status}): {message}", self.label);
        }

        Ok(response)
    }

    pub(crate) async fn complete(
        &self,
        request: CompletionRequest,
    ) -> anyhow::Result<CompletionResponse> {
        let body = self.body(&request, false);
        let response = self.send(&body).await?;
        let api_resp: ChatResponse = response.json().await?;

        let content = api_resp
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let model = api_resp.model.unwrap_or_else(|| request.model.clone());

        let (tokens_in, tokens_out) = api_resp
            .usage
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        let cost = cost_for_model(&model, tokens_in, tokens_out);

        Ok(CompletionResponse {
            content,
            model,
            tokens_in,
            tokens_out,
            cost,
        })
    }

    pub(crate) async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        let body = self.body(&request, true);
        let response = self.send(&body).await?;

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let byte_stream = response.bytes_stream();

        tokio::spawn(async move {
            // Bytes, not a `String`: an event boundary is ASCII, but the
            // payload is not, and a chunk can end in the middle of a
            // multi-byte character. Decoding per chunk (what the two
            // first-party providers do) would turn that into replacement
            // characters; decoding per completed event cannot.
            let mut buffer: Vec<u8> = Vec::new();
            tokio::pin!(byte_stream);

            while let Some(chunk) = byte_stream.next().await {
                match chunk {
                    Ok(bytes) => buffer.extend_from_slice(&bytes),
                    Err(e) => {
                        if let Err(send_err) =
                            tx.send(Err(anyhow::anyhow!("Stream error: {e}"))).await
                        {
                            tracing::debug!(
                                "Failed to send stream error (receiver dropped): {:?}",
                                send_err
                            );
                        }
                        return;
                    }
                }

                while let Some((end, sep)) = find_event_end(&buffer) {
                    let event = String::from_utf8_lossy(&buffer[..end]).into_owned();
                    buffer.drain(..end + sep);
                    if let Flow::Stop = emit_event(&event, &tx).await {
                        return;
                    }
                }
            }

            // A server may close right after the last event without the
            // trailing blank line. Whatever is left is a complete event.
            if !buffer.is_empty() {
                let event = String::from_utf8_lossy(&buffer).into_owned();
                let _ = emit_event(&event, &tx).await;
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- cost table ---

    #[test]
    fn known_openai_ids_are_priced_and_the_mini_prefix_wins() {
        // gpt-4o-mini also starts with gpt-4o: order matters.
        assert_eq!(rates_for_model("gpt-4o-mini"), Some((0.15, 0.60)));
        assert_eq!(rates_for_model("gpt-4o"), Some((2.50, 10.00)));
        assert_eq!(rates_for_model("gpt-4o-2024-08-06"), Some((2.50, 10.00)));
        assert_eq!(rates_for_model("o1"), Some((15.00, 60.00)));
        assert_eq!(rates_for_model("o1-mini"), Some((1.10, 4.40)));
        assert_eq!(rates_for_model("o3-mini"), Some((1.10, 4.40)));
        assert_eq!(rates_for_model("gpt-4.1-nano"), Some((0.10, 0.40)));
        assert_eq!(rates_for_model("gpt-4.1-mini"), Some((0.40, 1.60)));
        assert_eq!(rates_for_model("gpt-4.1"), Some((2.00, 8.00)));
    }

    #[test]
    fn a_vendor_prefixed_gateway_id_is_priced_like_the_bare_one() {
        assert_eq!(rates_for_model("openai/gpt-4o-mini"), Some((0.15, 0.60)));
        assert_eq!(rates_for_model("azure/gpt-4o"), Some((2.50, 10.00)));
    }

    /// A model this table doesn't know about must report nothing rather
    /// than a plausible-looking invented price.
    #[test]
    fn an_unknown_model_is_not_priced() {
        assert_eq!(rates_for_model("llama3.1:8b"), None);
        assert_eq!(rates_for_model("deepseek-chat"), None);
        assert_eq!(cost_for_model("llama3.1:8b", 100_000, 100_000), 0.0);
    }

    #[test]
    fn cost_is_per_million_tokens() {
        let cost = cost_for_model("gpt-4o", 1_000, 500);
        let expected = (1_000.0 * 2.50 + 500.0 * 10.00) / 1_000_000.0;
        assert!((cost - expected).abs() < f64::EPSILON, "{cost}");
    }

    // --- error bodies ---

    #[test]
    fn every_error_shape_this_dialect_produces_yields_its_message() {
        assert_eq!(
            error_message_from_body(r#"{"error":{"message":"Invalid API key","type":"auth"}}"#),
            "Invalid API key"
        );
        assert_eq!(
            error_message_from_body(r#"{"error":"model 'x' not found"}"#),
            "model 'x' not found"
        );
        assert_eq!(
            error_message_from_body(r#"{"object":"error","message":"bad request"}"#),
            "bad request"
        );
        assert_eq!(
            error_message_from_body(r#"{"detail":"Not Found"}"#),
            "Not Found"
        );
    }

    #[test]
    fn an_empty_error_body_says_so_rather_than_trailing_off() {
        assert_eq!(error_message_from_body(""), NO_BODY);
        assert_eq!(error_message_from_body("   \n "), NO_BODY);
    }

    #[test]
    fn a_non_json_error_body_is_echoed_and_bounded() {
        assert_eq!(
            error_message_from_body("<html>502 Bad Gateway</html>"),
            "<html>502 Bad Gateway</html>"
        );

        let long = "x".repeat(MAX_RAW_ERROR_CHARS + 50);
        let out = error_message_from_body(&long);
        assert!(out.ends_with("… (truncated)"), "{out}");
        assert_eq!(
            out.chars().count(),
            MAX_RAW_ERROR_CHARS + "… (truncated)".chars().count()
        );
    }

    /// Truncation counts characters, not bytes — slicing a multi-byte
    /// string by byte index panics.
    ///
    /// The fixture uses a **3**-byte character on purpose: with a 2-byte
    /// one, byte offset 500 happens to land on a character boundary and a
    /// byte-slicing implementation would pass this test unharmed (measured
    /// — that was the first version of this test, and it survived the
    /// mutation). 500 is not a multiple of 3, so `&text[..500]` here is
    /// squarely inside a character.
    #[test]
    fn truncation_does_not_split_a_multibyte_character() {
        assert_ne!(
            MAX_RAW_ERROR_CHARS % 3,
            0,
            "the fixture below only bites while the cap is not a multiple of the char width"
        );
        let text = "€".repeat(MAX_RAW_ERROR_CHARS + 10);
        let out = truncate_chars(&text, MAX_RAW_ERROR_CHARS);
        assert!(out.starts_with('€'));
        assert!(out.ends_with("… (truncated)"));
        assert_eq!(
            out.chars().count(),
            MAX_RAW_ERROR_CHARS + "… (truncated)".chars().count()
        );
    }

    // --- SSE framing ---

    #[test]
    fn an_event_boundary_is_found_for_both_lf_and_crlf_framing() {
        assert_eq!(find_event_end(b"data: a\n\ndata: b"), Some((7, 2)));
        assert_eq!(find_event_end(b"data: a\r\n\r\ndata: b"), Some((7, 4)));
    }

    #[test]
    fn a_partial_event_is_not_split_early() {
        assert_eq!(find_event_end(b"data: {\"choices\":[{\"delta\""), None);
        assert_eq!(find_event_end(b"data: a\r\n\r"), None);
        assert_eq!(find_event_end(b""), None);
    }

    #[test]
    fn the_earliest_boundary_wins_when_both_framings_appear() {
        // LF boundary at 1, CRLF boundary later: the LF one must be taken.
        assert_eq!(find_event_end(b"a\n\nb\r\n\r\nc"), Some((1, 2)));
        // And the reverse ordering.
        assert_eq!(find_event_end(b"a\r\n\r\nb\n\nc"), Some((1, 4)));
    }

    #[test]
    fn a_data_line_is_recognised_with_and_without_the_optional_space() {
        assert_eq!(sse_data_payload("data: {\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_data_payload("data:{\"a\":1}"), Some("{\"a\":1}"));
        // Only ONE space is framing; the rest is payload.
        assert_eq!(sse_data_payload("data:  x"), Some(" x"));
        assert_eq!(sse_data_payload("event: message"), None);
        assert_eq!(sse_data_payload(": keep-alive comment"), None);
        assert_eq!(sse_data_payload("id: 42"), None);
    }

    #[test]
    fn a_content_delta_is_extracted_and_every_empty_shape_is_skipped() {
        assert_eq!(
            parse_content_delta(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#),
            Some("Hi".to_string())
        );
        assert_eq!(parse_content_delta(r#"{"choices":[]}"#), None);
        assert_eq!(
            parse_content_delta(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#),
            None
        );
        assert_eq!(
            parse_content_delta(r#"{"choices":[{"delta":{"content":null}}]}"#),
            None
        );
        assert_eq!(
            parse_content_delta(r#"{"choices":[{"delta":{"content":""}}]}"#),
            None
        );
        assert_eq!(parse_content_delta("[DONE]"), None);
        assert_eq!(parse_content_delta("not json at all"), None);
        // The usage-only trailer some gateways append after the last delta.
        assert_eq!(
            parse_content_delta(r#"{"choices":[],"usage":{"prompt_tokens":9}}"#),
            None
        );
    }

    // --- request building ---

    fn base_request() -> CompletionRequest {
        CompletionRequest {
            model: "gpt-4o".to_string(),
            system_prompt: "sys".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            temperature: 0.4,
            max_tokens: None,
        }
    }

    fn endpoint<'a>(client: &'a Client, base_url: &'a str) -> Endpoint<'a> {
        Endpoint {
            client,
            base_url,
            api_key: None,
            label: "Test",
        }
    }

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_up() {
        let client = Client::new();
        assert_eq!(
            endpoint(&client, "http://localhost:4000/v1/").url(),
            "http://localhost:4000/v1/chat/completions"
        );
        assert_eq!(
            endpoint(&client, "http://localhost:4000/v1").url(),
            "http://localhost:4000/v1/chat/completions"
        );
    }

    #[test]
    fn the_system_prompt_becomes_a_leading_system_message() {
        let client = Client::new();
        let body = endpoint(&client, "http://x").body(&base_request(), false);
        let json = serde_json::to_string(&body).unwrap();
        assert!(
            json.starts_with(r#"{"model":"gpt-4o","messages":[{"role":"system","content":"sys"}"#),
            "{json}"
        );
        assert!(
            json.contains(r#"{"role":"user","content":"hello"}"#),
            "{json}"
        );
    }

    #[test]
    fn an_empty_system_prompt_adds_no_message() {
        let client = Client::new();
        let mut request = base_request();
        request.system_prompt = String::new();
        let body = endpoint(&client, "http://x").body(&request, false);
        assert_eq!(body.messages.len(), 1);
        assert_eq!(body.messages[0].role, "user");
    }

    #[test]
    fn optional_parameters_are_omitted_rather_than_defaulted() {
        let client = Client::new();
        let json =
            serde_json::to_string(&endpoint(&client, "http://x").body(&base_request(), false))
                .unwrap();
        assert!(!json.contains("max_tokens"), "{json}");
        assert!(!json.contains("stream"), "{json}");

        let mut request = base_request();
        request.max_tokens = Some(64);
        let json =
            serde_json::to_string(&endpoint(&client, "http://x").body(&request, true)).unwrap();
        assert!(json.contains(r#""max_tokens":64"#), "{json}");
        assert!(json.contains(r#""stream":true"#), "{json}");
    }

    // --- response parsing ---

    #[test]
    fn a_response_missing_every_optional_field_still_parses() {
        let resp: ChatResponse =
            serde_json::from_str(r#"{"choices":[{"message":{"content":"x"}}]}"#).unwrap();
        assert!(resp.model.is_none());
        assert!(resp.usage.is_none());
        assert_eq!(
            resp.choices[0].message.as_ref().unwrap().content.as_deref(),
            Some("x")
        );
    }

    /// A body with no `choices` key at all — `Vec` is a required field
    /// unless defaulted, so this is the one place the attribute matters.
    #[test]
    fn a_response_with_no_choices_key_still_parses() {
        let resp: ChatResponse =
            serde_json::from_str(r#"{"model":"m","usage":{"prompt_tokens":1}}"#).unwrap();
        assert!(resp.choices.is_empty());
        assert_eq!(resp.model.as_deref(), Some("m"));
    }

    #[test]
    fn a_null_usage_parses_as_absent_and_partial_usage_defaults_to_zero() {
        let resp: ChatResponse = serde_json::from_str(r#"{"choices":[],"usage":null}"#).unwrap();
        assert!(resp.usage.is_none());

        let resp: ChatResponse =
            serde_json::from_str(r#"{"choices":[],"usage":{"prompt_tokens":7}}"#).unwrap();
        let usage = resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 7);
        assert_eq!(usage.completion_tokens, 0);
    }

    #[test]
    fn a_tool_call_choice_with_null_content_parses_as_empty_text() {
        let resp: ChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"role":"assistant","content":null}}]}"#,
        )
        .unwrap();
        assert!(resp.choices[0].message.as_ref().unwrap().content.is_none());
    }
}
