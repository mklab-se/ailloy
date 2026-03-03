//! Ollama local LLM client.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::types::{ChatResponse, Message};

const DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// Client for the Ollama API.
pub struct OllamaClient {
    client: reqwest::Client,
    model: String,
    endpoint: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
}

#[derive(Deserialize)]
struct ChatApiResponse {
    message: ResponseMessage,
    model: String,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

impl OllamaClient {
    /// Create a new Ollama client.
    ///
    /// If `endpoint` is `None`, defaults to `http://localhost:11434`.
    pub fn new(model: impl Into<String>, endpoint: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            model: model.into(),
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
        }
    }

    /// Send a chat completion request.
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let url = format!("{}/api/chat", self.endpoint.trim_end_matches('/'));
        debug!(url = %url, model = %self.model, "Sending chat request to Ollama");

        let request = ChatRequest {
            model: &self.model,
            messages,
            stream: false,
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Ollama. Is Ollama running?")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error ({}): {}", status.as_u16(), body);
        }

        let api_response: ChatApiResponse = response
            .json()
            .await
            .context("Failed to parse Ollama API response")?;

        Ok(ChatResponse {
            content: api_response.message.content,
            model: api_response.model,
            usage: None,
        })
    }

    /// List available models.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.endpoint.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to list Ollama models. Is Ollama running?")?;

        #[derive(Deserialize)]
        struct TagsResponse {
            models: Vec<ModelInfo>,
        }

        #[derive(Deserialize)]
        struct ModelInfo {
            name: String,
        }

        let tags: TagsResponse = response.json().await?;
        Ok(tags.models.into_iter().map(|m| m.name).collect())
    }

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.model
    }
}
