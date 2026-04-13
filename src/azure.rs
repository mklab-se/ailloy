//! Azure OpenAI Service client.

use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::client::Provider;
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageFormat, ImageOptions,
    ImageResponse, Message, StreamEvent, Usage,
};

/// Authentication method for Azure OpenAI.
#[derive(Debug, Clone)]
pub enum AzureAuth {
    /// API key passed as a header.
    ApiKey(String),
    /// Authenticate via `az` CLI (Azure Active Directory).
    AzureCli,
}

/// Client for the Azure OpenAI Service.
pub struct AzureOpenAiClient {
    client: reqwest::Client,
    endpoint: String,
    deployment: String,
    api_version: String,
    auth: AzureAuth,
}

// Request types
#[derive(Serialize)]
struct ChatRequest<'a> {
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
    #[serde(default)]
    code: Option<String>,
}

// Streaming types
#[derive(Deserialize)]
#[allow(dead_code)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

// Image generation types
#[derive(Serialize)]
struct ImageGenRequest<'a> {
    prompt: &'a str,
    n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<&'a str>,
    response_format: &'a str,
}

#[derive(Deserialize)]
struct ImageGenResponse {
    data: Vec<ImageData>,
}

#[derive(Deserialize)]
struct ImageData {
    b64_json: Option<String>,
    revised_prompt: Option<String>,
}

// Embedding types
#[derive(Serialize)]
struct EmbedRequest<'a> {
    input: &'a [&'a str],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

#[derive(Deserialize)]
struct EmbedApiResponse {
    data: Vec<EmbedData>,
    model: String,
    usage: EmbedApiUsage,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedApiUsage {
    prompt_tokens: u32,
    total_tokens: u32,
}

impl AzureOpenAiClient {
    /// Create a new Azure OpenAI client.
    pub fn new(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        api_version: impl Into<String>,
        auth: AzureAuth,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            deployment: deployment.into(),
            api_version: api_version.into(),
            auth,
        }
    }

    fn base_url(&self) -> String {
        self.endpoint.trim_end_matches('/').to_string()
    }

    fn chat_url(&self) -> String {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.base_url(),
            self.deployment,
            self.api_version
        )
    }

    fn image_url(&self) -> String {
        format!(
            "{}/openai/deployments/{}/images/generations?api-version={}",
            self.base_url(),
            self.deployment,
            self.api_version
        )
    }

    fn embed_url(&self) -> String {
        format!(
            "{}/openai/deployments/{}/embeddings?api-version={}",
            self.base_url(),
            self.deployment,
            self.api_version
        )
    }

    fn format_api_error(&self, status: u16, body: &str) -> String {
        if let Ok(err) = serde_json::from_str::<ApiError>(body) {
            let code = err.error.code.as_deref().unwrap_or("");
            let msg = &err.error.message;
            match (status, code) {
                (404, _) => {
                    format!(
                        "Azure OpenAI: deployment '{}' not found at {} (HTTP 404: {}). \
                         Check that the deployment exists and the endpoint is correct. \
                         Run 'ailloy config' to reconfigure.",
                        self.deployment, self.endpoint, msg
                    )
                }
                (401, _) | (403, _) => {
                    format!(
                        "Azure OpenAI: authentication failed (HTTP {}: {}). \
                         Run 'az login' to refresh credentials, or check your API key.",
                        status, msg
                    )
                }
                _ => {
                    format!("Azure OpenAI API error (HTTP {}): {}", status, msg)
                }
            }
        } else {
            format!("Azure OpenAI API error (HTTP {}): {}", status, body)
        }
    }

    async fn get_auth_header(&self) -> Result<(&'static str, String)> {
        match &self.auth {
            AzureAuth::ApiKey(key) => Ok(("api-key", key.clone())),
            AzureAuth::AzureCli => {
                let output = tokio::process::Command::new("az")
                    .args([
                        "account",
                        "get-access-token",
                        "--resource",
                        "https://cognitiveservices.azure.com",
                        "--query",
                        "accessToken",
                        "-o",
                        "tsv",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .context("Failed to run 'az' CLI. Is Azure CLI installed and authenticated?")?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    anyhow::bail!(
                        "Azure CLI authentication failed: {}. Run 'az login' to authenticate.",
                        stderr.trim()
                    );
                }

                let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(("Authorization", format!("Bearer {}", token)))
            }
        }
    }
}

