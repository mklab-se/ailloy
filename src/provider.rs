//! Unified AI provider dispatcher.

use anyhow::{Context, Result, bail};

use crate::config::{Config, ProviderKind};
use crate::local_agent::LocalAgentClient;
use crate::ollama::OllamaClient;
use crate::openai::OpenAiClient;
use crate::types::{ChatResponse, Message};

/// A unified AI provider that dispatches to the appropriate backend.
pub enum AiProvider {
    OpenAi(OpenAiClient),
    Ollama(OllamaClient),
    LocalAgent(LocalAgentClient),
}

impl AiProvider {
    /// Send a chat message and get a response.
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        match self {
            Self::OpenAi(c) => c.chat(messages).await,
            Self::Ollama(c) => c.chat(messages).await,
            Self::LocalAgent(c) => c.chat(messages).await,
        }
    }

    /// Get the model or binary name for this provider.
    pub fn model(&self) -> &str {
        match self {
            Self::OpenAi(c) => c.model(),
            Self::Ollama(c) => c.model(),
            Self::LocalAgent(c) => c.binary(),
        }
    }
}

/// Create an AI provider from the default provider in the config.
pub fn create_provider(config: &Config) -> Result<AiProvider> {
    let (name, _) = config.default_provider_config()?;
    let name = name.to_string();
    create_provider_by_name(&name, config)
}

/// Create an AI provider by name from the config.
pub fn create_provider_by_name(name: &str, config: &Config) -> Result<AiProvider> {
    let provider_config = config.provider_config(name)?;

    match provider_config.kind {
        ProviderKind::OpenAi => {
            let api_key = provider_config
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .with_context(|| {
                    format!(
                        "No API key for provider '{}'. Set it in config or via OPENAI_API_KEY env var.",
                        name
                    )
                })?;
            let model = provider_config
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4o".to_string());
            let client = OpenAiClient::new(api_key, model, provider_config.endpoint.clone());
            Ok(AiProvider::OpenAi(client))
        }
        ProviderKind::Anthropic => {
            bail!(
                "Anthropic provider is not yet fully integrated into the legacy AiProvider. \
                 Use the new Client API instead."
            )
        }
        ProviderKind::AzureOpenAi => {
            bail!(
                "Azure OpenAI provider is not yet fully integrated into the legacy AiProvider. \
                 Use the new Client API instead."
            )
        }
        ProviderKind::VertexAi => {
            bail!(
                "Vertex AI provider is not yet fully integrated into the legacy AiProvider. \
                 Use the new Client API instead."
            )
        }
        ProviderKind::Ollama => {
            let model = provider_config
                .model
                .clone()
                .unwrap_or_else(|| "llama3.2".to_string());
            let client = OllamaClient::new(model, provider_config.endpoint.clone());
            Ok(AiProvider::Ollama(client))
        }
        ProviderKind::LocalAgent => {
            let binary = provider_config
                .binary
                .clone()
                .context("No binary specified for local-agent provider")?;
            let client = LocalAgentClient::new(binary);
            Ok(AiProvider::LocalAgent(client))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::config::ProviderConfig;

    fn make_config(default_chat: Option<&str>, providers: Vec<(&str, ProviderConfig)>) -> Config {
        Config {
            default_provider: None,
            defaults: default_chat
                .map(|d| HashMap::from([("chat".to_string(), d.to_string())]))
                .unwrap_or_default(),
            providers: providers
                .into_iter()
                .map(|(n, c)| (n.to_string(), c))
                .collect(),
        }
    }

    fn openai_config(api_key: Option<&str>, model: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::OpenAi,
            api_key: api_key.map(|s| s.to_string()),
            endpoint: None,
            model: model.map(|s| s.to_string()),
            deployment: None,
            api_version: None,
            binary: None,
            task: None,
            auth: None,
            project: None,
            location: None,
            provider_defaults: None,
        }
    }

    fn ollama_config(model: &str) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: None,
            endpoint: None,
            model: Some(model.to_string()),
            deployment: None,
            api_version: None,
            binary: None,
            task: None,
            auth: None,
            project: None,
            location: None,
            provider_defaults: None,
        }
    }

    fn local_agent_config(binary: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            kind: ProviderKind::LocalAgent,
            api_key: None,
            endpoint: None,
            model: None,
            deployment: None,
            api_version: None,
            binary: binary.map(|s| s.to_string()),
            task: None,
            auth: None,
            project: None,
            location: None,
            provider_defaults: None,
        }
    }

    #[test]
    fn test_create_provider_no_default() {
        let config = Config::default();
        assert!(create_provider(&config).is_err());
    }

    #[test]
    fn test_create_provider_missing_api_key() {
        let config = make_config(Some("test"), vec![("test", openai_config(None, None))]);
        // Without OPENAI_API_KEY env var, this should fail
        // SAFETY: This test does not run in parallel with other tests that depend on this env var.
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        assert!(create_provider(&config).is_err());
    }

    #[test]
    fn test_create_provider_ollama() {
        let config = make_config(Some("local"), vec![("local", ollama_config("llama3.2"))]);
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.model(), "llama3.2");
    }

    #[test]
    fn test_create_provider_local_agent_missing_binary() {
        let config = make_config(Some("agent"), vec![("agent", local_agent_config(None))]);
        assert!(create_provider(&config).is_err());
    }
}
