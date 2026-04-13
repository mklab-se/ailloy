//! Google Vertex AI client (Gemini, Imagen).

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
    ImageResponse, Message, Role, StreamEvent, Usage,
};

/// Client for Google Vertex AI (Gemini models, Imagen).
pub struct VertexAiClient {
    client: reqwest::Client,
    project: String,
    location: String,
    model: String,
}

// Gemini request types
#[derive(Serialize)]
struct GenerateContentRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Serialize, Deserialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize)]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "inlineData")]
    inline_data: Option<InlineData>,
}

#[derive(Serialize, Deserialize)]
struct InlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

// Response types
#[derive(Deserialize)]
struct GenerateContentResponse {
    candidates: Option<Vec<Candidate>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsageMetadata>,
    #[serde(rename = "modelVersion")]
    model_version: Option<String>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<GeminiContent>,
}

#[derive(Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
    #[serde(rename = "totalTokenCount")]
    total_token_count: Option<u32>,
}

// Imagen request/response types
#[derive(Serialize)]
struct ImagenRequest {
    instances: Vec<ImagenInstance>,
    parameters: ImagenParameters,
}

#[derive(Serialize)]
struct ImagenInstance {
    prompt: String,
}

#[derive(Serialize)]
struct ImagenParameters {
    #[serde(rename = "sampleCount")]
    sample_count: u32,
}

#[derive(Deserialize)]
struct ImagenResponse {
    predictions: Option<Vec<ImagenPrediction>>,
}

#[derive(Deserialize)]
struct ImagenPrediction {
    #[serde(rename = "bytesBase64Encoded")]
    bytes_base64_encoded: String,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

// Embedding types
#[derive(Serialize)]
struct EmbedPredictRequest {
    instances: Vec<EmbedInstance>,
}

#[derive(Serialize)]
struct EmbedInstance {
    content: String,
}

#[derive(Deserialize)]
struct EmbedPredictResponse {
    predictions: Vec<EmbedPrediction>,
}

#[derive(Deserialize)]
struct EmbedPrediction {
    embeddings: EmbedValues,
}

#[derive(Deserialize)]
struct EmbedValues {
    values: Vec<f32>,
}

// Error types
#[derive(Deserialize)]
struct ApiError {
    error: Option<ApiErrorDetail>,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    message: String,
}

impl VertexAiClient {
    /// Create a new Vertex AI client.
    pub fn new(
        project: impl Into<String>,
        location: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            project: project.into(),
            location: location.into(),
            model: model.into(),
        }
    }

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    fn base_url(&self) -> String {
        format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}",
            self.location, self.project, self.location, self.model
        )
    }

    async fn get_access_token() -> Result<String> {
        let output = tokio::process::Command::new("gcloud")
            .args(["auth", "print-access-token"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context(
                "Failed to run 'gcloud' CLI. Is Google Cloud SDK installed and authenticated?",
            )?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "gcloud authentication failed: {}. Run 'gcloud auth login' to authenticate.",
                stderr.trim()
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn convert_messages(messages: &[Message]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
        let mut system = None;
        let mut contents = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    system = Some(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart {
                            text: Some(msg.content.clone()),
                            inline_data: None,
                        }],
                    });
                }
                Role::User => {
                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: vec![GeminiPart {
                            text: Some(msg.content.clone()),
                            inline_data: None,
                        }],
                    });
                }
                Role::Assistant => {
                    contents.push(GeminiContent {
                        role: "model".to_string(),
                        parts: vec![GeminiPart {
                            text: Some(msg.content.clone()),
                            inline_data: None,
                        }],
                    });
                }
            }
        }

        (system, contents)
    }

    fn is_imagen_model(&self) -> bool {
        self.model.contains("imagen")
    }
}

#[async_trait]
impl Provider for VertexAiClient {
    fn name(&self) -> &str {
        "vertex-ai"
    }

    async fn chat(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = format!("{}:generateContent", self.base_url());
        debug!(url = %url, model = %self.model, "Sending chat request to Vertex AI");

        let token = Self::get_access_token().await?;
        let (system, contents) = Self::convert_messages(messages);

        let generation_config = options.map(|o| GenerationConfig {
            max_output_tokens: o.max_tokens,
            temperature: o.temperature,
        });

        let request = GenerateContentRequest {
            contents,
            system_instruction: system,
            generation_config,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Vertex AI")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .ok()
                .and_then(|e| e.error.map(|d| d.message))
                .unwrap_or(body);
            anyhow::bail!("Vertex AI API error ({}): {}", status.as_u16(), message);
        }

        let api_response: GenerateContentResponse = response
            .json()
            .await
            .context("Failed to parse Vertex AI response")?;

        let content = api_response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.content.as_ref())
            .and_then(|c| c.parts.first())
            .and_then(|p| p.text.clone())
            .unwrap_or_default();

        let model = api_response
            .model_version
            .unwrap_or_else(|| self.model.clone());

        let usage = api_response.usage_metadata.map(|u| Usage {
            prompt_tokens: u.prompt_token_count.unwrap_or(0),
            completion_tokens: u.candidates_token_count.unwrap_or(0),
            total_tokens: u.total_token_count.unwrap_or(0),
        });

        Ok(ChatResponse {
            content,
            model,
            usage,
        })
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        let url = format!("{}:streamGenerateContent?alt=sse", self.base_url());
        debug!(url = %url, model = %self.model, "Sending streaming chat request to Vertex AI");

        let token = Self::get_access_token().await?;
        let (system, contents) = Self::convert_messages(messages);

        let generation_config = options.map(|o| GenerationConfig {
            max_output_tokens: o.max_tokens,
            temperature: o.temperature,
        });

