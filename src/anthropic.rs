//! Anthropic API client (Claude models).

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::client::Provider;
use crate::types::{ChatOptions, ChatResponse, ChatStream, Message, Role, StreamEvent, Usage};

const API_ENDPOINT: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
/// Default output cap when the caller sets none. Anthropic requires
/// max_tokens; 8192 avoids silently truncating long answers (the old 4096
/// default bit users) while staying well inside every current model's limit.
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Client for the Anthropic Messages API.
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

// Request types
#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    messages: Vec<AnthropicMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Serialize, Clone)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

// Response types
#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    model: String,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

// Error types
#[derive(Deserialize)]
struct ApiError {
    error: ApiErrorDetail,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    message: String,
}

// Streaming types
#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamEvent_ {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<StreamDelta>,
    message: Option<StreamMessage>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct StreamDelta {
    text: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamMessage {
    model: Option<String>,
    usage: Option<AnthropicUsage>,
}

impl AnthropicClient {
    /// Create a new Anthropic client.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// List available models from the Anthropic API.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/v1/models?limit=100", API_ENDPOINT);

        let response = self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .send()
            .await
            .context("Failed to list Anthropic models")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to list Anthropic models ({}): {}",
                status.as_u16(),
                body
            );
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelInfo>,
        }

        #[derive(Deserialize)]
        struct ModelInfo {
            id: String,
        }

        let models: ModelsResponse = response.json().await?;
        let mut ids: Vec<String> = models.data.into_iter().map(|m| m.id).collect();
        ids.sort();
        Ok(ids)
    }

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    fn convert_messages<'a>(
        messages: &'a [Message],
    ) -> (Option<&'a str>, Vec<AnthropicMessage<'a>>) {
        let mut system_prompt = None;
        let mut converted = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    system_prompt = Some(msg.content.as_str());
                }
                Role::User => {
                    converted.push(AnthropicMessage {
                        role: "user",
                        content: &msg.content,
                    });
                }
                Role::Assistant => {
                    converted.push(AnthropicMessage {
                        role: "assistant",
                        content: &msg.content,
                    });
                }
            }
        }

        (system_prompt, converted)
    }
}

#[async_trait]
impl Provider for AnthropicClient {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = format!("{}/v1/messages", API_ENDPOINT);
        debug!(url = %url, model = %self.model, "Sending chat request to Anthropic");

        let (system, converted) = Self::convert_messages(messages);
        let max_tokens = options
            .and_then(|o| o.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS);

        let mut temperature = options.and_then(|o| o.temperature);
        let response = loop {
            let request = MessagesRequest {
                model: &self.model,
                messages: converted.clone(),
                system,
                max_tokens,
                temperature,
                stream: false,
            };

            let response = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await
                .context("Failed to send request to Anthropic API")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                if temperature.is_some()
                    && crate::types::is_sampling_rejection(status.as_u16(), &body)
                {
                    debug!("model rejected sampling parameters; retrying without temperature");
                    temperature = None;
                    continue;
                }
                let message = serde_json::from_str::<ApiError>(&body)
                    .map(|e| e.error.message)
                    .unwrap_or(body);
                anyhow::bail!("Anthropic API error ({}): {}", status.as_u16(), message);
            }
            break response;
        };

        let api_response: MessagesResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic API response")?;

        let content = api_response
            .content
            .into_iter()
            .filter_map(|b| b.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(ChatResponse {
            content,
            model: api_response.model,
            usage: Some(Usage {
                prompt_tokens: api_response.usage.input_tokens,
                completion_tokens: api_response.usage.output_tokens,
                total_tokens: api_response.usage.input_tokens + api_response.usage.output_tokens,
            }),
        })
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        let url = format!("{}/v1/messages", API_ENDPOINT);
        debug!(url = %url, model = %self.model, "Sending streaming chat request to Anthropic");

        let (system, converted) = Self::convert_messages(messages);
        let max_tokens = options
            .and_then(|o| o.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS);

        let request = MessagesRequest {
            model: &self.model,
            messages: converted,
            system,
            max_tokens,
            temperature: options.and_then(|o| o.temperature),
            stream: true,
        };

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request to Anthropic API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            anyhow::bail!("Anthropic API error ({}): {}", status.as_u16(), message);
        }

        let model = self.model.clone();
        let byte_stream = response.bytes_stream();

        let stream = futures_util::stream::unfold(
            (
                byte_stream,
                String::new(),
                String::new(),
                model,
                (0u32, 0u32),
            ),
            |(mut byte_stream, mut buffer, mut assembled, model, mut tokens)| async move {
                loop {
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim().to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        // Anthropic SSE format: "event: ..." followed by "data: ..."
                        if let Some(data) = line.strip_prefix("data: ") {
                            if let Ok(event) = serde_json::from_str::<StreamEvent_>(data) {
                                match event.event_type.as_str() {
                                    "message_start" => {
                                        // input tokens arrive on the initial message
                                        if let Some(u) =
                                            event.message.as_ref().and_then(|m| m.usage.as_ref())
                                        {
                                            tokens.0 = u.input_tokens;
                                        }
                                    }
                                    "message_delta" => {
                                        // cumulative output tokens arrive on deltas
                                        if let Some(u) = event.usage.as_ref() {
                                            tokens.1 = u.output_tokens;
                                        }
                                    }
                                    "content_block_delta" => {
                                        if let Some(delta) = &event.delta {
                                            if let Some(text) = &delta.text {
                                                if !text.is_empty() {
                                                    assembled.push_str(text);
                                                    return Some((
                                                        Ok(StreamEvent::Delta(text.clone())),
                                                        (
                                                            byte_stream,
                                                            buffer,
                                                            assembled,
                                                            model,
                                                            tokens,
                                                        ),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    "message_stop" => {
                                        let usage = (tokens.0 + tokens.1 > 0).then_some(Usage {
                                            prompt_tokens: tokens.0,
                                            completion_tokens: tokens.1,
                                            total_tokens: tokens.0 + tokens.1,
                                        });
                                        let response = ChatResponse {
                                            content: assembled.clone(),
                                            model: model.clone(),
                                            usage,
                                        };
                                        return Some((
                                            Ok(StreamEvent::Done(response)),
                                            (byte_stream, buffer, assembled, model, tokens),
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(e.into()),
                                (byte_stream, buffer, assembled, model, tokens),
                            ));
                        }
                        None => {
                            if !assembled.is_empty() {
                                let usage = (tokens.0 + tokens.1 > 0).then_some(Usage {
                                    prompt_tokens: tokens.0,
                                    completion_tokens: tokens.1,
                                    total_tokens: tokens.0 + tokens.1,
                                });
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: model.clone(),
                                    usage,
                                };
                                assembled.clear();
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, model, tokens),
                                ));
                            }
                            return None;
                        }
                    }
                }
            },
        );

        Ok(Box::pin(stream))
    }
}
