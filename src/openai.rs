//! OpenAI API client.
//!
//! Works with any OpenAI-compatible endpoint (OpenAI, Azure via proxy, vLLM, etc.).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::types::{ChatResponse, Message, Usage};

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

    /// Send a chat completion request.
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let url = format!(
            "{}/v1/chat/completions",
            self.endpoint.trim_end_matches('/')
        );
        debug!(url = %url, model = %self.model, "Sending chat request");

        let request = ChatRequest {
            model: &self.model,
            messages,
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

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.model
    }
}
