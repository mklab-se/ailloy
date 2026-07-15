//! Client abstraction and Provider trait.

use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::config::{AiNode, Auth, Config, ProviderKind};
use crate::error::ClientError;
use crate::types::{
    Background, ChatOptions, ChatResponse, ChatStream, EmbedOptions, EmbedResponse, ImageFormat,
    ImageOptions, ImageResponse, Message, Task, VideoJob, VideoJobStatus, VideoOptions,
    VideoProgress, VideoResponse,
};

/// Unified provider trait. Override methods for the capabilities you support.
/// Methods you don't override return `ClientError::Unsupported`.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Returns the name of this provider instance.
    fn name(&self) -> &str;

    /// Send a chat completion request.
    async fn chat(
        &self,
        _messages: &[Message],
        _options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        Err(ClientError::Unsupported("chat".to_string()).into())
    }

    /// Send a streaming chat completion request.
    async fn chat_stream(
        &self,
        _messages: &[Message],
        _options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        Err(ClientError::Unsupported("streaming".to_string()).into())
    }

    /// Generate one or more images from a text prompt.
    async fn generate_images(
        &self,
        _prompt: &str,
        _options: Option<&ImageOptions>,
    ) -> Result<Vec<ImageResponse>> {
        Err(ClientError::Unsupported("image generation".to_string()).into())
    }

    /// Generate an image from a text prompt.
    ///
    /// Default implementation delegates to [`Provider::generate_images`] and
    /// returns the first image.
    async fn generate_image(
        &self,
        prompt: &str,
        options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        let images = self.generate_images(prompt, options).await?;
        images
            .into_iter()
            .next()
            .context("Provider returned no images")
    }

    /// Generate embeddings for the given texts.
    async fn embed(
        &self,
        _texts: &[&str],
        _options: Option<&EmbedOptions>,
    ) -> Result<EmbedResponse> {
        Err(ClientError::Unsupported("embedding".to_string()).into())
    }

    /// Create an asynchronous video generation job.
    async fn create_video_job(
        &self,
        _prompt: &str,
        _options: Option<&VideoOptions>,
    ) -> Result<VideoJob> {
        Err(ClientError::Unsupported("video generation".to_string()).into())
    }

    /// Fetch the current state of a video generation job.
    async fn get_video_job(&self, _id: &str) -> Result<VideoJob> {
        Err(ClientError::Unsupported("video generation".to_string()).into())
    }

    /// Download a completed video generation by generation ID.
    async fn download_video(&self, _generation_id: &str) -> Result<VideoResponse> {
        Err(ClientError::Unsupported("video generation".to_string()).into())
    }

    /// Delete a video generation job and its artifacts.
    async fn delete_video_job(&self, _id: &str) -> Result<()> {
        Err(ClientError::Unsupported("video generation".to_string()).into())
    }

    /// Generate one or more videos from a text prompt.
    ///
    /// Default implementation creates a job via [`Provider::create_video_job`],
    /// polls [`Provider::get_video_job`] until the job reaches a terminal
    /// state (with exponential backoff, capped overall at 15 minutes), then
    /// downloads every resulting generation via [`Provider::download_video`].
    async fn generate_video(
        &self,
        prompt: &str,
        options: Option<&VideoOptions>,
        progress: Option<VideoProgress<'_>>,
    ) -> Result<Vec<VideoResponse>> {
        let job = self.create_video_job(prompt, options).await?;
        let job =
            poll_video_job_until_terminal(self, &job.id, 2_000, 10_000, 900_000, progress).await?;

        match job.status {
            VideoJobStatus::Succeeded => {
                if job.generation_ids.is_empty() {
                    anyhow::bail!(
                        "job succeeded but returned no generations (job id {})",
                        job.id
                    );
                }
                let mut videos = Vec::with_capacity(job.generation_ids.len());
                for generation_id in &job.generation_ids {
                    videos.push(self.download_video(generation_id).await?);
                }
                Ok(videos)
            }
            VideoJobStatus::Failed | VideoJobStatus::Cancelled => {
                let reason = job
                    .failure_reason
                    .unwrap_or_else(|| "no reason given".to_string());
                anyhow::bail!(
                    "video generation job {} ended with status '{}': {}",
                    job.id,
                    job.status,
                    reason
                );
            }
            _ => anyhow::bail!(
                "video generation job {} did not reach a terminal state",
                job.id
            ),
        }
    }
}

/// Poll a video generation job until it reaches a terminal status, invoking
/// `progress` after each poll and backing off exponentially between polls.
///
/// Exposed as `pub(crate)` so tests can drive it with tiny delays and a
/// tight timeout.
pub(crate) async fn poll_video_job_until_terminal<P: Provider + ?Sized>(
    provider: &P,
    job_id: &str,
    initial_delay_ms: u64,
    max_delay_ms: u64,
    timeout_ms: u64,
    progress: Option<VideoProgress<'_>>,
) -> Result<VideoJob> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let mut delay_ms = initial_delay_ms;

    loop {
        let job = provider.get_video_job(job_id).await?;
        if let Some(cb) = progress {
            cb(&job);
        }
        if job.status.is_terminal() {
            return Ok(job);
        }

        if start.elapsed() >= timeout {
            anyhow::bail!(
                "video generation job {} timed out after {}ms waiting for completion; \
                 resume with get_video_job()/download_video(); job artifacts expire after 24 hours",
                job_id,
                timeout_ms
            );
        }

        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * 2).min(max_delay_ms);
    }
}

// ---------------------------------------------------------------------------
// Per-node default parameter resolution
// ---------------------------------------------------------------------------

/// Log a hand-edited node default whose stored value could not be parsed.
fn warn_unparseable_default(key: &str, value: &str) {
    tracing::warn!(
        "ignoring node default '{}' = '{}': value could not be parsed for this parameter",
        key,
        value
    );
}

/// Parse a `WxH` size string into `(width, height)`, rejecting zero dims.
fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once('x')?;
    let width: u32 = w.parse().ok()?;
    let height: u32 = h.parse().ok()?;
    if width == 0 || height == 0 {
        None
    } else {
        Some((width, height))
    }
}

