//! Microsoft Foundry client.
//!
//! Supports chat completions and streaming via the Model
//! Inference API (`*.services.ai.azure.com`).

use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::azure::AzureAuth;
use crate::client::Provider;
use crate::openai_images::{
    ImageFlavor, build_edits_form, build_generations_body, parse_images_response, wants_edits,
};
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageOptions,
    ImageResponse, Message, StreamEvent, Usage, VideoJob, VideoOptions, VideoResponse,
};
use crate::video_jobs::VideoJobsApi;

/// Client for Microsoft Foundry (AI Services).
pub struct FoundryClient {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    /// Dated `api-version` for the legacy `/models/...` inference endpoints.
    /// `None` (the default) uses the unified `/openai/v1/` surface.
    api_version: Option<String>,
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

// Embedding types
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
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

/// Pick the image request shape for a model/deployment name.
///
/// Mirrors `azure::flavor_for`: Foundry only ever exposes DALL·E or
/// gpt-image style deployments; anything not explicitly named `dall-e*` is
/// treated as a gpt-image deployment (including custom deployment names
/// that don't echo the underlying model).
fn flavor_for(name: &str) -> ImageFlavor {
    if name.starts_with("dall-e") {
        ImageFlavor::DallE
    } else {
        ImageFlavor::AzureGptImage
    }
}

impl FoundryClient {
    /// Create a new Microsoft Foundry client on the unified `/openai/v1/`
    /// surface (recommended). Use [`FoundryClient::with_api_version`] for the
    /// legacy dated `/models/...` endpoints.
    pub fn new(endpoint: impl Into<String>, model: impl Into<String>, auth: AzureAuth) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            model: model.into(),
            api_version: None,
            auth,
        }
    }

    /// Create a client pinned to a legacy dated `api-version`
    /// (`/models/chat/completions?api-version=...`).
    pub fn with_api_version(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_version: impl Into<String>,
        auth: AzureAuth,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.into(),
            model: model.into(),
            api_version: Some(api_version.into()),
            auth,
        }
    }

    fn base_url(&self) -> String {
        let url = self.endpoint.trim_end_matches('/').to_string();
        // Model Inference API lives on *.services.ai.azure.com, not *.cognitiveservices.azure.com
        url.replace(".cognitiveservices.azure.com", ".services.ai.azure.com")
    }

    fn chat_url(&self) -> String {
        match &self.api_version {
            None => format!("{}/openai/v1/chat/completions", self.base_url()),
            Some(version) => format!(
                "{}/models/chat/completions?api-version={version}",
                self.base_url()
            ),
        }
    }

    fn embed_url(&self) -> String {
        match &self.api_version {
            None => format!("{}/openai/v1/embeddings", self.base_url()),
            Some(version) => format!(
                "{}/models/embeddings?api-version={version}",
                self.base_url()
            ),
        }
    }

    /// Image generation URL. Unlike `chat_url`/`embed_url`, the legacy dated
    /// surface for images uses the `/openai/deployments/{model}/...` path
    /// (like Azure OpenAI), not `/models/...` — Foundry's image API is
    /// exposed through the Azure OpenAI-compatible surface, with the model
    /// field doubling as the deployment name.
    fn image_url(&self) -> String {
        match &self.api_version {
            None => format!("{}/openai/v1/images/generations", self.base_url()),
            Some(version) => format!(
                "{}/openai/deployments/{}/images/generations?api-version={version}",
                self.base_url(),
                self.model
            ),
        }
    }

    /// Image edits URL; see [`FoundryClient::image_url`] for the v1-vs-dated
    /// path rule.
    fn edits_url(&self) -> String {
        match &self.api_version {
            None => format!("{}/openai/v1/images/edits", self.base_url()),
            Some(version) => format!(
                "{}/openai/deployments/{}/images/edits?api-version={version}",
                self.base_url(),
                self.model
            ),
        }
    }

    /// Model value for v1 request bodies (`None` on dated endpoints, where
    /// the deployment is part of the URL). Mirrors `AzureOpenAiClient::body_model`.
    fn body_model(&self) -> Option<&str> {
        self.api_version.is_none().then_some(self.model.as_str())
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

        let mut temperature = options.and_then(|o| o.temperature);
        let response = loop {
            let request = ChatRequest {
                model: &self.model,
                messages,
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
                .context("Failed to send request to Microsoft Foundry")?;

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
            .context("Failed to send streaming request to Microsoft Foundry")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let model = self.model.clone();
        let byte_stream = response.bytes_stream();

        let stream = futures_util::stream::unfold(
            (
                byte_stream,
                String::new(),
                String::new(),
                model,
                None::<Usage>,
            ),
            |(mut byte_stream, mut buffer, mut assembled, model, mut usage)| async move {
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
                                    usage: usage.take(),
                                };
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, model, usage),
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
                                                (byte_stream, buffer, assembled, model, usage),
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
                                (byte_stream, buffer, assembled, model, usage),
                            ));
                        }
                        None => {
                            if !assembled.is_empty() {
                                let response = ChatResponse {
                                    content: assembled.clone(),
                                    model: model.clone(),
                                    usage: usage.take(),
                                };
                                assembled.clear();
                                return Some((
                                    Ok(StreamEvent::Done(response)),
                                    (byte_stream, buffer, assembled, model, usage),
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
        let flavor = flavor_for(&self.model);

        let response = if wants_edits(options) {
            // `wants_edits` only returns true when `options` is `Some` with
            // non-empty `reference_images`.
            let opts = options.expect("wants_edits(true) implies options is Some");
            let url = self.edits_url();
            debug!(url = %url, "Sending image edit request to Microsoft Foundry");

            let form = build_edits_form(self.body_model(), prompt, opts).await?;
            self.client
                .post(&url)
                .header(header_name, &header_value)
                .multipart(form)
                .send()
                .await
                .context("Failed to send image edit request to Microsoft Foundry")?
        } else {
            let url = self.image_url();
            debug!(url = %url, "Sending image generation request to Microsoft Foundry");

            let body = build_generations_body(self.body_model(), prompt, options, flavor)?;
            self.client
                .post(&url)
                .header(header_name, &header_value)
                .json(&body)
                .send()
                .await
                .context("Failed to send image generation request to Microsoft Foundry")?
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let body = response
            .text()
            .await
            .context("Failed to read Microsoft Foundry image response")?;

        parse_images_response(&body, options.and_then(|o| o.size))
    }

    async fn embed(&self, texts: &[&str], options: Option<&EmbedOptions>) -> Result<EmbedResponse> {
        let url = self.embed_url();
        debug!(url = %url, model = %self.model, count = texts.len(), "Sending embedding request to Microsoft Foundry");

        let (header_name, header_value) = self.get_auth_header().await?;

        let request = EmbedRequest {
            model: &self.model,
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
            .context("Failed to send embedding request to Microsoft Foundry")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("{}", self.format_api_error(status.as_u16(), &body));
        }

        let api_response: EmbedApiResponse = response
            .json()
            .await
            .context("Failed to parse Microsoft Foundry embedding response")?;

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
        debug!(model = %self.model, "Creating video generation job on Microsoft Foundry");
        let header = self.get_auth_header().await?;
        self.video_api(header)
            .create(&self.model, prompt, options)
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
}

#[cfg(test)]
mod v1_surface_tests {
    use super::*;
    use crate::azure::AzureAuth;

    #[test]
    fn default_uses_unified_v1_and_normalizes_host() {
        let c = FoundryClient::new(
            "https://acct.cognitiveservices.azure.com",
            "claude-sonnet-5",
            AzureAuth::AzureCli,
        );
        assert_eq!(
            c.chat_url(),
            "https://acct.services.ai.azure.com/openai/v1/chat/completions"
        );
        assert_eq!(
            c.embed_url(),
            "https://acct.services.ai.azure.com/openai/v1/embeddings"
        );
        assert_eq!(
            c.image_url(),
            "https://acct.services.ai.azure.com/openai/v1/images/generations"
        );
        assert_eq!(
            c.edits_url(),
            "https://acct.services.ai.azure.com/openai/v1/images/edits"
        );
        assert_eq!(
            c.body_model(),
            Some("claude-sonnet-5"),
            "v1 bodies carry the model as deployment name"
        );
    }

    #[test]
    fn dated_api_version_uses_legacy_models_path() {
        let c = FoundryClient::with_api_version(
            "https://acct.services.ai.azure.com",
            "m",
            "2024-05-01-preview",
            AzureAuth::AzureCli,
        );
        assert_eq!(
            c.chat_url(),
            "https://acct.services.ai.azure.com/models/chat/completions?api-version=2024-05-01-preview"
        );
    }

    #[test]
    fn dated_api_version_uses_legacy_deployments_path_for_images() {
        // Unlike chat/embed (which use /models/... on the dated surface),
        // the legacy image endpoints follow the Azure OpenAI-style
        // /openai/deployments/{model}/... path.
        let c = FoundryClient::with_api_version(
            "https://acct.services.ai.azure.com",
            "gpt-image-1-deployment",
            "2024-05-01-preview",
            AzureAuth::AzureCli,
        );
        assert_eq!(
            c.image_url(),
            "https://acct.services.ai.azure.com/openai/deployments/gpt-image-1-deployment/images/generations?api-version=2024-05-01-preview"
        );
        assert_eq!(
            c.edits_url(),
            "https://acct.services.ai.azure.com/openai/deployments/gpt-image-1-deployment/images/edits?api-version=2024-05-01-preview"
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
    fn wants_edits_routes_to_edits_url_when_reference_images_present() {
        use crate::types::ImageOptions;

        let c = FoundryClient::new(
            "https://acct.services.ai.azure.com",
            "gpt-image-1",
            AzureAuth::AzureCli,
        );
        let opts = ImageOptions::builder()
            .reference_image(std::path::PathBuf::from("ref.png"))
            .build();
        assert!(wants_edits(Some(&opts)));
        // The routing decision itself just picks which URL function to call;
        // confirm both resolve to the expected distinct endpoints.
        assert!(c.edits_url().ends_with("/images/edits"));
        assert!(c.image_url().ends_with("/images/generations"));
        assert_ne!(c.edits_url(), c.image_url());
    }

    #[test]
    fn no_reference_images_routes_to_generations_url() {
        use crate::types::ImageOptions;

        assert!(!wants_edits(None));
        assert!(!wants_edits(Some(&ImageOptions::default())));
    }

    #[test]
    fn video_api_builds_urls_from_client_base_and_api_version() {
        let c = FoundryClient::new(
            "https://acct.cognitiveservices.azure.com",
            "sora-2",
            AzureAuth::AzureCli,
        );
        let api = c.video_api(("Authorization", "Bearer test".to_string()));
        assert_eq!(
            api.jobs_url(),
            "https://acct.services.ai.azure.com/openai/v1/video/generations/jobs?api-version=preview"
        );
        assert_eq!(
            api.job_url("job-1"),
            "https://acct.services.ai.azure.com/openai/v1/video/generations/jobs/job-1?api-version=preview"
        );

        let c = FoundryClient::with_api_version(
            "https://acct.services.ai.azure.com",
            "sora-2",
            "2025-04-01-preview",
            AzureAuth::AzureCli,
        );
        let api = c.video_api(("Authorization", "Bearer test".to_string()));
        assert_eq!(
            api.jobs_url(),
            "https://acct.services.ai.azure.com/openai/v1/video/generations/jobs?api-version=2025-04-01-preview"
        );
    }
}
