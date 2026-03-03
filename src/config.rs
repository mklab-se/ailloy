//! Configuration types and loading.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The kind of AI provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenAi,
    AzureOpenAi,
    Ollama,
    LocalAgent,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAi => write!(f, "openai"),
            Self::AzureOpenAi => write!(f, "azure-openai"),
            Self::Ollama => write!(f, "ollama"),
            Self::LocalAgent => write!(f, "local-agent"),
        }
    }
}

/// Configuration for a single AI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
}

/// Top-level ailloy configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

impl Config {
    /// Returns the platform config directory for ailloy.
    pub fn config_dir() -> Result<PathBuf> {
        let dir = dirs::config_dir()
            .context("Could not determine config directory")?
            .join("ailloy");
        Ok(dir)
    }

    /// Returns the path to the config file.
    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.yaml"))
    }

    /// Load config from the default location, returning an empty config if the file doesn't exist.
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let config: Config = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;
        Ok(config)
    }

    /// Save config to the default location.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let dir = Self::config_dir()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create config directory {}", dir.display()))?;
        let content = serde_yaml::to_string(self)?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;
        Ok(())
    }

    /// Get the default provider name and config.
    pub fn default_provider_config(&self) -> Result<(&str, &ProviderConfig)> {
        let name = self
            .default_provider
            .as_deref()
            .context("No default provider configured. Run `ailloy config init` to set one up.")?;
        let config = self
            .providers
            .get(name)
            .with_context(|| format!("Default provider '{}' not found in config", name))?;
        Ok((name, config))
    }

    /// Get a provider config by name.
    pub fn provider_config(&self, name: &str) -> Result<&ProviderConfig> {
        self.providers
            .get(name)
            .with_context(|| format!("Provider '{}' not found in config", name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_roundtrip() {
        let config = Config {
            default_provider: Some("openai".to_string()),
            providers: HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    kind: ProviderKind::OpenAi,
                    api_key: Some("sk-test".to_string()),
                    endpoint: None,
                    model: Some("gpt-4o".to_string()),
                    deployment: None,
                    api_version: None,
                    binary: None,
                },
            )]),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.default_provider, config.default_provider);
        assert!(parsed.providers.contains_key("openai"));
        assert_eq!(parsed.providers["openai"].kind, ProviderKind::OpenAi);
    }

    #[test]
    fn test_empty_config() {
        let config = Config::default();
        assert!(config.default_provider.is_none());
        assert!(config.providers.is_empty());
    }

    #[test]
    fn test_default_provider_config_missing() {
        let config = Config::default();
        assert!(config.default_provider_config().is_err());
    }

    #[test]
    fn test_provider_kind_display() {
        assert_eq!(ProviderKind::OpenAi.to_string(), "openai");
        assert_eq!(ProviderKind::AzureOpenAi.to_string(), "azure-openai");
        assert_eq!(ProviderKind::Ollama.to_string(), "ollama");
        assert_eq!(ProviderKind::LocalAgent.to_string(), "local-agent");
    }

    #[test]
    fn test_provider_kind_serde() {
        let yaml = "kind: open-ai\nmodel: gpt-4o\n";
        let parsed: ProviderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.kind, ProviderKind::OpenAi);
    }
}