/// Fill only the `None` fields of `opts` from a node's stored `image.*`
/// defaults. Explicit (already-`Some`) values always win. Unparseable stored
/// values are logged and skipped.
pub fn merge_image_defaults(opts: &mut ImageOptions, defaults: &BTreeMap<String, String>) {
    if opts.size.is_none() {
        if let Some(v) = defaults.get("image.size") {
            match parse_size(v) {
                Some(size) => opts.size = Some(size),
                None => warn_unparseable_default("image.size", v),
            }
        }
    }
    if opts.quality.is_none() {
        if let Some(v) = defaults.get("image.quality") {
            opts.quality = Some(v.clone());
        }
    }
    if opts.output_format.is_none() {
        if let Some(v) = defaults.get("image.format") {
            match ImageFormat::from_str(v) {
                Ok(fmt) => opts.output_format = Some(fmt),
                Err(_) => warn_unparseable_default("image.format", v),
            }
        }
    }
    if opts.compression.is_none() {
        if let Some(v) = defaults.get("image.compression") {
            match v.parse::<u8>() {
                Ok(c) => opts.compression = Some(c),
                Err(_) => warn_unparseable_default("image.compression", v),
            }
        }
    }
    if opts.n.is_none() {
        if let Some(v) = defaults.get("image.variants") {
            match v.parse::<u8>() {
                Ok(n) => opts.n = Some(n),
                Err(_) => warn_unparseable_default("image.variants", v),
            }
        }
    }
    if opts.background.is_none() {
        if let Some(v) = defaults.get("image.background") {
            match Background::from_str(v) {
                Ok(bg) => opts.background = Some(bg),
                Err(_) => warn_unparseable_default("image.background", v),
            }
        }
    }
}

/// Fill only the `None` fields of `opts` from a node's stored `video.*`
/// defaults.
pub fn merge_video_defaults(opts: &mut VideoOptions, defaults: &BTreeMap<String, String>) {
    if opts.size.is_none() {
        if let Some(v) = defaults.get("video.size") {
            match parse_size(v) {
                Some(size) => opts.size = Some(size),
                None => warn_unparseable_default("video.size", v),
            }
        }
    }
    if opts.seconds.is_none() {
        if let Some(v) = defaults.get("video.seconds") {
            match v.parse::<u32>() {
                Ok(s) => opts.seconds = Some(s),
                Err(_) => warn_unparseable_default("video.seconds", v),
            }
        }
    }
    if opts.variants.is_none() {
        if let Some(v) = defaults.get("video.variants") {
            match v.parse::<u8>() {
                Ok(n) => opts.variants = Some(n),
                Err(_) => warn_unparseable_default("video.variants", v),
            }
        }
    }
}

/// Fill only the `None` fields of `opts` from a node's stored `chat.*`
/// defaults.
pub fn merge_chat_defaults(opts: &mut ChatOptions, defaults: &BTreeMap<String, String>) {
    if opts.temperature.is_none() {
        if let Some(v) = defaults.get("chat.temperature") {
            match v.parse::<f32>() {
                Ok(t) => opts.temperature = Some(t),
                Err(_) => warn_unparseable_default("chat.temperature", v),
            }
        }
    }
    if opts.max_tokens.is_none() {
        if let Some(v) = defaults.get("chat.max_tokens") {
            match v.parse::<u32>() {
                Ok(m) => opts.max_tokens = Some(m),
                Err(_) => warn_unparseable_default("chat.max_tokens", v),
            }
        }
    }
}

/// Fill only the `None` field of `opts` from a node's stored
/// `embedding.dimensions` default, honoring the legacy un-namespaced
/// `dimensions` key when the namespaced one is absent.
pub fn merge_embed_defaults(opts: &mut EmbedOptions, defaults: &BTreeMap<String, String>) {
    if opts.dimensions.is_none() {
        if let Some(v) = defaults
            .get("embedding.dimensions")
            .or_else(|| defaults.get("dimensions"))
        {
            match v.parse::<u32>() {
                Ok(d) => opts.dimensions = Some(d),
                Err(_) => warn_unparseable_default("embedding.dimensions", v),
            }
        }
    }
}

/// A high-level client that wraps a [`Provider`] for convenient AI interactions.
pub struct Client {
    provider: Box<dyn Provider>,
    /// Per-node default parameter values (namespaced keys, e.g.
    /// `image.quality`), captured when the client is built from config or a
    /// node. `None` for clients built from a raw provider or direct
    /// constructors.
    node_defaults: Option<BTreeMap<String, String>>,
}

impl Client {
    /// Create a client from the default config (uses `defaults.chat` node).
    pub fn from_config() -> Result<Self> {
        let config = Config::load()?;
        let (id, node) = config.default_chat_node()?;
        let provider = create_provider_from_node(id, node)?;
        Ok(Self {
            provider,
            node_defaults: node.node_defaults.clone(),
        })
    }

    /// Create a client using a specific node by ID or alias.
    pub fn with_node(id_or_alias: &str) -> Result<Self> {
        let config = Config::load()?;
        let (id, node) = config.get_node(id_or_alias).ok_or_else(|| {
            ClientError::NodeNotFound(format!(
                "Node '{}' not found in config. Run `ailloy ai config` to add it.",
                id_or_alias
            ))
        })?;
        let provider = create_provider_from_node(id, node)?;
        Ok(Self {
            provider,
            node_defaults: node.node_defaults.clone(),
        })
    }

    /// Create a client for a specific capability (uses the capability's default node).
    pub fn for_capability(cap: &str) -> Result<Self> {
        let config = Config::load()?;
        let (id, node) = config.default_node_for(cap)?;
        let provider = create_provider_from_node(id, node)?;
        Ok(Self {
            provider,
            node_defaults: node.node_defaults.clone(),
        })
    }

    /// Create a client for a specific task type (uses the task's default node).
    pub fn for_task(task: Task) -> Result<Self> {
        Self::for_capability(task.config_key())
    }

    /// Create a client directly from an [`AiNode`] (no config file needed).
    pub fn from_node(node: &AiNode) -> Result<Self> {
        let provider = create_provider_from_node("inline", node)?;
        Ok(Self {
            provider,
            node_defaults: node.node_defaults.clone(),
        })
    }

    /// Create a client wrapping an existing provider.
    pub fn from_provider(provider: Box<dyn Provider>) -> Self {
        Self {
            provider,
            node_defaults: None,
        }
    }

