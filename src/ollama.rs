//! Ollama local LLM client.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::client::Provider;
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, Message, StreamEvent,
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
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

// Embedding types
#[derive(Serialize)]
struct EmbedApiRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedApiResponse {
    model: String,
    embeddings: Vec<Vec<f32>>,
}

// Streaming response chunk
#[derive(Deserialize)]
struct StreamChunk {
    message: Option<ResponseMessage>,
    model: Option<String>,
    done: bool,
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

    /// Send a chat completion request (legacy method for backward compatibility).
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        <Self as Provider>::chat(self, messages, None).await
    }

    /// List available models.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url());
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

    fn base_url(&self) -> String {
        self.endpoint.trim_end_matches('/').to_string()
    }

    fn build_options(options: Option<&ChatOptions>) -> Option<OllamaOptions> {
        options.map(|o| OllamaOptions {
            num_predict: o.max_tokens,
            temperature: o.temperature,
        })
    }
}

#[async_trait]
impl Provider for OllamaClient {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = format!("{}/api/chat", self.base_url());
        debug!(url = %url, model = %self.model, "Sending chat request to Ollama");

        let request = ChatRequest {
            model: &self.model,
            messages,
            stream: false,
            options: Self::build_options(options),
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

    async fn chat_stream(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        let url = format!("{}/api/chat", self.base_url());
        debug!(url = %url, model = %self.model, "Sending streaming chat request to Ollama");

        let request = ChatRequest {
            model: &self.model,
            messages,
            stream: true,
            options: Self::build_options(options),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request to Ollama. Is Ollama running?")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error ({}): {}", status.as_u16(), body);
        }

        let model = self.model.clone();
        let byte_stream = response.bytes_stream();

        // Ollama streams NDJSON (one JSON object per line)
        let stream = futures_util::stream::unfold(
            (byte_stream, String::new(), String::new(), model),
            |(mut byte_stream, mut buffer, mut assembled, model)| async move {
                loop {
                    // Process complete lines
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim().to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        if let Ok(chunk) = serde_json::from_str::<StreamChunk>(&line) {
                            if chunk.done {
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: chunk.model.unwrap_or_else(|| model.clone()),
                                    usage: None,
                                };
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, model),
                                ));
                            }

                            if let Some(msg) = &chunk.message {
                                if !msg.content.is_empty() {
                                    assembled.push_str(&msg.content);
                                    return Some((
                                        Ok(StreamEvent::Delta(msg.content.clone())),
                                        (byte_stream, buffer, assembled, model),
                                    ));
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

    async fn embed(
        &self,
        texts: &[&str],
        _options: Option<&EmbedOptions>,
    ) -> Result<EmbedResponse> {
        let url = format!("{}/api/embed", self.base_url());
        debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request to Ollama");
        let request = EmbedApiRequest {
            model: &self.model,
            input: texts,
        };
        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send embedding request to Ollama. Is Ollama running?")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama embedding error ({}): {}", status.as_u16(), body);
        }
        let api_response: EmbedApiResponse = response
            .json()
            .await
            .context("Failed to parse Ollama embedding response")?;
        Ok(EmbedResponse {
            embeddings: api_response.embeddings,
            model: api_response.model,
            usage: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{"model":"nomic-embed-text","embeddings":[[0.1,0.2,0.3],[0.4,0.5,0.6]]}"#;
        let response: EmbedApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.model, "nomic-embed-text");
        assert_eq!(response.embeddings.len(), 2);
    }
}
