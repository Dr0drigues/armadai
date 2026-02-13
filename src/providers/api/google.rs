use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::providers::traits::*;

const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct GoogleProvider {
    pub api_key: String,
    pub(crate) base_url: String,
    client: Client,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
            client: Client::new(),
        }
    }
}

// --- API request/response types ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    temperature: f32,
    max_output_tokens: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsage>,
    #[serde(default)]
    model_version: Option<String>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsage {
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
}

#[derive(Deserialize)]
struct GeminiError {
    error: GeminiErrorDetail,
}

#[derive(Deserialize)]
struct GeminiErrorDetail {
    message: String,
}

// --- Cost calculation ---

fn cost_for_model(model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    let (input_rate, output_rate) = match model {
        m if m.contains("2.5-pro") => (1.25, 10.0),
        m if m.contains("2.5-flash") => (0.15, 0.60),
        m if m.contains("2.0-flash") => (0.10, 0.40),
        _ => (0.15, 0.60), // flash pricing as default
    };
    (input_tokens as f64 * input_rate + output_tokens as f64 * output_rate) / 1_000_000.0
}

// --- SSE parsing ---

fn parse_gemini_sse_chunk(data: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let text = value
        .get("candidates")?
        .get(0)?
        .get("content")?
        .get("parts")?
        .get(0)?
        .get("text")?
        .as_str()?;
    Some(text.to_string())
}

#[async_trait]
impl Provider for GoogleProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let body = build_request(&request);

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, request.model, self.api_key
        );

        let response = self.client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<GeminiError>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            anyhow::bail!("Google API error ({status}): {msg}");
        }

        let api_resp: GeminiResponse = response.json().await?;

        let content = api_resp
            .candidates
            .first()
            .map(|c| {
                c.content
                    .parts
                    .iter()
                    .map(|p| p.text.as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();

        let model = api_resp
            .model_version
            .unwrap_or_else(|| request.model.clone());

        let (tokens_in, tokens_out) = api_resp
            .usage_metadata
            .map(|u| (u.prompt_token_count, u.candidates_token_count))
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

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        let body = build_request(&request);

        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, request.model, self.api_key
        );

        let response = self.client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            let msg = serde_json::from_str::<GeminiError>(&text)
                .map(|e| e.error.message)
                .unwrap_or(text);
            anyhow::bail!("Google API error ({status}): {msg}");
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

                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    for line in event_block.lines() {
                        if let Some(data) = line.strip_prefix("data: ")
                            && let Some(text) = parse_gemini_sse_chunk(data)
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
            name: "google".to_string(),
            models: vec![
                "gemini-2.5-pro".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.0-flash".to_string(),
            ],
            supports_streaming: true,
        }
    }
}

fn build_request(request: &CompletionRequest) -> GeminiRequest {
    let contents: Vec<GeminiContent> = request
        .messages
        .iter()
        .map(|m| GeminiContent {
            role: Some(match m.role.as_str() {
                "assistant" => "model".to_string(),
                other => other.to_string(),
            }),
            parts: vec![GeminiPart {
                text: m.content.clone(),
            }],
        })
        .collect();

    let system_instruction = if request.system_prompt.is_empty() {
        None
    } else {
        Some(GeminiContent {
            role: None,
            parts: vec![GeminiPart {
                text: request.system_prompt.clone(),
            }],
        })
    };

    GeminiRequest {
        contents,
        system_instruction,
        generation_config: Some(GenerationConfig {
            temperature: request.temperature,
            max_output_tokens: request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_response() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello, world!"}],
                    "role": "model"
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5
            },
            "modelVersion": "gemini-2.5-pro"
        }"#;

        let resp: GeminiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.candidates[0].content.parts[0].text, "Hello, world!");
        assert_eq!(resp.model_version.as_deref(), Some("gemini-2.5-pro"));
        let usage = resp.usage_metadata.unwrap();
        assert_eq!(usage.prompt_token_count, 10);
        assert_eq!(usage.candidates_token_count, 5);
    }

    #[test]
    fn test_parse_gemini_sse_chunk() {
        let data = r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"}}]}"#;
        assert_eq!(parse_gemini_sse_chunk(data), Some("Hello".to_string()));
    }

    #[test]
    fn test_parse_gemini_sse_chunk_no_candidates() {
        let data = r#"{"usageMetadata":{"promptTokenCount":5}}"#;
        assert_eq!(parse_gemini_sse_chunk(data), None);
    }

    #[test]
    fn test_cost_calculation() {
        // 2.5 Pro: $1.25/M in, $10/M out
        let cost = cost_for_model("gemini-2.5-pro", 1000, 500);
        let expected = (1000.0 * 1.25 + 500.0 * 10.0) / 1_000_000.0;
        assert!((cost - expected).abs() < f64::EPSILON);

        // 2.5 Flash: $0.15/M in, $0.60/M out
        let cost = cost_for_model("gemini-2.5-flash", 1000, 500);
        let expected = (1000.0 * 0.15 + 500.0 * 0.60) / 1_000_000.0;
        assert!((cost - expected).abs() < f64::EPSILON);

        // 2.0 Flash: $0.10/M in, $0.40/M out
        let cost = cost_for_model("gemini-2.0-flash", 2000, 1000);
        let expected = (2000.0 * 0.10 + 1000.0 * 0.40) / 1_000_000.0;
        assert!((cost - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gemini_error_parsing() {
        let json =
            r#"{"error":{"message":"API key not valid","status":"INVALID_ARGUMENT","code":400}}"#;
        let err: GeminiError = serde_json::from_str(json).unwrap();
        assert_eq!(err.error.message, "API key not valid");
    }

    #[test]
    fn test_build_request_with_system_prompt() {
        let request = CompletionRequest {
            model: "gemini-2.5-pro".to_string(),
            system_prompt: "You are helpful.".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            temperature: 0.7,
            max_tokens: Some(1024),
        };

        let gemini_req = build_request(&request);
        assert!(gemini_req.system_instruction.is_some());
        assert_eq!(
            gemini_req.system_instruction.unwrap().parts[0].text,
            "You are helpful."
        );
        assert_eq!(gemini_req.contents.len(), 1);
        assert_eq!(gemini_req.contents[0].role.as_deref(), Some("user"));
    }

    #[test]
    fn test_build_request_maps_assistant_to_model() {
        let request = CompletionRequest {
            model: "gemini-2.5-pro".to_string(),
            system_prompt: String::new(),
            messages: vec![
                ChatMessage {
                    role: "user".to_string(),
                    content: "Hi".to_string(),
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: "Hello!".to_string(),
                },
            ],
            temperature: 0.5,
            max_tokens: None,
        };

        let gemini_req = build_request(&request);
        assert!(gemini_req.system_instruction.is_none());
        assert_eq!(gemini_req.contents[1].role.as_deref(), Some("model"));
    }
}