    /// Create a client builder.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Create a client for OpenAI programmatically (no config needed).
    pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let client = crate::openai::OpenAiClient::new(api_key, model, None);
        Ok(Self {
            provider: Box::new(client),
            node_defaults: None,
        })
    }

    /// Create a client for Anthropic programmatically (no config needed).
    pub fn anthropic(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let client = crate::anthropic::AnthropicClient::new(api_key, model);
        Ok(Self {
            provider: Box::new(client),
            node_defaults: None,
        })
    }

    /// Create a client for Ollama programmatically (no config needed).
    pub fn ollama(model: impl Into<String>, endpoint: Option<String>) -> Result<Self> {
        let client = crate::ollama::OllamaClient::new(model, endpoint);
        Ok(Self {
            provider: Box::new(client),
            node_defaults: None,
        })
    }

    /// Create a client for Azure OpenAI programmatically (no config needed).
    ///
    /// Uses the unified `/openai/v1/` surface; pass `Some(api_version)` to pin
    /// the legacy dated endpoints instead.
    pub fn azure(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        api_version: Option<String>,
    ) -> Result<Self> {
        let auth = crate::azure::AzureAuth::AzureCli;
        let client = match api_version {
            Some(version) => crate::azure::AzureOpenAiClient::with_api_version(
                endpoint, deployment, version, auth,
            ),
            None => crate::azure::AzureOpenAiClient::new(endpoint, deployment, auth),
        };
        Ok(Self {
            provider: Box::new(client),
            node_defaults: None,
        })
    }

    /// Create a client for Microsoft Foundry programmatically (no config needed).
    ///
    /// Uses the unified `/openai/v1/` surface; pass `Some(api_version)` to pin
    /// the legacy dated `/models/...` endpoints instead.
    pub fn foundry(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_version: Option<String>,
    ) -> Result<Self> {
        let auth = crate::azure::AzureAuth::AzureCli;
        let client = match api_version {
            Some(version) => {
                crate::foundry::FoundryClient::with_api_version(endpoint, model, version, auth)
            }
            None => crate::foundry::FoundryClient::new(endpoint, model, auth),
        };
        Ok(Self {
            provider: Box::new(client),
            node_defaults: None,
        })
    }

    /// Create a client for Google Vertex AI programmatically (no config needed).
    pub fn vertex(
        project: impl Into<String>,
        location: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self> {
        let client = crate::vertex::VertexAiClient::new(project, location, model);
        Ok(Self {
            provider: Box::new(client),
            node_defaults: None,
        })
    }

    /// The node's per-node defaults, but only when they would actually change
    /// a request. Returns `None` when there are no defaults (either the client
    /// wasn't built from a node, or the node carried an empty map), so callers
    /// can take a byte-for-byte identical fast path.
    fn active_defaults(&self) -> Option<&BTreeMap<String, String>> {
        self.node_defaults.as_ref().filter(|d| !d.is_empty())
    }

    /// Send a simple chat request (no options).
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = ChatOptions::default();
                merge_chat_defaults(&mut opts, defaults);
                self.provider.chat(messages, Some(&opts)).await
            }
            None => self.provider.chat(messages, None).await,
        }
    }

    /// Send a chat request with options.
    pub async fn chat_with(
        &self,
        messages: &[Message],
        options: &ChatOptions,
    ) -> Result<ChatResponse> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = options.clone();
                merge_chat_defaults(&mut opts, defaults);
                self.provider.chat(messages, Some(&opts)).await
            }
            None => self.provider.chat(messages, Some(options)).await,
        }
    }

    /// Send a streaming chat request.
    pub async fn chat_stream(&self, messages: &[Message]) -> Result<ChatStream> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = ChatOptions::default();
                merge_chat_defaults(&mut opts, defaults);
                self.provider.chat_stream(messages, Some(&opts)).await
            }
            None => self.provider.chat_stream(messages, None).await,
        }
    }

    /// Generate an image from a text prompt.
    pub async fn generate_image(&self, prompt: &str) -> Result<ImageResponse> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = ImageOptions::default();
                merge_image_defaults(&mut opts, defaults);
                self.provider.generate_image(prompt, Some(&opts)).await
            }
            None => self.provider.generate_image(prompt, None).await,
        }
    }

    /// Generate an image with options.
    #[deprecated(
        since = "2.0.0",
        note = "use generate_images_with; image models can return multiple variants"
    )]
    pub async fn generate_image_with(
        &self,
        prompt: &str,
        options: &ImageOptions,
    ) -> Result<ImageResponse> {
        let images = self.generate_images_with(prompt, options).await?;
        images
            .into_iter()
            .next()
            .context("Provider returned no images")
    }

    /// Generate one or more images from a text prompt.
    pub async fn generate_images(&self, prompt: &str) -> Result<Vec<ImageResponse>> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = ImageOptions::default();
                merge_image_defaults(&mut opts, defaults);
                self.provider.generate_images(prompt, Some(&opts)).await
            }
            None => self.provider.generate_images(prompt, None).await,
        }
    }

    /// Generate one or more images with options.
    pub async fn generate_images_with(
        &self,
        prompt: &str,
        options: &ImageOptions,
    ) -> Result<Vec<ImageResponse>> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = options.clone();
                merge_image_defaults(&mut opts, defaults);
                self.provider.generate_images(prompt, Some(&opts)).await
            }
            None => self.provider.generate_images(prompt, Some(options)).await,
        }
    }

    /// Generate embeddings for multiple texts.
    pub async fn embed(&self, texts: &[&str]) -> Result<EmbedResponse> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = EmbedOptions::default();
                merge_embed_defaults(&mut opts, defaults);
                self.provider.embed(texts, Some(&opts)).await
            }
            None => self.provider.embed(texts, None).await,
        }
    }

    /// Generate embeddings with options.
    pub async fn embed_with(
        &self,
        texts: &[&str],
        options: &EmbedOptions,
    ) -> Result<EmbedResponse> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = options.clone();
                merge_embed_defaults(&mut opts, defaults);
                self.provider.embed(texts, Some(&opts)).await
            }
            None => self.provider.embed(texts, Some(options)).await,
        }
    }

    /// Embed a single text, returning the vector directly.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let response = match self.active_defaults() {
            Some(defaults) => {
                let mut opts = EmbedOptions::default();
                merge_embed_defaults(&mut opts, defaults);
                self.provider.embed(&[text], Some(&opts)).await?
            }
            None => self.provider.embed(&[text], None).await?,
        };
        response
            .embeddings
            .into_iter()
            .next()
            .context("No embedding returned")
    }

    /// Generate a video from a text prompt (no options, no progress callback).
    pub async fn generate_video(&self, prompt: &str) -> Result<Vec<VideoResponse>> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = VideoOptions::default();
                merge_video_defaults(&mut opts, defaults);
                self.provider
                    .generate_video(prompt, Some(&opts), None)
                    .await
            }
            None => self.provider.generate_video(prompt, None, None).await,
        }
    }

    /// Generate a video with options.
    pub async fn generate_video_with(
        &self,
        prompt: &str,
        options: &VideoOptions,
    ) -> Result<Vec<VideoResponse>> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = options.clone();
                merge_video_defaults(&mut opts, defaults);
                opts.validate()?;
                self.provider
                    .generate_video(prompt, Some(&opts), None)
                    .await
            }
            None => {
                options.validate()?;
                self.provider
                    .generate_video(prompt, Some(options), None)
                    .await
            }
        }
    }

    /// Generate a video with options, invoking `progress` after each poll of
    /// the job status.
    pub async fn generate_video_with_progress(
        &self,
        prompt: &str,
        options: &VideoOptions,
        progress: impl Fn(&VideoJob) + Send + Sync,
    ) -> Result<Vec<VideoResponse>> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = options.clone();
                merge_video_defaults(&mut opts, defaults);
                opts.validate()?;
                self.provider
                    .generate_video(prompt, Some(&opts), Some(&progress))
                    .await
            }
            None => {
                options.validate()?;
                self.provider
                    .generate_video(prompt, Some(options), Some(&progress))
                    .await
            }
        }
    }

    /// Create an asynchronous video generation job.
    pub async fn create_video_job(
        &self,
        prompt: &str,
        options: Option<&VideoOptions>,
    ) -> Result<VideoJob> {
        match self.active_defaults() {
            Some(defaults) => {
                let mut opts = options.cloned().unwrap_or_default();
                merge_video_defaults(&mut opts, defaults);
                opts.validate()?;
                self.provider.create_video_job(prompt, Some(&opts)).await
            }
            None => {
                if let Some(options) = options {
                    options.validate()?;
                }
                self.provider.create_video_job(prompt, options).await
            }
        }
    }

    /// Fetch the current state of a video generation job.
    pub async fn get_video_job(&self, id: &str) -> Result<VideoJob> {
        self.provider.get_video_job(id).await
    }

    /// Download a completed video generation by generation ID.
    pub async fn download_video(&self, generation_id: &str) -> Result<VideoResponse> {
        self.provider.download_video(generation_id).await
    }

    /// Delete a video generation job and its artifacts.
    pub async fn delete_video_job(&self, id: &str) -> Result<()> {
        self.provider.delete_video_job(id).await
    }

    /// Get the provider name.
    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }
}

