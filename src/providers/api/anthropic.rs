use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::providers::traits::*;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicProvider {
    api_key: String,
    pub(crate) base_url: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.anthropic.com/v1".to_string(),
            client: Client::new(),
        }
    }
}

// --- API request/response types ---

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ApiMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
    model: String,
    usage: ApiUsage,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[derive(Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
struct ApiError {
    error: ApiErrorDetail,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    message: String,
}

// --- Cost calculation ---

fn cost_for_model(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let (input_rate, output_rate) = match model {
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("haiku") => (0.80, 4.0),
        _ => (3.0, 15.0), // sonnet pricing as default
    };
    (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0
}

// --- SSE parsing ---

fn parse_sse_text_delta(data: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let event_type = value.get("type")?.as_str()?;
    if event_type == "content_block_delta" {
        return value.get("delta")?.get("text")?.as_str().map(String::from);
    }
    None
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let body = ApiRequest {
            model: request.model.clone(),
            max_tokens: request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            system: if request.system_prompt.is_empty() {
                None
            } else {
                Some(request.system_prompt)
            },
            messages: request
                .messages
                .into_iter()
                .map(|m| ApiMessage {
                    role: m.role,
                    content: m.content,
                })
                .collect(),
            temperature: request.temperature,
            stream: None,
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<ApiError>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            anyhow::bail!("Anthropic API error ({status}): {msg}");
        }

        let api_resp: ApiResponse = response.json().await?;
        let content = api_resp
            .content
            .into_iter()
            .map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");

        let cost = cost_for_model(
            &api_resp.model,
            api_resp.usage.input_tokens,
            api_resp.usage.output_tokens,
        );

        Ok(CompletionResponse {
            content,
            model: api_resp.model,
            tokens_in: api_resp.usage.input_tokens,
            tokens_out: api_resp.usage.output_tokens,
            cost,
        })
    }

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        let body = ApiRequest {
            model: request.model.clone(),
            max_tokens: request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            system: if request.system_prompt.is_empty() {
                None
            } else {
                Some(request.system_prompt)
            },
            messages: request
                .messages
                .into_iter()
                .map(|m| ApiMessage {
                    role: m.role,
                    content: m.content,
                })
                .collect(),
            temperature: request.temperature,
            stream: Some(true),
        };

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<ApiError>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            anyhow::bail!("Anthropic API error ({status}): {msg}");
        }

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let byte_stream = response.bytes_stream();

        tokio::spawn(async move {
            let mut buffer = String::new();
            tokio::pin!(byte_stream);

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("Stream error: {e}"))).await;
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE events (delimited by \n\n)
                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    // Extract data line from SSE event
                    for line in event_block.lines() {
                        if let Some(data) = line.strip_prefix("data: ")
                            && let Some(text) = parse_sse_text_delta(data)
                            && tx.send(Ok(text)).await.is_err()
                        {
                            return;
                        }
                    }
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_content_block_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        assert_eq!(parse_sse_text_delta(data), Some("Hello".to_string()));
    }

    #[test]
    fn parse_non_delta_event() {
        let data = r#"{"type":"message_start","message":{"id":"msg_1"}}"#;
        assert_eq!(parse_sse_text_delta(data), None);
    }

    #[test]
    fn cost_calculation() {
        // Sonnet: $3/M in, $15/M out
        let cost = cost_for_model("claude-sonnet-4-5-20250929", 1000, 500);
        let expected = (1000.0 * 3.0 + 500.0 * 15.0) / 1_000_000.0;
        assert!((cost - expected).abs() < f64::EPSILON);

        // Opus: $15/M in, $75/M out
        let cost = cost_for_model("claude-opus-4-6", 100, 200);
        let expected = (100.0 * 15.0 + 200.0 * 75.0) / 1_000_000.0;
        assert!((cost - expected).abs() < f64::EPSILON);
    }
}
