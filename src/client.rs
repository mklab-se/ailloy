//! Client abstraction and Provider trait.

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::config::{AiNode, Auth, Config, ProviderKind};
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
    /// Create a client from the default config (uses `defaults.chat` node).
    pub fn from_config() -> Result<Self> {
        let config = Config::load()?;
        let (id, node) = config.default_chat_node()?;
        let provider = create_provider_from_node(id, node)?;
        Ok(Self { provider })
    }

    /// Create a client using a specific node by ID or alias.
    pub fn with_node(id_or_alias: &str) -> Result<Self> {
        let config = Config::load()?;
        let (id, node) = config.get_node(id_or_alias).ok_or_else(|| {
            ClientError::NodeNotFound(format!(
                "Node '{}' not found in config. Run `ailloy config` to add it.",
                id_or_alias
            ))
        })?;
        let provider = create_provider_from_node(id, node)?;
        Ok(Self { provider })
    }

    /// Create a client for a specific capability (uses the capability's default node).
    pub fn for_capability(cap: &str) -> Result<Self> {
        let config = Config::load()?;
        let (id, node) = config.default_node_for(cap)?;
        let provider = create_provider_from_node(id, node)?;
        Ok(Self { provider })
    }

    /// Create a client for a specific task type (uses the task's default node).
    pub fn for_task(task: Task) -> Result<Self> {
        Self::for_capability(task.config_key())
    }

    /// Create a client directly from an [`AiNode`] (no config file needed).
    pub fn from_node(node: &AiNode) -> Result<Self> {
        let provider = create_provider_from_node("inline", node)?;
        Ok(Self { provider })
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

// ---------------------------------------------------------------------------
// Auth resolution
// ---------------------------------------------------------------------------

/// Resolve an `Auth` enum to an API key string, if applicable.
fn resolve_auth_api_key(auth: &Auth, node_id: &str) -> Result<String> {
    match auth {
        Auth::Env(var_name) => std::env::var(var_name).with_context(|| {
            format!(
                "Environment variable '{}' not set for node '{}'. Set it or run `ailloy config` to change auth.",
                var_name, node_id
            )
        }),
        Auth::ApiKey(key) => Ok(key.clone()),
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
            let model = node.model.clone().unwrap_or_else(|| "gpt-4o".to_string());
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
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
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
            let api_version = node
                .api_version
                .clone()
                .unwrap_or_else(|| "2025-04-01-preview".to_string());
            let auth = resolve_azure_auth(node, node_id)?;
            Ok(Box::new(crate::azure::AzureOpenAiClient::new(
                endpoint,
                deployment,
                api_version,
                auth,
            )))
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
            let api_version = node
                .api_version
                .clone()
                .unwrap_or_else(|| "2024-05-01-preview".to_string());
            let auth = resolve_azure_auth(node, node_id)?;
            Ok(Box::new(crate::foundry::FoundryClient::new(
                endpoint,
                model,
                api_version,
                auth,
            )))
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
}