/// Builder for constructing a [`Client`] programmatically.
#[derive(Default)]
pub struct ClientBuilder {
    kind: Option<ProviderKind>,
    api_key: Option<String>,
    model: Option<String>,
    endpoint: Option<String>,
    deployment: Option<String>,
    api_version: Option<String>,
    binary: Option<String>,
    project: Option<String>,
    location: Option<String>,
}

impl ClientBuilder {
    pub fn openai(mut self) -> Self {
        self.kind = Some(ProviderKind::OpenAi);
        self
    }

    pub fn anthropic(mut self) -> Self {
        self.kind = Some(ProviderKind::Anthropic);
        self
    }

    pub fn azure(mut self) -> Self {
        self.kind = Some(ProviderKind::AzureOpenAi);
        self
    }

    pub fn foundry(mut self) -> Self {
        self.kind = Some(ProviderKind::MicrosoftFoundry);
        self
    }

    pub fn vertex(mut self) -> Self {
        self.kind = Some(ProviderKind::VertexAi);
        self
    }

    pub fn ollama(mut self) -> Self {
        self.kind = Some(ProviderKind::Ollama);
        self
    }

    pub fn local_agent(mut self) -> Self {
        self.kind = Some(ProviderKind::LocalAgent);
        self
    }

    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn deployment(mut self, deployment: impl Into<String>) -> Self {
        self.deployment = Some(deployment.into());
        self
    }

    pub fn api_version(mut self, api_version: impl Into<String>) -> Self {
        self.api_version = Some(api_version.into());
        self
    }

    pub fn binary(mut self, binary: impl Into<String>) -> Self {
        self.binary = Some(binary.into());
        self
    }

