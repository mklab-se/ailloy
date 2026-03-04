//! Microsoft Foundry client.
//!
//! Supports chat completions, streaming, and embeddings via the Model
//! Inference API (`*.services.ai.azure.com`).

use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::azure::AzureAuth;
use crate::client::Provider;
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbeddingResponse, Message, StreamEvent, Usage,
};

/// Client for Microsoft Foundry (AI Services).
pub struct FoundryClient {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_version: String,
    auth: AzureAuth,
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

impl FoundryClient {
    /// Create a new Microsoft Foundry client.
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_version: impl Into<String>,
        auth: AzureAuth,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            model: model.into(),
            api_version: api_version.into(),
            auth,
        }
    }

    fn base_url(&self) -> String {
        let url = self.endpoint.trim_end_matches('/').to_string();
        // Model Inference API lives on *.services.ai.azure.com, not *.cognitiveservices.azure.com
        url.replace(".cognitiveservices.azure.com", ".services.ai.azure.com")
    }

    fn chat_url(&self) -> String {
        format!(
            "{}/models/chat/completions?api-version={}",
            self.base_url(),
            self.api_version
        )
    }

    fn embedding_url(&self) -> String {
        format!(
            "{}/models/embeddings?api-version={}",
            self.base_url(),
            self.api_version
        )
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

    fn format_api_error(&self, status: u16, body: &str) -> String {
        if let Ok(err) = serde_json::from_str::<ApiError>(body) {
            let code = err.error.code.as_deref().unwrap_or("");
            let msg = &err.error.message;
            match (status, code) {
                (404, _) => {
                    format!(
                        "Microsoft Foundry: model '{}' not found at {} (HTTP 404: {}). \
                         Check that the model is deployed and the endpoint is correct. \
                         Run 'ailloy config' to reconfigure.",
                        self.model, self.endpoint, msg
                    )
                }
                (401, _) | (403, _) => {
                    format!(
                        "Microsoft Foundry: authentication failed (HTTP {}: {}). \
                         Run 'az login' to refresh credentials, or check your API key.",
                        status, msg
                    )
                }
                _ => {
                    format!("Microsoft Foundry API error (HTTP {}): {}", status, msg)
                }
            }
        } else {
            format!("Microsoft Foundry API error (HTTP {}): {}", status, body)
        }
    }
}

#[async_trait]
impl Provider for FoundryClient {
    fn name(&self) -> &str {
        "microsoft-foundry"
    }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = self.chat_url();
        debug!(url = %url, model = %self.model, "Sending chat request to Microsoft Foundry");

        let (header_name, header_value) = self.get_auth_header().await?;

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
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Microsoft Foundry")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let api_response: ChatApiResponse = response
            .json()
            .await
            .context("Failed to parse Microsoft Foundry API response")?;

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
        debug!(url = %url, model = %self.model, "Sending streaming chat request to Microsoft Foundry");

        let (header_name, header_value) = self.get_auth_header().await?;

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
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request to Microsoft Foundry")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let model = self.model.clone();
        let byte_stream = response.bytes_stream();

        let stream = futures_util::stream::unfold(
            (byte_stream, String::new(), String::new(), model),
            |(mut byte_stream, mut buffer, mut assembled, model)| async move {
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

                    match byte_stream.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((Err(e.into()), (byte_stream, buffer, assembled, model)));
                        }
                        None => {
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

    async fn embed(&self, input: &str) -> Result<EmbeddingResponse> {
        let url = self.embedding_url();
        debug!(url = %url, model = %self.model, "Sending embedding request to Microsoft Foundry");

        let (header_name, header_value) = self.get_auth_header().await?;

        let request = EmbeddingRequest {
            model: &self.model,
            input,
        };

        let response = self
            .client
            .post(&url)
            .header(header_name, &header_value)
            .json(&request)
            .send()
            .await
            .context("Failed to send embedding request to Microsoft Foundry")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let api_response: EmbeddingApiResponse = response
            .json()
            .await
            .context("Failed to parse Microsoft Foundry embedding response")?;

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
