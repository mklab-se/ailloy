//! OpenAI API client.
//!
//! Works with any OpenAI-compatible endpoint (OpenAI, Azure via proxy, vLLM, etc.).

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::client::Provider;
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbeddingResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, StreamEvent, Usage,
};

const DEFAULT_ENDPOINT: &str = "https://api.openai.com";

/// Client for the OpenAI chat completions API.
pub struct OpenAiClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatApiResponse {
    choices: Vec<Choice>,
    model: String,
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Deserialize)]
struct MessageContent {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ApiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

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
struct StreamChunk {
    choices: Vec<StreamChoice>,
    model: Option<String>,
    usage: Option<ApiUsage>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

// Image generation uses two different API paths:
// - Dedicated image models (dall-e-*, gpt-image-*) → Images API (/v1/images/generations)
// - Chat models (gpt-5, gpt-4o, etc.) → Responses API (/v1/responses) with image_generation tool
fn is_dedicated_image_model(model: &str) -> bool {
    model.starts_with("gpt-image") || model.starts_with("dall-e")
}

fn is_gpt_image_model(model: &str) -> bool {
    model.starts_with("gpt-image")
}

// Images API response types
#[derive(Deserialize)]
struct ImageGenResponse {
    data: Vec<ImageData>,
}

#[derive(Deserialize)]
struct ImageData {
    b64_json: Option<String>,
    revised_prompt: Option<String>,
}

// Responses API response types (for chat models with image_generation tool)
#[derive(Deserialize)]
struct ResponsesApiResponse {
    output: Vec<ResponsesOutput>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ResponsesOutput {
    #[serde(rename = "image_generation_call")]
    ImageGenerationCall {
        result: Option<String>,
        revised_prompt: Option<String>,
    },
    #[serde(other)]
    Other,
}

// Embedding types
#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbeddingApiResponse {
    data: Vec<EmbeddingData>,
    model: String,
    usage: Option<EmbeddingUsage>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbeddingUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

impl OpenAiClient {
    /// Create a new OpenAI client.
    ///
    /// If `endpoint` is `None`, defaults to `https://api.openai.com`.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        endpoint: Option<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            model: model.into(),
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
        }
    }

    /// Send a chat completion request (legacy method for backward compatibility).
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        <Self as Provider>::chat(self, messages, None).await
    }

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// List available models from the API.
    ///
    /// Works with OpenAI and any OpenAI-compatible endpoint (LM Studio, vLLM, etc.).
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/v1/models", self.base_url());

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .context("Failed to list models")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to list models ({}): {}", status.as_u16(), body);
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

    fn base_url(&self) -> String {
        self.endpoint.trim_end_matches('/').to_string()
    }

    /// Generate an image using the Images API (`/v1/images/generations`).
    /// For dedicated image models: dall-e-*, gpt-image-*.
    async fn generate_image_via_images_api(
        &self,
        prompt: &str,
        options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        let url = format!("{}/v1/images/generations", self.base_url());
        debug!(url = %url, model = %self.model, "Sending image generation request (Images API)");

        let size = options
            .and_then(|o| o.size)
            .map(|(w, h)| format!("{}x{}", w, h));

        let mut body = serde_json::json!({
            "model": &self.model,
            "prompt": prompt,
            "n": 1,
        });
        if let Some(size) = &size {
            body["size"] = serde_json::json!(size);
        }
        if let Some(quality) = options.and_then(|o| o.quality.as_deref()) {
            body["quality"] = serde_json::json!(quality);
        }

        if is_gpt_image_model(&self.model) {
            // gpt-image models: no response_format/style, use output_format instead
            body["output_format"] = serde_json::json!("png");
        } else {
            // DALL-E models: use response_format and style
            body["response_format"] = serde_json::json!("b64_json");
            if let Some(style) = options.and_then(|o| o.style.as_deref()) {
                body["style"] = serde_json::json!(style);
            }
        }

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("Failed to send image generation request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            anyhow::bail!(
                "OpenAI image generation error ({}): {}",
                status.as_u16(),
                message
            );
        }

        let api_response: ImageGenResponse = response
            .json()
            .await
            .context("Failed to parse image generation response")?;

        let image_data = api_response
            .data
            .first()
            .context("No image data in response")?;

        let b64 = image_data
            .b64_json
            .as_ref()
            .context("No base64 image data in response")?;

        let data = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode base64 image data")?;

        let (width, height) = crate::types::image_dimensions(&data)
            .or_else(|| options.and_then(|o| o.size))
            .unwrap_or((1024, 1024));

