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
        ProviderKind::AzureOpenAi => {
            bail!(
                "Azure OpenAI provider is not yet implemented. \
                 Use 'openai' kind with an Azure-compatible endpoint for now."
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

    #[test]
    fn test_create_provider_no_default() {
        let config = Config::default();
        assert!(create_provider(&config).is_err());
    }

    #[test]
    fn test_create_provider_missing_api_key() {
        let config = Config {
            default_provider: Some("test".to_string()),
            providers: HashMap::from([(
                "test".to_string(),
                ProviderConfig {
                    kind: ProviderKind::OpenAi,
                    api_key: None,
                    endpoint: None,
                    model: None,
                    deployment: None,
                    api_version: None,
                    binary: None,
                },
            )]),
        };
        // Without OPENAI_API_KEY env var, this should fail
        // SAFETY: This test does not run in parallel with other tests that depend on this env var.
        unsafe { std::env::remove_var("OPENAI_API_KEY") };
        assert!(create_provider(&config).is_err());
    }

    #[test]
    fn test_create_provider_ollama() {
        let config = Config {
            default_provider: Some("local".to_string()),
            providers: HashMap::from([(
                "local".to_string(),
                ProviderConfig {
                    kind: ProviderKind::Ollama,
                    api_key: None,
                    endpoint: None,
                    model: Some("llama3.2".to_string()),
                    deployment: None,
                    api_version: None,
                    binary: None,
                },
            )]),
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.model(), "llama3.2");
    }

    #[test]
    fn test_create_provider_local_agent_missing_binary() {
        let config = Config {
            default_provider: Some("agent".to_string()),
            providers: HashMap::from([(
                "agent".to_string(),
                ProviderConfig {
                    kind: ProviderKind::LocalAgent,
                    api_key: None,
                    endpoint: None,
                    model: None,
                    deployment: None,
                    api_version: None,
                    binary: None,
                },
            )]),
        };
        assert!(create_provider(&config).is_err());
    }
}