        let request = GenerateContentRequest {
            contents,
            system_instruction: system,
            generation_config,
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request to Vertex AI")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .ok()
                .and_then(|e| e.error.map(|d| d.message))
                .unwrap_or(body);
            anyhow::bail!("Vertex AI API error ({}): {}", status.as_u16(), message);
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
                            if let Ok(chunk) = serde_json::from_str::<GenerateContentResponse>(data)
                            {
                                if let Some(text) = chunk
                                    .candidates
                                    .as_ref()
                                    .and_then(|c| c.first())
                                    .and_then(|c| c.content.as_ref())
                                    .and_then(|c| c.parts.first())
                                    .and_then(|p| p.text.clone())
                                {
                                    if !text.is_empty() {
                                        assembled.push_str(&text);
                                        return Some((
                                            Ok(StreamEvent::Delta(text)),
                                            (byte_stream, buffer, assembled, model),
                                        ));
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

    async fn generate_image(
        &self,
        prompt: &str,
        _options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        let token = Self::get_access_token().await?;

        if self.is_imagen_model() {
            // Imagen endpoint
            let url = format!("{}:predict", self.base_url());
            debug!(url = %url, model = %self.model, "Sending image generation request to Vertex AI (Imagen)");

            let request = ImagenRequest {
                instances: vec![ImagenInstance {
                    prompt: prompt.to_string(),
                }],
                parameters: ImagenParameters { sample_count: 1 },
            };

            let response = self
                .client
                .post(&url)
                .bearer_auth(&token)
                .json(&request)
                .send()
                .await
                .context("Failed to send image generation request to Vertex AI")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Vertex AI image generation error ({}): {}",
                    status.as_u16(),
                    body
                );
            }

            let api_response: ImagenResponse = response
                .json()
                .await
                .context("Failed to parse Vertex AI image generation response")?;

            let prediction = api_response
                .predictions
                .as_ref()
                .and_then(|p| p.first())
                .context("No image data in Vertex AI response")?;

            let data = base64::engine::general_purpose::STANDARD
                .decode(&prediction.bytes_base64_encoded)
                .context("Failed to decode base64 image data")?;

            let format = match prediction.mime_type.as_deref() {
                Some("image/jpeg") => ImageFormat::Jpeg,
                Some("image/webp") => ImageFormat::Webp,
                _ => ImageFormat::Png,
            };

            let (width, height) = crate::types::image_dimensions(&data).unwrap_or((1024, 1024));

            Ok(ImageResponse {
                data,
                width,
                height,
                format,
                revised_prompt: None,
            })
        } else {
            // Gemini with image generation (Nano Banana style)
            let url = format!("{}:generateContent", self.base_url());
            debug!(url = %url, model = %self.model, "Sending image generation request to Vertex AI (Gemini)");

            let request = serde_json::json!({
                "contents": [{
                    "role": "user",
                    "parts": [{"text": prompt}]
                }],
                "generationConfig": {
                    "responseModalities": ["IMAGE", "TEXT"]
                }
            });

            let response = self
                .client
                .post(&url)
                .bearer_auth(&token)
                .json(&request)
                .send()
                .await
                .context("Failed to send image generation request to Vertex AI")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Vertex AI image generation error ({}): {}",
                    status.as_u16(),
                    body
                );
            }

            let api_response: GenerateContentResponse = response
                .json()
                .await
                .context("Failed to parse Vertex AI response")?;

            // Look for inline image data in the response
            let part = api_response
                .candidates
                .as_ref()
                .and_then(|c| c.first())
                .and_then(|c| c.content.as_ref())
                .and_then(|c| c.parts.iter().find(|p| p.inline_data.is_some()))
                .context("No image data in Vertex AI response")?;

            let inline = part
                .inline_data
                .as_ref()
                .context("No inline data in response")?;

            let data = base64::engine::general_purpose::STANDARD
                .decode(&inline.data)
                .context("Failed to decode base64 image data")?;

            let format = match inline.mime_type.as_str() {
                "image/jpeg" => ImageFormat::Jpeg,
                "image/webp" => ImageFormat::Webp,
                _ => ImageFormat::Png,
            };

            let (width, height) = crate::types::image_dimensions(&data).unwrap_or((1024, 1024));

            Ok(ImageResponse {
                data,
                width,
                height,
                format,
                revised_prompt: None,
            })
        }
    }

    async fn embed(
        &self,
        texts: &[&str],
        _options: Option<&EmbedOptions>,
    ) -> Result<EmbedResponse> {
        let url = format!("{}:predict", self.base_url());
        debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request to Vertex AI");
        let token = Self::get_access_token().await?;
        let request = EmbedPredictRequest {
            instances: texts
                .iter()
                .map(|t| EmbedInstance {
                    content: t.to_string(),
                })
                .collect(),
        };
        let response = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .context("Failed to send embedding request to Vertex AI")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiError>(&body)
                .ok()
                .and_then(|e| e.error.map(|d| d.message))
                .unwrap_or(body);
            anyhow::bail!(
                "Vertex AI embedding error ({}): {}",
                status.as_u16(),
                message
            );
        }
        let api_response: EmbedPredictResponse = response
            .json()
            .await
            .context("Failed to parse Vertex AI embedding response")?;
        Ok(EmbedResponse {
            embeddings: api_response
                .predictions
                .into_iter()
                .map(|p| p.embeddings.values)
                .collect(),
            model: self.model.clone(),
            usage: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_response_parsing() {
        let json = r#"{"predictions":[{"embeddings":{"values":[0.1,0.2,0.3]}},{"embeddings":{"values":[0.4,0.5,0.6]}}]}"#;
        let response: EmbedPredictResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.predictions.len(), 2);
        assert_eq!(
            response.predictions[0].embeddings.values,
            vec![0.1, 0.2, 0.3]
        );
    }
}
