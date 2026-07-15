//! Azure OpenAI Service client.

use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::client::Provider;
use crate::openai_images::{
    ImageFlavor, build_edits_form, build_generations_body, parse_images_response, wants_edits,
};
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageOptions,
    ImageResponse, Message, StreamEvent, Usage, VideoJob, VideoOptions, VideoResponse,
};
use crate::video_jobs::VideoJobsApi;

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
    /// Dated `api-version` for the legacy per-deployment endpoints.
    /// `None` (the default) uses the unified `/openai/v1/` surface that
    /// Microsoft recommends since August 2025 — no api-version at all.
    api_version: Option<String>,
    auth: AzureAuth,
}

// Request types
#[derive(Serialize)]
struct ChatRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    /// OpenAI Chat Completions message array, built via
    /// [`crate::types::to_openai_wire`]; identical body shape on the v1 and
    /// dated surfaces.
    messages: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
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

// Embedding types
#[derive(Serialize)]
struct EmbedRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
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

/// Pick the image request shape for a deployment/model name.
///
/// Azure/Foundry only ever exposes DALL·E or gpt-image style deployments;
/// anything not explicitly named `dall-e*` is treated as a gpt-image
/// deployment (including custom deployment names that don't echo the
/// underlying model).
fn flavor_for(name: &str) -> ImageFlavor {
    if name.starts_with("dall-e") {
        ImageFlavor::DallE
    } else {
        ImageFlavor::AzureGptImage
    }
}

