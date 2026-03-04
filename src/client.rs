//! Client abstraction and Provider trait.

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::config::{Config, ProviderKind};
use crate::error::ClientError;
use crate::types::{
    ChatOptions, ChatResponse, ChatStream, EmbeddingResponse, ImageOptions, ImageResponse, Message,
    Task,
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

    /// Generate an image from a text prompt.
    async fn generate_image(
        &self,
        _prompt: &str,
        _options: Option<&ImageOptions>,
    ) -> Result<ImageResponse> {
        Err(ClientError::Unsupported("image generation".to_string()).into())
    }

    /// Generate an embedding vector from text.
    async fn embed(&self, _input: &str) -> Result<EmbeddingResponse> {
        Err(ClientError::Unsupported("embeddings".to_string()).into())
    }
}

/// A high-level client that wraps a [`Provider`] for convenient AI interactions.
pub struct Client {
    provider: Box<dyn Provider>,
}

impl Client {
    /// Create a client from the default config (uses `defaults.chat` provider).
    pub fn from_config() -> Result<Self> {
        let config = Config::load()?;
        let (name, _) = config.default_provider_config()?;
        let name = name.to_string();
        Self::from_config_provider(&config, &name)
    }

    /// Create a client using a specific named provider from config.
    pub fn with_provider(provider_name: &str) -> Result<Self> {
        let config = Config::load()?;
        Self::from_config_provider(&config, provider_name)
    }

    /// Create a client for a specific task type (uses the task's default provider from config).
    pub fn for_task(task: Task) -> Result<Self> {
        let config = Config::load()?;
        let (name, _) = config.provider_for_task(task.config_key())?;
        let name = name.to_string();
        Self::from_config_provider(&config, &name)
    }

    /// Create a client wrapping an existing provider.
    pub fn from_provider(provider: Box<dyn Provider>) -> Self {
        Self { provider }
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
        })
    }

    /// Create a client for Anthropic programmatically (no config needed).
    pub fn anthropic(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let client = crate::anthropic::AnthropicClient::new(api_key, model);
        Ok(Self {
            provider: Box::new(client),
        })
    }

    /// Create a client for Ollama programmatically (no config needed).
    pub fn ollama(model: impl Into<String>, endpoint: Option<String>) -> Result<Self> {
        let client = crate::ollama::OllamaClient::new(model, endpoint);
        Ok(Self {
            provider: Box::new(client),
        })
    }

    /// Create a client for Azure OpenAI programmatically (no config needed).
    pub fn azure(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        api_version: impl Into<String>,
    ) -> Result<Self> {
        let client = crate::azure::AzureOpenAiClient::new(
            endpoint,
            deployment,
            api_version,
            crate::azure::AzureAuth::AzureCli,
        );
        Ok(Self {
            provider: Box::new(client),
        })
    }

    /// Create a client for Microsoft Foundry programmatically (no config needed).
    pub fn foundry(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_version: impl Into<String>,
    ) -> Result<Self> {
        let client = crate::foundry::FoundryClient::new(
            endpoint,
            model,
            api_version,
            crate::azure::AzureAuth::AzureCli,
        );
        Ok(Self {
            provider: Box::new(client),
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
        })
    }

    // Internal: create a provider from config by name.
    fn from_config_provider(config: &Config, name: &str) -> Result<Self> {
        let provider_config = config.provider_config(name)?;
        let provider = create_provider_from_config(name, provider_config)?;
        Ok(Self { provider })
    }

    /// Send a simple chat request (no options).
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        self.provider.chat(messages, None).await
    }

    /// Send a chat request with options.
    pub async fn chat_with(
        &self,
        messages: &[Message],
        options: &ChatOptions,
    ) -> Result<ChatResponse> {
        self.provider.chat(messages, Some(options)).await
    }

    /// Send a streaming chat request.
    pub async fn chat_stream(&self, messages: &[Message]) -> Result<ChatStream> {
        self.provider.chat_stream(messages, None).await
    }

    /// Generate an image from a text prompt.
    pub async fn generate_image(&self, prompt: &str) -> Result<ImageResponse> {
        self.provider.generate_image(prompt, None).await
    }

    /// Generate an image with options.
    pub async fn generate_image_with(
        &self,
        prompt: &str,
        options: &ImageOptions,
    ) -> Result<ImageResponse> {
        self.provider.generate_image(prompt, Some(options)).await
    }

    /// Generate an embedding vector from text.
    pub async fn embed(&self, input: &str) -> Result<EmbeddingResponse> {
        self.provider.embed(input).await
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
                let model = self.model.unwrap_or_else(|| "gpt-4o".to_string());
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
                let model = self
                    .model
                    .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
                Box::new(crate::anthropic::AnthropicClient::new(api_key, model))
            }
            ProviderKind::AzureOpenAi => {
                let endpoint = self
                    .endpoint
                    .context("Endpoint required for Azure OpenAI")?;
                let deployment = self
                    .deployment
                    .context("Deployment required for Azure OpenAI")?;
                let api_version = self
                    .api_version
                    .unwrap_or_else(|| "2025-04-01-preview".to_string());
                let auth = if let Some(key) = self.api_key {
                    crate::azure::AzureAuth::ApiKey(key)
                } else {
                    crate::azure::AzureAuth::AzureCli
                };
                Box::new(crate::azure::AzureOpenAiClient::new(
                    endpoint,
                    deployment,
                    api_version,
                    auth,
                ))
            }
            ProviderKind::MicrosoftFoundry => {
                let endpoint = self
                    .endpoint
                    .context("Endpoint required for Microsoft Foundry")?;
                let model = self.model.context("Model required for Microsoft Foundry")?;
                let api_version = self
                    .api_version
                    .unwrap_or_else(|| "2024-05-01-preview".to_string());
                let auth = if let Some(key) = self.api_key {
                    crate::azure::AzureAuth::ApiKey(key)
                } else {
                    crate::azure::AzureAuth::AzureCli
                };
                Box::new(crate::foundry::FoundryClient::new(
                    endpoint,
                    model,
                    api_version,
                    auth,
                ))
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

        Ok(Client { provider })
    }
}