        Ok(ImageResponse {
            data,
            width,
            height,
            format: ImageFormat::Png,
            revised_prompt: image_data.revised_prompt.clone(),
        })
    }

    /// Generate an image using the Responses API (`/v1/responses`).
    /// For chat models (gpt-5, gpt-4o, etc.) via the image_generation tool.
    async fn generate_image_via_responses_api(
        &self,
        prompt: &str,
        options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        let url = format!("{}/v1/responses", self.base_url());
        debug!(url = %url, model = %self.model, "Sending image generation request (Responses API)");

        let mut tool = serde_json::json!({
            "type": "image_generation",
        });
        if let Some(size) = options.and_then(|o| o.size) {
            tool["size"] = serde_json::json!(format!("{}x{}", size.0, size.1));
        }
        if let Some(quality) = options.and_then(|o| o.quality.as_deref()) {
            tool["quality"] = serde_json::json!(quality);
        }

        let body = serde_json::json!({
            "model": &self.model,
            "input": prompt,
            "tools": [tool],
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("Failed to send image generation request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            anyhow::bail!(
                "OpenAI image generation error ({}): {}",
                status.as_u16(),
                message
            );
        }

        let api_response: ResponsesApiResponse = response
            .json()
            .await
            .context("Failed to parse Responses API response")?;

        // Find the first image_generation_call in the output
        let (b64, revised_prompt) = api_response
            .output
            .iter()
            .find_map(|o| match o {
                ResponsesOutput::ImageGenerationCall {
                    result,
                    revised_prompt,
                } => result.as_ref().map(|r| (r.clone(), revised_prompt.clone())),
                ResponsesOutput::Other => None,
            })
            .context("No image_generation_call in response output")?;

        let data = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .context("Failed to decode base64 image data")?;

        let (width, height) = crate::types::image_dimensions(&data)
            .or_else(|| options.and_then(|o| o.size))
            .unwrap_or((1024, 1024));

        Ok(ImageResponse {
            data,
            width,
            height,
            format: ImageFormat::Png,
            revised_prompt,
        })
    }
}

#[async_trait]
impl Provider for OpenAiClient {
    fn name(&self) -> &str {
        "openai"
    }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url());
        debug!(url = %url, model = %self.model, "Sending chat request");

        let request = ChatRequest {
            model: &self.model,
            messages,
            max_completion_tokens: options.and_then(|o| o.max_tokens),
            temperature: options.and_then(|o| o.temperature),
            stream: false,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to OpenAI API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            anyhow::bail!("OpenAI API error ({}): {}", status.as_u16(), message);
        }

        let api_response: ChatApiResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI API response")?;

        let content = api_response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(ChatResponse {
            content,
            model: api_response.model,
            usage: api_response.usage.map(|u| Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
        })
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        let url = format!("{}/v1/chat/completions", self.base_url());
        debug!(url = %url, model = %self.model, "Sending streaming chat request");

        let request = ChatRequest {
            model: &self.model,
            messages,
            max_completion_tokens: options.and_then(|o| o.max_tokens),
            temperature: options.and_then(|o| o.temperature),
            stream: true,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request to OpenAI API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            anyhow::bail!("OpenAI API error ({}): {}", status.as_u16(), message);
        }

        let model = self.model.clone();
        let byte_stream = response.bytes_stream();

        let stream = futures_util::stream::unfold(
            (byte_stream, String::new(), String::new(), model),
            |(mut byte_stream, mut buffer, mut assembled, model)| async move {
                loop {
                    // Process any complete lines in the buffer
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim().to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: model.clone(),
                                    usage: None,
                                };
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, model),
                                ));
                            }

                            if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                                if let Some(choice) = chunk.choices.first() {
                                    if let Some(text) = &choice.delta.content {
                                        if !text.is_empty() {
                                            assembled.push_str(text);
                                            return Some((
                                                Ok(StreamEvent::Delta(text.clone())),
                                                (byte_stream, buffer, assembled, model),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Read more data
                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((Err(e.into()), (byte_stream, buffer, assembled, model)));
                        }
                        None => {
                            // Stream ended without [DONE]
                            if !assembled.is_empty() {
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: model.clone(),
                                    usage: None,
                                };
                                assembled.clear();
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, model),
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

    async fn generate_image(
        &self,
        prompt: &str,
        options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        if is_dedicated_image_model(&self.model) {
            self.generate_image_via_images_api(prompt, options).await
        } else {
            self.generate_image_via_responses_api(prompt, options).await
        }
    }

    async fn embed(&self, input: &str) -> Result<EmbeddingResponse> {
        let url = format!("{}/v1/embeddings", self.base_url());
        debug!(url = %url, "Sending embedding request");

        let request = EmbeddingRequest {
            model: &self.model,
            input,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .context("Failed to send embedding request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            anyhow::bail!("OpenAI embedding error ({}): {}", status.as_u16(), message);
        }

        let api_response: EmbeddingApiResponse = response
            .json()
            .await
            .context("Failed to parse embedding response")?;

        let vector = api_response
            .data
            .first()
            .map(|d| d.embedding.clone())
            .unwrap_or_default();

        Ok(EmbeddingResponse {
            vector,
            model: api_response.model,
            usage: api_response.usage.map(|u| Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: 0,
                total_tokens: u.total_tokens,
            }),
        })
    }
}