impl AzureOpenAiClient {
    /// Create a new Azure OpenAI client on the unified `/openai/v1/` surface
    /// (recommended). Use [`AzureOpenAiClient::with_api_version`] for the
    /// legacy dated endpoints.
    pub fn new(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        auth: AzureAuth,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            deployment: deployment.into(),
            api_version: None,
            auth,
        }
    }

    /// Create a client pinned to a legacy dated `api-version`
    /// (`/openai/deployments/{d}/...?api-version=...`).
    pub fn with_api_version(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        api_version: impl Into<String>,
        auth: AzureAuth,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            deployment: deployment.into(),
            api_version: Some(api_version.into()),
            auth,
        }
    }

    /// Model value for v1 request bodies (None on dated endpoints, where the
    /// deployment is part of the URL).
    fn body_model(&self) -> Option<&str> {
        self.api_version
            .is_none()
            .then_some(self.deployment.as_str())
    }

    fn base_url(&self) -> String {
        self.endpoint.trim_end_matches('/').to_string()
    }

    fn url_for(&self, op: &str) -> String {
        match &self.api_version {
            None => format!("{}/openai/v1/{op}", self.base_url()),
            Some(version) => format!(
                "{}/openai/deployments/{}/{op}?api-version={version}",
                self.base_url(),
                self.deployment
            ),
        }
    }

    fn chat_url(&self) -> String {
        self.url_for("chat/completions")
    }

    fn image_url(&self) -> String {
        self.url_for("images/generations")
    }

    fn edits_url(&self) -> String {
        self.url_for("images/edits")
    }

    fn embed_url(&self) -> String {
        self.url_for("embeddings")
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

    /// Build a [`VideoJobsApi`] scoped to this client's endpoint and
    /// api-version, using the given auth header.
    fn video_api(&self, header: (&'static str, String)) -> VideoJobsApi<'_> {
        VideoJobsApi {
            client: &self.client,
            base: self.base_url(),
            api_version: self.api_version.as_deref(),
            header,
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

        let wire_messages = crate::types::to_openai_wire(messages);
        let mut temperature = options.and_then(|o| o.temperature);
        let response = loop {
            let request = ChatRequest {
                model: self.body_model(),
                messages: wire_messages.clone(),
                max_completion_tokens: options.and_then(|o| o.max_tokens),
                temperature,
                stream: false,
                stream_options: None,
                response_format: options
                    .and_then(|o| o.response_format.as_ref())
                    .map(|f| f.to_openai_value()),
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
                if temperature.is_some()
                    && crate::types::is_sampling_rejection(status.as_u16(), &body)
                {
                    debug!("model rejected sampling parameters; retrying without temperature");
                    temperature = None;
                    continue;
                }
                anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
            }
            break response;
        };

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
            model: self.body_model(),
            messages: crate::types::to_openai_wire(messages),
            max_completion_tokens: options.and_then(|o| o.max_tokens),
            temperature: options.and_then(|o| o.temperature),
            stream: true,
            // usage in the final chunk; supported on the v1 surface
            stream_options: self.api_version.is_none().then_some(StreamOptions {
                include_usage: true,
            }),
            response_format: options
                .and_then(|o| o.response_format.as_ref())
                .map(|f| f.to_openai_value()),
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
            (
                byte_stream,
                String::new(),
                String::new(),
                deployment,
                None::<Usage>,
            ),
            |(mut byte_stream, mut buffer, mut assembled, deployment, mut usage)| async move {
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
                                    usage: usage.take(),
                                };
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, deployment, usage),
                                ));
                            }

                            if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) {
                                if let Some(u) = chunk.usage {
                                    usage = Some(Usage {
                                        prompt_tokens: u.prompt_tokens,
                                        completion_tokens: u.completion_tokens,
                                        total_tokens: u.total_tokens,
                                    });
                                }
                                if let Some(choice) = chunk.choices.first() {
                                    if let Some(text) = &choice.delta.content {
                                        if !text.is_empty() {
                                            assembled.push_str(text);
                                            return Some((
                                                Ok(StreamEvent::Delta(text.clone())),
                                                (byte_stream, buffer, assembled, deployment, usage),
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
                                (byte_stream, buffer, assembled, deployment, usage),
                            ));
                        }
                        None => {
                            if !assembled.is_empty() {
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: deployment.clone(),
                                    usage: usage.take(),
                                };
                                assembled.clear();
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, deployment, usage),
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

    async fn generate_images(
        &self,
        prompt: &str,
        options: Option<&ImageOptions>,
    ) -> Result<Vec<ImageResponse>> {
        let (header_name, header_value) = self.get_auth_header().await?;
        let flavor = flavor_for(&self.deployment);

        let response = if wants_edits(options) {
            // `wants_edits` only returns true when `options` is `Some` with
            // non-empty `reference_images`.
            let opts = options.expect("wants_edits(true) implies options is Some");
            let url = self.edits_url();
            debug!(url = %url, "Sending image edit request to Azure OpenAI");

            let form = build_edits_form(self.body_model(), prompt, opts).await?;
            self.client
                .post(&url)
                .header(header_name, &header_value)
                .multipart(form)
                .send()
                .await
                .context("Failed to send image edit request to Azure OpenAI")?
        } else {
            let url = self.image_url();
            debug!(url = %url, "Sending image generation request to Azure OpenAI");

            let body = build_generations_body(self.body_model(), prompt, options, flavor)?;
            self.client
                .post(&url)
                .header(header_name, &header_value)
                .json(&body)
                .send()
                .await
                .context("Failed to send image generation request to Azure OpenAI")?
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let body = response
            .text()
            .await
            .context("Failed to read Azure OpenAI image response")?;

        parse_images_response(&body, options.and_then(|o| o.size))
    }

    async fn embed(&self, texts: &[&str], options: Option<&EmbedOptions>) -> Result<EmbedResponse> {
        let url = self.embed_url();
        debug!(url = %url, deployment = %self.deployment, count = texts.len(), "Sending embedding request to Azure OpenAI");

        let (header_name, header_value) = self.get_auth_header().await?;

        let request = EmbedRequest {
            model: self.body_model(),
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

    async fn create_video_job(
        &self,
        prompt: &str,
        options: Option<&VideoOptions>,
    ) -> Result<VideoJob> {
        if let Some(options) = options {
            options.validate()?;
        }
        debug!(deployment = %self.deployment, "Creating video generation job on Azure OpenAI");
        let header = self.get_auth_header().await?;
        self.video_api(header)
            .create(&self.deployment, prompt, options)
            .await
    }

    async fn get_video_job(&self, id: &str) -> Result<VideoJob> {
        let header = self.get_auth_header().await?;
        self.video_api(header).get(id).await
    }

    async fn download_video(&self, generation_id: &str) -> Result<VideoResponse> {
        let header = self.get_auth_header().await?;
        self.video_api(header).download(generation_id).await
    }

    async fn delete_video_job(&self, id: &str) -> Result<()> {
        let header = self.get_auth_header().await?;
        self.video_api(header).delete(id).await
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

    #[test]
    fn chat_request_embeds_wire_messages() {
        use crate::types::{ContentPart, Message, MessageContent};

        // Text-only regression: content stays a plain string in the body.
        let text_req = ChatRequest {
            model: Some("gpt-5-mini"),
            messages: crate::types::to_openai_wire(&[Message::user("hello")]),
            max_completion_tokens: None,
            temperature: None,
            stream: false,
            stream_options: None,
            response_format: None,
        };
        let json = serde_json::to_value(&text_req).unwrap();
        assert_eq!(
            json["messages"],
            serde_json::json!([{"role": "user", "content": "hello"}])
        );

        // Attachment message embeds the typed content array.
        let content = MessageContent::from(vec![
            ContentPart::Text {
                text: "see".to_string(),
            },
            ContentPart::Image {
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
                media_type: "image/png".to_string(),
            },
        ]);
        let img_req = ChatRequest {
            model: Some("gpt-5-mini"),
            messages: crate::types::to_openai_wire(&[Message::user(content)]),
            max_completion_tokens: None,
            temperature: None,
            stream: false,
            stream_options: None,
            response_format: None,
        };
        let json = serde_json::to_value(&img_req).unwrap();
        assert_eq!(json["messages"][0]["content"][1]["type"], "image_url");
        assert_eq!(
            json["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,3q2+7w=="
        );
    }
}

#[cfg(test)]
mod v1_surface_tests {
    use super::*;

    #[test]
    fn default_uses_unified_v1_urls() {
        let c = AzureOpenAiClient::new(
            "https://r.openai.azure.com",
            "gpt-5-mini",
            AzureAuth::AzureCli,
        );
        assert_eq!(
            c.chat_url(),
            "https://r.openai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            c.embed_url(),
            "https://r.openai.azure.com/openai/v1/embeddings"
        );
        assert_eq!(
            c.image_url(),
            "https://r.openai.azure.com/openai/v1/images/generations"
        );
        assert_eq!(
            c.edits_url(),
            "https://r.openai.azure.com/openai/v1/images/edits"
        );
        assert_eq!(
            c.body_model(),
            Some("gpt-5-mini"),
            "v1 bodies carry the deployment as model"
        );
    }

    #[test]
    fn dated_api_version_uses_legacy_urls() {
        let c = AzureOpenAiClient::with_api_version(
            "https://r.openai.azure.com/",
            "dep",
            "2024-10-21",
            AzureAuth::AzureCli,
        );
        assert_eq!(
            c.chat_url(),
            "https://r.openai.azure.com/openai/deployments/dep/chat/completions?api-version=2024-10-21"
        );
        assert_eq!(
            c.edits_url(),
            "https://r.openai.azure.com/openai/deployments/dep/images/edits?api-version=2024-10-21"
        );
        assert_eq!(c.body_model(), None, "legacy bodies must not carry model");
    }

    #[test]
    fn flavor_for_selects_dalle_for_dalle_deployments() {
        assert!(matches!(flavor_for("dall-e-3"), ImageFlavor::DallE));
        assert!(matches!(flavor_for("dall-e-2"), ImageFlavor::DallE));
    }

    #[test]
    fn flavor_for_selects_azure_gpt_image_for_everything_else() {
        assert!(matches!(
            flavor_for("gpt-image-1"),
            ImageFlavor::AzureGptImage
        ));
        assert!(matches!(
            flavor_for("my-custom-deployment"),
            ImageFlavor::AzureGptImage
        ));
    }

    #[test]
    fn video_api_builds_urls_from_client_base_and_api_version() {
        let c = AzureOpenAiClient::new(
            "https://r.openai.azure.com",
            "sora-2-deployment",
            AzureAuth::AzureCli,
        );
        let api = c.video_api(("Authorization", "Bearer test".to_string()));
        assert_eq!(
            api.jobs_url(),
            "https://r.openai.azure.com/openai/v1/video/generations/jobs?api-version=preview"
        );
        assert_eq!(
            api.job_url("job-1"),
            "https://r.openai.azure.com/openai/v1/video/generations/jobs/job-1?api-version=preview"
        );

        let c = AzureOpenAiClient::with_api_version(
            "https://r.openai.azure.com/",
            "sora-2-deployment",
            "2025-04-01-preview",
            AzureAuth::AzureCli,
        );
        let api = c.video_api(("Authorization", "Bearer test".to_string()));
        assert_eq!(
            api.jobs_url(),
            "https://r.openai.azure.com/openai/v1/video/generations/jobs?api-version=2025-04-01-preview"
        );
    }
}