/// Create a `Box<dyn Provider>` from a config entry.
pub fn create_provider_from_config(
    name: &str,
    config: &crate::config::ProviderConfig,
) -> Result<Box<dyn Provider>> {
    match config.kind {
        ProviderKind::OpenAi => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .with_context(|| {
                    format!(
                        "No API key for provider '{}'. Set it in config or via OPENAI_API_KEY env var.",
                        name
                    )
                })?;
            let model = config.model.clone().unwrap_or_else(|| "gpt-4o".to_string());
            Ok(Box::new(crate::openai::OpenAiClient::new(
                api_key,
                model,
                config.endpoint.clone(),
            )))
        }
        ProviderKind::Anthropic => {
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .with_context(|| {
                    format!(
                        "No API key for provider '{}'. Set it in config or via ANTHROPIC_API_KEY env var.",
                        name
                    )
                })?;
            let model = config
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
            Ok(Box::new(crate::anthropic::AnthropicClient::new(
                api_key, model,
            )))
        }
        ProviderKind::AzureOpenAi => {
            let endpoint = config
                .endpoint
                .clone()
                .with_context(|| format!("No endpoint for Azure provider '{}'", name))?;
            let deployment = config
                .deployment
                .clone()
                .with_context(|| format!("No deployment for Azure provider '{}'", name))?;
            let api_version = config
                .api_version
                .clone()
                .unwrap_or_else(|| "2025-04-01-preview".to_string());
            let auth = match config.auth.as_deref() {
                Some("azure-cli") | None => {
                    if let Some(key) = config.api_key.clone() {
                        crate::azure::AzureAuth::ApiKey(key)
                    } else {
                        crate::azure::AzureAuth::AzureCli
                    }
                }
                Some(other) => {
                    anyhow::bail!(
                        "Unknown auth method '{}' for Azure provider '{}'",
                        other,
                        name
                    )
                }
            };
            Ok(Box::new(crate::azure::AzureOpenAiClient::new(
                endpoint,
                deployment,
                api_version,
                auth,
            )))
        }
        ProviderKind::MicrosoftFoundry => {
            let endpoint = config.endpoint.clone().with_context(|| {
                format!("No endpoint for Microsoft Foundry provider '{}'", name)
            })?;
            let model = config
                .model
                .clone()
                .with_context(|| format!("No model for Microsoft Foundry provider '{}'", name))?;
            let api_version = config
                .api_version
                .clone()
                .unwrap_or_else(|| "2024-05-01-preview".to_string());
            let auth = match config.auth.as_deref() {
                Some("azure-cli") | None => {
                    if let Some(key) = config.api_key.clone() {
                        crate::azure::AzureAuth::ApiKey(key)
                    } else {
                        crate::azure::AzureAuth::AzureCli
                    }
                }
                Some(other) => {
                    anyhow::bail!(
                        "Unknown auth method '{}' for Microsoft Foundry provider '{}'",
                        other,
                        name
                    )
                }
            };
            Ok(Box::new(crate::foundry::FoundryClient::new(
                endpoint,
                model,
                api_version,
                auth,
            )))
        }
        ProviderKind::VertexAi => {
            let project = config
                .project
                .clone()
                .with_context(|| format!("No project for Vertex AI provider '{}'", name))?;
            let location = config
                .location
                .clone()
                .unwrap_or_else(|| "us-central1".to_string());
            let model = config
                .model
                .clone()
                .unwrap_or_else(|| "gemini-3.1-pro".to_string());
            Ok(Box::new(crate::vertex::VertexAiClient::new(
                project, location, model,
            )))
        }
        ProviderKind::Ollama => {
            let model = config
                .model
                .clone()
                .unwrap_or_else(|| "llama3.2".to_string());
            Ok(Box::new(crate::ollama::OllamaClient::new(
                model,
                config.endpoint.clone(),
            )))
        }
        ProviderKind::LocalAgent => {
            let binary = config.binary.clone().with_context(|| {
                format!("No binary specified for local-agent provider '{}'", name)
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
}