#[async_trait]
impl Provider for AzureOpenAiClient {
    fn name(&self) -> &str {
        "azure-openai"
    }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = self.chat_url();
        debug!(url = %url, deployment = %self.deployment, "Sending chat request to Azure OpenAI");

        let (header_name, header_value) = self.get_auth_header().await?;

        let request = ChatRequest {
            messages,
            max_completion_tokens: options.and_then(|o| o.max_tokens),
            temperature: options.and_then(|o| o.temperature),
            stream: false,
        };

        let response = self
            .client
            .post(&url)
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Azure OpenAI")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let api_response: ChatApiResponse = response
            .json()
            .await
            .context("Failed to parse Azure OpenAI API response")?;

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
        let url = self.chat_url();
        debug!(url = %url, deployment = %self.deployment, "Sending streaming chat request to Azure OpenAI");

        let (header_name, header_value) = self.get_auth_header().await?;

        let request = ChatRequest {
            messages,
            max_completion_tokens: options.and_then(|o| o.max_tokens),
            temperature: options.and_then(|o| o.temperature),
            stream: true,
        };

        let response = self
            .client
            .post(&url)
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request to Azure OpenAI")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let deployment = self.deployment.clone();
        let byte_stream = response.bytes_stream();

        let stream = futures_util::stream::unfold(
            (byte_stream, String::new(), String::new(), deployment),
            |(mut byte_stream, mut buffer, mut assembled, deployment)| async move {
                loop {
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
                                    model: deployment.clone(),
                                    usage: None,
                                };
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, deployment),
                                ));
                            }

                            if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                                if let Some(choice) = chunk.choices.first() {
                                    if let Some(text) = &choice.delta.content {
                                        if !text.is_empty() {
                                            assembled.push_str(text);
                                            return Some((
                                                Ok(StreamEvent::Delta(text.clone())),
                                                (byte_stream, buffer, assembled, deployment),
                                            ));
                                        }
                                    }
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
                                (byte_stream, buffer, assembled, deployment),
                            ));
                        }
                        None => {
                            if !assembled.is_empty() {
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: deployment.clone(),
                                    usage: None,
                                };
                                assembled.clear();
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, deployment),
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
        let url = self.image_url();
        debug!(url = %url, "Sending image generation request to Azure OpenAI");

        let (header_name, header_value) = self.get_auth_header().await?;

        let size = options
            .and_then(|o| o.size)
            .map(|(w, h)| format!("{}x{}", w, h));

        let request = ImageGenRequest {
            prompt,
            n: 1,
            size,
            quality: options.and_then(|o| o.quality.as_deref()),
            response_format: "b64_json",
        };

        let response = self
            .client
            .post(&url)
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send image generation request to Azure OpenAI")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let api_response: ImageGenResponse = response
            .json()
            .await
            .context("Failed to parse Azure image generation response")?;

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

    async fn embed(&self, texts: &[&str], options: Option<&EmbedOptions>) -> Result<EmbedResponse> {
        let url = self.embed_url();
        debug!(url = %url, deployment = %self.deployment, count = texts.len(), "Sending embedding request to Azure OpenAI");

        let (header_name, header_value) = self.get_auth_header().await?;

        let request = EmbedRequest {
            input: texts,
            dimensions: options.and_then(|o| o.dimensions),
        };

        let response = self
            .client
            .post(&url)
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send embedding request to Azure OpenAI")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let api_response: EmbedApiResponse = response
            .json()
            .await
            .context("Failed to parse Azure OpenAI embedding response")?;

        Ok(EmbedResponse {
            embeddings: api_response.data.into_iter().map(|d| d.embedding).collect(),
            model: api_response.model,
            usage: Some(Usage {
                prompt_tokens: api_response.usage.prompt_tokens,
                completion_tokens: 0,
                total_tokens: api_response.usage.total_tokens,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{"data":[{"embedding":[0.1,0.2,0.3],"index":0}],"model":"text-embedding-3-large","usage":{"prompt_tokens":5,"total_tokens":5}}"#;
        let response: EmbedApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].embedding, vec![0.1, 0.2, 0.3]);
    }
}