    pub fn project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    pub fn location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    pub fn build(self) -> Result<Client> {
        let kind = self
            .kind
            .context("Provider kind must be set (e.g. .openai(), .anthropic())")?;

        let provider: Box<dyn Provider> = match kind {
            ProviderKind::OpenAi => {
                let api_key = self
                    .api_key
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                    .context("API key required for OpenAI")?;
                let model = self.model.unwrap_or_else(|| "gpt-5.4-mini".to_string());
                Box::new(crate::openai::OpenAiClient::new(
                    api_key,
                    model,
                    self.endpoint,
                ))
            }
            ProviderKind::Anthropic => {
                let api_key = self
                    .api_key
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                    .context("API key required for Anthropic")?;
                let model = self.model.unwrap_or_else(|| "claude-sonnet-5".to_string());
                Box::new(crate::anthropic::AnthropicClient::new(api_key, model))
            }
            ProviderKind::AzureOpenAi => {
                let endpoint = self
                    .endpoint
                    .context("Endpoint required for Azure OpenAI")?;
                let deployment = self
                    .deployment
                    .context("Deployment required for Azure OpenAI")?;
                let auth = if let Some(key) = self.api_key {
                    crate::azure::AzureAuth::ApiKey(key)
                } else {
                    crate::azure::AzureAuth::AzureCli
                };
                Box::new(match self.api_version {
                    Some(version) => crate::azure::AzureOpenAiClient::with_api_version(
                        endpoint, deployment, version, auth,
                    ),
                    None => crate::azure::AzureOpenAiClient::new(endpoint, deployment, auth),
                })
            }
            ProviderKind::MicrosoftFoundry => {
                let endpoint = self
                    .endpoint
                    .context("Endpoint required for Microsoft Foundry")?;
                let model = self.model.context("Model required for Microsoft Foundry")?;
                let auth = if let Some(key) = self.api_key {
                    crate::azure::AzureAuth::ApiKey(key)
                } else {
                    crate::azure::AzureAuth::AzureCli
                };
                Box::new(match self.api_version {
                    Some(version) => crate::foundry::FoundryClient::with_api_version(
                        endpoint.clone(),
                        model.clone(),
                        version,
                        auth,
                    ),
                    None => crate::foundry::FoundryClient::new(endpoint, model, auth),
                })
            }
            ProviderKind::VertexAi => {
                let project = self.project.context("Project required for Vertex AI")?;
                let location = self.location.unwrap_or_else(|| "us-central1".to_string());
                let model = self.model.unwrap_or_else(|| "gemini-3.1-pro".to_string());
                Box::new(crate::vertex::VertexAiClient::new(project, location, model))
            }
            ProviderKind::Ollama => {
                let model = self.model.unwrap_or_else(|| "llama3.2".to_string());
                Box::new(crate::ollama::OllamaClient::new(model, self.endpoint))
            }
            ProviderKind::LocalAgent => {
                let binary = self.binary.context("Binary required for local agent")?;
                Box::new(crate::local_agent::LocalAgentClient::new(binary))
            }
        };

        // The builder constructs a provider directly from raw parameters, not
        // from an `AiNode`, so there are no per-node defaults to apply.
        Ok(Client {
            provider,
            node_defaults: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Auth resolution
// ---------------------------------------------------------------------------

/// Resolve an `Auth` enum to an API key string, if applicable.
fn resolve_auth_api_key(auth: &Auth, node_id: &str) -> Result<String> {
    match auth {
        Auth::Env(var_name) => std::env::var(var_name).with_context(|| {
            format!(
                "Environment variable '{}' not set for node '{}'. Set it or run `ailloy ai config` to change auth.",
                var_name, node_id
            )
        }),
        Auth::ApiKey(key) => Ok(key.clone()),
        Auth::Keychain(_) => crate::config::keychain_secret(node_id),
        Auth::AzureCli(_) | Auth::GcloudCli(_) => {
            anyhow::bail!(
                "Node '{}' uses CLI-based auth, not an API key",
                node_id
            )
        }
    }
}

/// Resolve the Azure auth method from an `AiNode`.
fn resolve_azure_auth(node: &AiNode, node_id: &str) -> Result<crate::azure::AzureAuth> {
    match &node.auth {
        Some(Auth::ApiKey(key)) => Ok(crate::azure::AzureAuth::ApiKey(key.clone())),
        Some(Auth::Keychain(_)) => Ok(crate::azure::AzureAuth::ApiKey(
            crate::config::keychain_secret(node_id)?,
        )),
        Some(Auth::Env(var_name)) => {
            let key = std::env::var(var_name).with_context(|| {
                format!(
                    "Environment variable '{}' not set for node '{}'",
                    var_name, node_id
                )
            })?;
            Ok(crate::azure::AzureAuth::ApiKey(key))
        }
        Some(Auth::AzureCli(_)) | None => Ok(crate::azure::AzureAuth::AzureCli),
        Some(Auth::GcloudCli(_)) => {
            anyhow::bail!(
                "Node '{}' uses gcloud auth but is an Azure provider",
                node_id
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Provider creation from AiNode
// ---------------------------------------------------------------------------

/// Create a `Box<dyn Provider>` from an AI node.
pub fn create_provider_from_node(node_id: &str, node: &AiNode) -> Result<Box<dyn Provider>> {
    match node.provider {
        ProviderKind::OpenAi => {
            let api_key = match &node.auth {
                Some(auth) => resolve_auth_api_key(auth, node_id)?,
                None => std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            };
            let model = node
                .model
                .clone()
                .unwrap_or_else(|| "gpt-5.4-mini".to_string());
            Ok(Box::new(crate::openai::OpenAiClient::new(
                api_key,
                model,
                node.endpoint.clone(),
            )))
        }
        ProviderKind::Anthropic => {
            let api_key = match &node.auth {
                Some(auth) => resolve_auth_api_key(auth, node_id)?,
                None => std::env::var("ANTHROPIC_API_KEY").with_context(|| {
                    format!(
                        "No auth configured for node '{}'. Set ANTHROPIC_API_KEY or add auth to config.",
                        node_id
                    )
                })?,
            };
            let model = node
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-5".to_string());
            Ok(Box::new(crate::anthropic::AnthropicClient::new(
                api_key, model,
            )))
        }
        ProviderKind::AzureOpenAi => {
            let endpoint = node
                .endpoint
                .clone()
                .with_context(|| format!("No endpoint for Azure node '{}'", node_id))?;
            let deployment = node
                .deployment
                .clone()
                .with_context(|| format!("No deployment for Azure node '{}'", node_id))?;
            let auth = resolve_azure_auth(node, node_id)?;
            Ok(Box::new(match node.api_version.clone() {
                Some(version) => crate::azure::AzureOpenAiClient::with_api_version(
                    endpoint, deployment, version, auth,
                ),
                None => crate::azure::AzureOpenAiClient::new(endpoint, deployment, auth),
            }))
        }
        ProviderKind::MicrosoftFoundry => {
            let endpoint = node
                .endpoint
                .clone()
                .with_context(|| format!("No endpoint for Foundry node '{}'", node_id))?;
            let model = node
                .model
                .clone()
                .with_context(|| format!("No model for Foundry node '{}'", node_id))?;
            let auth = resolve_azure_auth(node, node_id)?;
            Ok(Box::new(match node.api_version.clone() {
                Some(version) => crate::foundry::FoundryClient::with_api_version(
                    endpoint.clone(),
                    model.clone(),
                    version,
                    auth,
                ),
                None => crate::foundry::FoundryClient::new(endpoint, model, auth),
            }))
        }
        ProviderKind::VertexAi => {
            let project = node
                .project
                .clone()
                .with_context(|| format!("No project for Vertex AI node '{}'", node_id))?;
            let location = node
                .location
                .clone()
                .unwrap_or_else(|| "us-central1".to_string());
            let model = node
                .model
                .clone()
                .unwrap_or_else(|| "gemini-3.1-pro".to_string());
            Ok(Box::new(crate::vertex::VertexAiClient::new(
                project, location, model,
            )))
        }
        ProviderKind::Ollama => {
            let model = node.model.clone().unwrap_or_else(|| "llama3.2".to_string());
            Ok(Box::new(crate::ollama::OllamaClient::new(
                model,
                node.endpoint.clone(),
            )))
        }
        ProviderKind::LocalAgent => {
            let binary = node.binary.clone().with_context(|| {
                format!("No binary specified for local-agent node '{}'", node_id)
            })?;
            Ok(Box::new(crate::local_agent::LocalAgentClient::new(binary)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_builder_missing_kind() {
        let result = Client::builder().api_key("test").build();
        assert!(result.is_err());
    }

    #[test]
    fn test_client_builder_openai_missing_key() {
        // Remove env var to ensure it's not set
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        let result = Client::builder().openai().build();
        assert!(result.is_err());
    }

    #[test]
    fn test_client_builder_ollama() {
        let client = Client::builder()
            .ollama()
            .model("llama3.2")
            .build()
            .unwrap();
        assert_eq!(client.provider_name(), "ollama");
    }

    #[test]
    fn test_client_builder_local_agent() {
        let client = Client::builder()
            .local_agent()
            .binary("claude")
            .build()
            .unwrap();
        assert_eq!(client.provider_name(), "claude");
    }

    #[test]
    fn test_client_from_provider() {
        let provider = crate::ollama::OllamaClient::new("test-model", None);
        let client = Client::from_provider(Box::new(provider));
        assert_eq!(client.provider_name(), "ollama");
    }

    #[test]
    fn test_create_provider_from_node_ollama() {
        let node = AiNode {
            provider: ProviderKind::Ollama,
            alias: None,
            capabilities: vec![],
            auth: None,
            model: Some("llama3.2".to_string()),
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        };
        let provider = create_provider_from_node("ollama/llama3.2", &node).unwrap();
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_create_provider_from_node_local_agent() {
        let node = AiNode {
            provider: ProviderKind::LocalAgent,
            alias: None,
            capabilities: vec![],
            auth: None,
            model: None,
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: Some("claude".to_string()),
            project: None,
            location: None,
            node_defaults: None,
        };
        let provider = create_provider_from_node("local-agent/claude", &node).unwrap();
        assert_eq!(provider.name(), "claude");
    }

    #[test]
    fn test_create_provider_from_node_missing_binary() {
        let node = AiNode {
            provider: ProviderKind::LocalAgent,
            alias: None,
            capabilities: vec![],
            auth: None,
            model: None,
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        };
        assert!(create_provider_from_node("local-agent/missing", &node).is_err());
    }

    #[test]
    fn test_create_provider_openai_no_auth() {
        // LM Studio and similar local servers don't require auth
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        let node = AiNode {
            provider: ProviderKind::OpenAi,
            alias: None,
            capabilities: vec![],
            auth: None,
            model: Some("local-model".to_string()),
            endpoint: Some("http://localhost:1234".to_string()),
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        };
        let provider = create_provider_from_node("lm-studio/local-model", &node).unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[tokio::test]
    async fn test_unsupported_embed() {
        let client = Client::builder()
            .local_agent()
            .binary("echo")
            .build()
            .unwrap();
        let result = client.embed(&["hello"]).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("embedding"),
            "Error should mention embedding: {}",
            err
        );
    }

    #[test]
    fn test_client_from_node() {
        let node = AiNode {
            provider: ProviderKind::Ollama,
            alias: None,
            capabilities: vec![],
            auth: None,
            model: Some("llama3.2".to_string()),
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        };
        let client = Client::from_node(&node).unwrap();
        assert_eq!(client.provider_name(), "ollama");
    }

    /// Mock provider implementing only `generate_images`, to exercise the
    /// default `generate_image` method on the `Provider` trait.
    struct MockImagesProvider {
        images: Vec<crate::types::ImageResponse>,
    }

    #[async_trait]
    impl Provider for MockImagesProvider {
        fn name(&self) -> &str {
            "mock-images"
        }

        async fn generate_images(
            &self,
            _prompt: &str,
            _options: Option<&ImageOptions>,
        ) -> Result<Vec<ImageResponse>> {
            Ok(self.images.clone())
        }
    }

    fn mock_image(tag: u8) -> ImageResponse {
        ImageResponse {
            data: vec![tag],
            width: 1,
            height: 1,
            format: crate::types::ImageFormat::Png,
            revised_prompt: None,
            usage: None,
        }
    }

    #[tokio::test]
    async fn test_generate_image_default_returns_first_of_generate_images() {
        let provider = MockImagesProvider {
            images: vec![mock_image(1), mock_image(2)],
        };
        let image = provider.generate_image("a prompt", None).await.unwrap();
        assert_eq!(image.data, vec![1]);
    }

    #[tokio::test]
    async fn test_generate_image_default_empty_vec_errors() {
        let provider = MockImagesProvider { images: vec![] };
        let result = provider.generate_image("a prompt", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no images"),
            "Error should mention no images: {}",
            err
        );
    }

    // -----------------------------------------------------------------
    // Video generation
    // -----------------------------------------------------------------

    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock provider with a scripted sequence of job statuses, to exercise
    /// [`poll_video_job_until_terminal`] and the default `generate_video`.
    struct MockVideoProvider {
        statuses: Mutex<VecDeque<VideoJobStatus>>,
        failure_reason: Option<String>,
        poll_count: AtomicUsize,
    }

    impl MockVideoProvider {
        fn new(statuses: Vec<VideoJobStatus>) -> Self {
            Self {
                statuses: Mutex::new(statuses.into()),
                failure_reason: None,
                poll_count: AtomicUsize::new(0),
            }
        }

        fn with_failure(statuses: Vec<VideoJobStatus>, reason: impl Into<String>) -> Self {
            Self {
                statuses: Mutex::new(statuses.into()),
                failure_reason: Some(reason.into()),
                poll_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Provider for MockVideoProvider {
        fn name(&self) -> &str {
            "mock-video"
        }

        async fn create_video_job(
            &self,
            _prompt: &str,
            _options: Option<&VideoOptions>,
        ) -> Result<VideoJob> {
            // Real providers return the job in a non-terminal state right
            // after creation; the scripted `statuses` queue is reserved for
            // `get_video_job` polls so tests can precisely control how many
            // polls occur before a terminal status appears.
            Ok(VideoJob {
                id: "job-1".to_string(),
                status: VideoJobStatus::Queued,
                generation_ids: vec![],
                failure_reason: None,
            })
        }

        async fn get_video_job(&self, id: &str) -> Result<VideoJob> {
            self.poll_count.fetch_add(1, Ordering::SeqCst);
            let status = {
                let mut statuses = self.statuses.lock().unwrap();
                statuses.pop_front().unwrap_or(VideoJobStatus::Running)
            };
            let (generation_ids, failure_reason) = match status {
                VideoJobStatus::Succeeded => (vec!["gen-1".to_string(), "gen-2".to_string()], None),
                VideoJobStatus::Failed | VideoJobStatus::Cancelled => {
                    (vec![], self.failure_reason.clone())
                }
                _ => (vec![], None),
            };
            Ok(VideoJob {
                id: id.to_string(),
                status,
                generation_ids,
                failure_reason,
            })
        }

        async fn download_video(&self, generation_id: &str) -> Result<VideoResponse> {
            Ok(VideoResponse {
                data: generation_id.as_bytes().to_vec(),
                width: 1280,
                height: 720,
                duration_seconds: 4,
            })
        }
    }

    #[tokio::test]
    async fn test_poll_video_job_until_terminal_succeeds() {
        let provider = MockVideoProvider::new(vec![
            VideoJobStatus::Queued,
            VideoJobStatus::Running,
            VideoJobStatus::Succeeded,
        ]);
        let job = poll_video_job_until_terminal(&provider, "job-1", 1, 5, 5_000, None)
            .await
            .unwrap();
        assert_eq!(job.status, VideoJobStatus::Succeeded);
        assert_eq!(job.generation_ids, vec!["gen-1", "gen-2"]);
        assert!(provider.poll_count.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn test_poll_video_job_until_terminal_immediate_failure() {
        let provider = MockVideoProvider::with_failure(
            vec![VideoJobStatus::Failed],
            "content policy violation",
        );
        let job = poll_video_job_until_terminal(&provider, "job-1", 1, 5, 5_000, None)
            .await
            .unwrap();
        assert_eq!(job.status, VideoJobStatus::Failed);
        assert_eq!(
            job.failure_reason.as_deref(),
            Some("content policy violation")
        );
    }

    #[tokio::test]
    async fn test_poll_video_job_until_terminal_timeout() {
        // Never returns a terminal status.
        let provider = MockVideoProvider::new(vec![]);
        let result = poll_video_job_until_terminal(&provider, "job-1", 1, 2, 5, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job-1") && err.contains("expire after 24 hours"),
            "Error should mention job id and expiry: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_poll_video_job_until_terminal_progress_callback_invoked() {
        let provider = MockVideoProvider::new(vec![
            VideoJobStatus::Queued,
            VideoJobStatus::Running,
            VideoJobStatus::Succeeded,
        ]);
        let calls = AtomicUsize::new(0);
        let cb = |_job: &VideoJob| {
            calls.fetch_add(1, Ordering::SeqCst);
        };
        let progress: VideoProgress = &cb;
        let job = poll_video_job_until_terminal(&provider, "job-1", 1, 5, 5_000, Some(progress))
            .await
            .unwrap();
        assert_eq!(job.status, VideoJobStatus::Succeeded);
        assert!(calls.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn test_generate_video_default_downloads_all_generations() {
        // Terminal on the very first poll so the default `generate_video`
        // (which uses real-world backoff timings) doesn't need to sleep.
        let provider = MockVideoProvider::new(vec![VideoJobStatus::Succeeded]);
        let videos = provider.generate_video("a cat", None, None).await.unwrap();
        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].data, b"gen-1");
        assert_eq!(videos[1].data, b"gen-2");
    }

    #[tokio::test]
    async fn test_generate_video_default_failed_job_errors_with_id_and_reason() {
        let provider = MockVideoProvider::with_failure(
            vec![VideoJobStatus::Failed],
            "content policy violation",
        );
        let result = provider.generate_video("a cat", None, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job-1") && err.contains("content policy violation"),
            "Error should mention job id and failure reason: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_generate_video_default_succeeded_no_generations_errors() {
        // A provider that reports Succeeded but returns no generation IDs.
        struct EmptySuccessProvider;

        #[async_trait]
        impl Provider for EmptySuccessProvider {
            fn name(&self) -> &str {
                "empty-success"
            }

            async fn create_video_job(
                &self,
                _prompt: &str,
                _options: Option<&VideoOptions>,
            ) -> Result<VideoJob> {
                Ok(VideoJob {
                    id: "job-empty".to_string(),
                    status: VideoJobStatus::Succeeded,
                    generation_ids: vec![],
                    failure_reason: None,
                })
            }

            async fn get_video_job(&self, id: &str) -> Result<VideoJob> {
                Ok(VideoJob {
                    id: id.to_string(),
                    status: VideoJobStatus::Succeeded,
                    generation_ids: vec![],
                    failure_reason: None,
                })
            }
        }

        let provider = EmptySuccessProvider;
        let result = provider.generate_video("a cat", None, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("job-empty") && err.contains("no generations"),
            "Error should mention job id and lack of generations: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_generate_video_unsupported_by_default() {
        let client = Client::builder()
            .local_agent()
            .binary("echo")
            .build()
            .unwrap();
        let result = client.generate_video("a cat").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("video generation"),
            "Error should mention video generation: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_client_generate_video_with_validates_options() {
        let client = Client::builder()
            .local_agent()
            .binary("echo")
            .build()
            .unwrap();
        let bad_options = VideoOptions {
            seconds: Some(999),
            ..Default::default()
        };
        let result = client.generate_video_with("a cat", &bad_options).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid video duration"),
            "Error should mention invalid duration: {}",
            err
        );
    }

    // -----------------------------------------------------------------
    // Per-node default parameter resolution
    // -----------------------------------------------------------------

    use crate::types::{EmbedResponse, ImageFormat};

    /// A provider that records the options it last received, so tests can
    /// assert exactly what the merging logic produced.
    #[derive(Default)]
    struct CapturingProvider {
        chat: Mutex<Option<Option<ChatOptions>>>,
        image: Mutex<Option<Option<ImageOptions>>>,
        embed: Mutex<Option<Option<EmbedOptions>>>,
    }

    #[async_trait]
    impl Provider for CapturingProvider {
        fn name(&self) -> &str {
            "capturing"
        }

        async fn chat(
            &self,
            _messages: &[Message],
            options: Option<&ChatOptions>,
        ) -> Result<ChatResponse> {
            *self.chat.lock().unwrap() = Some(options.cloned());
            Ok(ChatResponse {
                content: String::new(),
                model: "mock".to_string(),
                usage: None,
            })
        }

        async fn generate_images(
            &self,
            _prompt: &str,
            options: Option<&ImageOptions>,
        ) -> Result<Vec<ImageResponse>> {
            *self.image.lock().unwrap() = Some(options.cloned());
            Ok(vec![])
        }

        async fn embed(
            &self,
            _texts: &[&str],
            options: Option<&EmbedOptions>,
        ) -> Result<EmbedResponse> {
            *self.embed.lock().unwrap() = Some(options.cloned());
            Ok(EmbedResponse {
                embeddings: vec![vec![0.0]],
                model: "mock".to_string(),
                usage: None,
            })
        }
    }

    /// Build a `Client` around an arbitrary provider with explicit defaults,
    /// mirroring what config/node constructors capture without touching disk.
    fn test_client(
        provider: Box<dyn Provider>,
        defaults: Option<BTreeMap<String, String>>,
    ) -> Client {
        Client {
            provider,
            node_defaults: defaults,
        }
    }

    fn defaults_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn defaults_fill_when_option_absent() {
        let provider = std::sync::Arc::new(CapturingProvider::default());
        let client = test_client(
            Box::new(CapturingProviderRef(provider.clone())),
            Some(defaults_map(&[
                ("image.quality", "high"),
                ("chat.temperature", "0.4"),
                ("embedding.dimensions", "256"),
            ])),
        );

        client.chat(&[Message::user("hi")]).await.unwrap();
        let captured = provider.chat.lock().unwrap().clone().unwrap().unwrap();
        assert_eq!(captured.temperature, Some(0.4));

        client.generate_images("a cat").await.unwrap();
        let img = provider.image.lock().unwrap().clone().unwrap().unwrap();
        assert_eq!(img.quality.as_deref(), Some("high"));

        client.embed(&["x"]).await.unwrap();
        let emb = provider.embed.lock().unwrap().clone().unwrap().unwrap();
        assert_eq!(emb.dimensions, Some(256));
    }

    #[tokio::test]
    async fn explicit_option_wins_over_default() {
        let provider = std::sync::Arc::new(CapturingProvider::default());
        let client = test_client(
            Box::new(CapturingProviderRef(provider.clone())),
            Some(defaults_map(&[("chat.temperature", "0.4")])),
        );

        let opts = ChatOptions {
            temperature: Some(1.5),
            ..Default::default()
        };
        client
            .chat_with(&[Message::user("hi")], &opts)
            .await
            .unwrap();
        let captured = provider.chat.lock().unwrap().clone().unwrap().unwrap();
        assert_eq!(captured.temperature, Some(1.5));
    }

    #[tokio::test]
    async fn unparseable_default_is_skipped() {
        let provider = std::sync::Arc::new(CapturingProvider::default());
        let client = test_client(
            Box::new(CapturingProviderRef(provider.clone())),
            Some(defaults_map(&[
                ("image.format", "gif"),
                ("image.quality", "high"),
            ])),
        );

        client.generate_images("a cat").await.unwrap();
        let img = provider.image.lock().unwrap().clone().unwrap().unwrap();
        // The bad format is skipped, but the good sibling value still applies.
        assert_eq!(img.output_format, None);
        assert_eq!(img.quality.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn legacy_dimensions_alias_applies() {
        let provider = std::sync::Arc::new(CapturingProvider::default());
        let client = test_client(
            Box::new(CapturingProviderRef(provider.clone())),
            Some(defaults_map(&[("dimensions", "512")])),
        );

        client.embed(&["x"]).await.unwrap();
        let emb = provider.embed.lock().unwrap().clone().unwrap().unwrap();
        assert_eq!(emb.dimensions, Some(512));
    }

    #[tokio::test]
    async fn no_defaults_passes_options_through_unchanged() {
        // None defaults: the no-option methods must pass `None` (identity).
        let provider = std::sync::Arc::new(CapturingProvider::default());
        let client = test_client(Box::new(CapturingProviderRef(provider.clone())), None);

        client.chat(&[Message::user("hi")]).await.unwrap();
        assert!(matches!(&*provider.chat.lock().unwrap(), Some(None)));

        client.generate_images("a cat").await.unwrap();
        assert!(matches!(&*provider.image.lock().unwrap(), Some(None)));

        client.embed(&["x"]).await.unwrap();
        assert!(matches!(&*provider.embed.lock().unwrap(), Some(None)));
    }

    #[tokio::test]
    async fn empty_defaults_map_is_identity() {
        // An empty (but Some) map must behave exactly like None.
        let provider = std::sync::Arc::new(CapturingProvider::default());
        let client = test_client(
            Box::new(CapturingProviderRef(provider.clone())),
            Some(BTreeMap::new()),
        );

        client.chat(&[Message::user("hi")]).await.unwrap();
        assert!(matches!(&*provider.chat.lock().unwrap(), Some(None)));
    }

    #[test]
    fn merge_image_defaults_parses_all_fields() {
        let defaults = defaults_map(&[
            ("image.size", "512x768"),
            ("image.quality", "high"),
            ("image.format", "jpeg"),
            ("image.compression", "80"),
            ("image.variants", "3"),
            ("image.background", "transparent"),
        ]);
        let mut opts = ImageOptions::default();
        merge_image_defaults(&mut opts, &defaults);
        assert_eq!(opts.size, Some((512, 768)));
        assert_eq!(opts.quality.as_deref(), Some("high"));
        assert_eq!(opts.output_format, Some(ImageFormat::Jpeg));
        assert_eq!(opts.compression, Some(80));
        assert_eq!(opts.n, Some(3));
        assert_eq!(opts.background, Some(Background::Transparent));
    }

    #[test]
    fn merge_image_defaults_skips_unparseable_size() {
        let defaults = defaults_map(&[("image.size", "not-a-size")]);
        let mut opts = ImageOptions::default();
        merge_image_defaults(&mut opts, &defaults);
        assert_eq!(opts.size, None);
    }

    /// A newtype forwarding to a shared `CapturingProvider` so a test can hold
    /// a handle to inspect captures while the `Client` owns the boxed provider.
    struct CapturingProviderRef(std::sync::Arc<CapturingProvider>);

    #[async_trait]
    impl Provider for CapturingProviderRef {
        fn name(&self) -> &str {
            self.0.name()
        }
        async fn chat(
            &self,
            messages: &[Message],
            options: Option<&ChatOptions>,
        ) -> Result<ChatResponse> {
            self.0.chat(messages, options).await
        }
        async fn generate_images(
            &self,
            prompt: &str,
            options: Option<&ImageOptions>,
        ) -> Result<Vec<ImageResponse>> {
            self.0.generate_images(prompt, options).await
        }
        async fn embed(
            &self,
            texts: &[&str],
            options: Option<&EmbedOptions>,
        ) -> Result<EmbedResponse> {
            self.0.embed(texts, options).await
        }
    }
}
