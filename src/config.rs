//! Configuration types and loading.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The kind of AI provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    AzureOpenAi,
    MicrosoftFoundry,
    VertexAi,
    Ollama,
    LocalAgent,
}

impl std::str::FromStr for ProviderKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" | "open-ai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "azure-openai" | "azure-open-ai" => Ok(Self::AzureOpenAi),
            "microsoft-foundry" => Ok(Self::MicrosoftFoundry),
            "vertex-ai" => Ok(Self::VertexAi),
            "ollama" => Ok(Self::Ollama),
            "local-agent" => Ok(Self::LocalAgent),
            _ => Err(format!(
                "Unknown provider kind '{}'. Valid: openai, anthropic, azure-openai, microsoft-foundry, vertex-ai, ollama, local-agent",
                s
            )),
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAi => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::AzureOpenAi => write!(f, "azure-openai"),
            Self::MicrosoftFoundry => write!(f, "microsoft-foundry"),
            Self::VertexAi => write!(f, "vertex-ai"),
            Self::Ollama => write!(f, "ollama"),
            Self::LocalAgent => write!(f, "local-agent"),
        }
    }
}

/// Ordered list of task keys with human-readable labels.
pub const ALL_TASKS: &[(&str, &str)] = &[
    ("chat", "Chat"),
    ("image", "Image Generation"),
    ("embedding", "Embeddings"),
];

impl ProviderKind {
    /// Returns whether this provider kind supports a given task.
    ///
    /// Platforms like Ollama and Microsoft Foundry can support image generation
    /// and embeddings depending on the deployed model.
    pub fn supports_task(&self, task: &str) -> bool {
        matches!(
            (self, task),
            (_, "chat")
                | (
                    Self::OpenAi
                        | Self::AzureOpenAi
                        | Self::MicrosoftFoundry
                        | Self::VertexAi
                        | Self::Ollama,
                    "image",
                )
                | (
                    Self::OpenAi
                        | Self::AzureOpenAi
                        | Self::MicrosoftFoundry
                        | Self::VertexAi
                        | Self::Ollama,
                    "embedding",
                )
        )
    }
}

/// Configuration for a single AI provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, rename = "defaults", skip_serializing_if = "Option::is_none")]
    pub provider_defaults: Option<BTreeMap<String, String>>,
}

/// Well-known consent keys for external CLI tools.
pub mod consent_keys {
    /// Azure CLI (`az`) — used for Azure OpenAI discovery and authentication.
    pub const AZURE_CLI: &str = "azure-cli";
    /// Google Cloud CLI (`gcloud`) — used for Vertex AI authentication.
    pub const GCLOUD_CLI: &str = "gcloud-cli";
}

/// Top-level ailloy configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Legacy field — auto-migrated to `defaults.chat` on load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,

    /// Task-level defaults: maps task names ("chat", "image", "embedding") to provider names.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, String>,

    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,

    /// User consent for external CLI tools (e.g. "azure-cli" → true).
    /// Security decisions — not overridable by local `.ailloy.yaml`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub consents: BTreeMap<String, bool>,
}

impl Config {
    /// Returns the config directory for ailloy (`~/.config/ailloy`).
    ///
    /// Respects `XDG_CONFIG_HOME` if set, otherwise uses `~/.config/ailloy`.
    pub fn config_dir() -> Result<PathBuf> {
        let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg)
        } else {
            dirs::home_dir()
                .context("Could not determine home directory")?
                .join(".config")
        };
        Ok(base.join("ailloy"))
    }

    /// Returns the path to the config file.
    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.yaml"))
    }

    /// Load config from the default location, returning an empty config if the file doesn't exist.
    /// Also merges local `.ailloy.yaml` if found.
    pub fn load() -> Result<Self> {
        let global = Self::load_global()?;
        let local = Self::load_local()?;
        Ok(Self::merge(global, local))
    }

    /// Load only the global config.
    pub fn load_global() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;
        let mut config: Config = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;
        config.migrate();
        Ok(config)
    }

    /// Load local `.ailloy.yaml` from the current directory or parent directories.
    pub fn load_local() -> Result<Option<Self>> {
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            let path = d.join(".ailloy.yaml");
            if path.exists() {
                let content = std::fs::read_to_string(&path).with_context(|| {
                    format!("Failed to read local config from {}", path.display())
                })?;
                let mut config: Config = serde_yaml::from_str(&content).with_context(|| {
                    format!("Failed to parse local config from {}", path.display())
                })?;
                config.migrate();
                return Ok(Some(config));
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        Ok(None)
    }

    /// Merge global and local configs. Local overrides global.
    fn merge(global: Self, local: Option<Self>) -> Self {
        let Some(local) = local else {
            return global;
        };

        let mut defaults = global.defaults;
        for (k, v) in local.defaults {
            defaults.insert(k, v);
        }

        let mut providers = global.providers;
        for (k, v) in local.providers {
            providers.insert(k, v);
        }

        // Consents are security decisions — always use global, never overridden by local config.
        let consents = global.consents;

        Self {
            default_provider: None,
            defaults,
            providers,
            consents,
        }
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

    /// Auto-migrate v0.1 config: move `default_provider` → `defaults.chat`.
    fn migrate(&mut self) {
        if let Some(dp) = self.default_provider.take() {
            self.defaults.entry("chat".to_string()).or_insert(dp);
        }
    }

    /// Remove a provider by name and clean up any defaults that reference it.
    pub fn remove_provider(&mut self, name: &str) -> bool {
        if self.providers.remove(name).is_some() {
            self.defaults.retain(|_, v| v != name);
            true
        } else {
            false
        }
    }

    /// Get the provider name and config for a given task (e.g. "chat", "image", "embedding").
    pub fn provider_for_task(&self, task: &str) -> Result<(&str, &ProviderConfig)> {
        let name = self.defaults.get(task).with_context(|| {
            format!(
                "No default provider configured for task '{}'. Run `ailloy config` to set one up.",
                task
            )
        })?;
        let config = self.providers.get(name.as_str()).with_context(|| {
            format!(
                "Default provider '{}' for task '{}' not found in config",
                name, task
            )
        })?;
        Ok((name.as_str(), config))
    }

    /// Get the default provider name and config (for chat task).
    /// This is a convenience wrapper around `provider_for_task("chat")`.
    pub fn default_provider_config(&self) -> Result<(&str, &ProviderConfig)> {
        self.provider_for_task("chat")
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
            default_provider: None,
            defaults: BTreeMap::from([("chat".to_string(), "openai".to_string())]),
            providers: BTreeMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    kind: ProviderKind::OpenAi,
                    api_key: Some("sk-test".to_string()),
                    endpoint: None,
                    model: Some("gpt-4o".to_string()),
                    deployment: None,
                    api_version: None,
                    binary: None,
                    task: None,
                    auth: None,
                    project: None,
                    location: None,
                    provider_defaults: None,
                },
            )]),
            consents: BTreeMap::new(),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.defaults.get("chat").unwrap(), "openai");
        assert!(parsed.providers.contains_key("openai"));
        assert_eq!(parsed.providers["openai"].kind, ProviderKind::OpenAi);
    }

    #[test]
    fn test_empty_config() {
        let config = Config::default();
        assert!(config.defaults.is_empty());
        assert!(config.providers.is_empty());
        assert!(config.consents.is_empty());
    }

    #[test]
    fn test_default_provider_config_missing() {
        let config = Config::default();
        assert!(config.default_provider_config().is_err());
    }

    #[test]
    fn test_provider_kind_display() {
        assert_eq!(ProviderKind::OpenAi.to_string(), "openai");
        assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderKind::AzureOpenAi.to_string(), "azure-openai");
        assert_eq!(
            ProviderKind::MicrosoftFoundry.to_string(),
            "microsoft-foundry"
        );
        assert_eq!(ProviderKind::VertexAi.to_string(), "vertex-ai");
        assert_eq!(ProviderKind::Ollama.to_string(), "ollama");
        assert_eq!(ProviderKind::LocalAgent.to_string(), "local-agent");
    }

    #[test]
    fn test_provider_kind_serde() {
        let yaml = "kind: open-ai\nmodel: gpt-4o\n";
        let parsed: ProviderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.kind, ProviderKind::OpenAi);
    }

    #[test]
    fn test_migrate_default_provider() {
        let mut config = Config {
            default_provider: Some("openai".to_string()),
            defaults: BTreeMap::new(),
            providers: BTreeMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    kind: ProviderKind::OpenAi,
                    api_key: Some("sk-test".to_string()),
                    endpoint: None,
                    model: Some("gpt-4o".to_string()),
                    deployment: None,
                    api_version: None,
                    binary: None,
                    task: None,
                    auth: None,
                    project: None,
                    location: None,
                    provider_defaults: None,
                },
            )]),
            consents: BTreeMap::new(),
        };

        config.migrate();
        assert!(config.default_provider.is_none());
        assert_eq!(config.defaults.get("chat").unwrap(), "openai");
    }

    #[test]
    fn test_migrate_preserves_existing_defaults() {
        let mut config = Config {
            default_provider: Some("old-provider".to_string()),
            defaults: BTreeMap::from([("chat".to_string(), "existing".to_string())]),
            providers: BTreeMap::new(),
            consents: BTreeMap::new(),
        };

        config.migrate();
        assert_eq!(config.defaults.get("chat").unwrap(), "existing");
    }

    #[test]
    fn test_provider_for_task() {
        let config = Config {
            default_provider: None,
            defaults: BTreeMap::from([
                ("chat".to_string(), "openai".to_string()),
                ("image".to_string(), "dalle".to_string()),
            ]),
            providers: BTreeMap::from([
                (
                    "openai".to_string(),
                    ProviderConfig {
                        kind: ProviderKind::OpenAi,
                        api_key: Some("sk-test".to_string()),
                        endpoint: None,
                        model: Some("gpt-4o".to_string()),
                        deployment: None,
                        api_version: None,
                        binary: None,
                        task: None,
                        auth: None,
                        project: None,
                        location: None,
                        provider_defaults: None,
                    },
                ),
                (
                    "dalle".to_string(),
                    ProviderConfig {
                        kind: ProviderKind::OpenAi,
                        api_key: Some("sk-test".to_string()),
                        endpoint: None,
                        model: Some("dall-e-3".to_string()),
                        deployment: None,
                        api_version: None,
                        binary: None,
                        task: Some("image-generation".to_string()),
                        auth: None,
                        project: None,
                        location: None,
                        provider_defaults: None,
                    },
                ),
            ]),
            consents: BTreeMap::new(),
        };

        let (name, _) = config.provider_for_task("chat").unwrap();
        assert_eq!(name, "openai");

        let (name, _) = config.provider_for_task("image").unwrap();
        assert_eq!(name, "dalle");

        assert!(config.provider_for_task("embedding").is_err());
    }

    #[test]
    fn test_provider_defaults_serde() {
        let yaml = r#"
kind: open-ai
model: dall-e-3
task: image-generation
defaults:
  size: 1024x1024
  quality: hd
"#;
        let parsed: ProviderConfig = serde_yaml::from_str(yaml).unwrap();
        let defaults = parsed.provider_defaults.unwrap();
        assert_eq!(defaults.get("size").unwrap(), "1024x1024");
        assert_eq!(defaults.get("quality").unwrap(), "hd");
    }

    #[test]
    fn test_anthropic_kind_serde() {
        let yaml = "kind: anthropic\nmodel: claude-sonnet-4-6\n";
        let parsed: ProviderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.kind, ProviderKind::Anthropic);
    }

    #[test]
    fn test_vertex_ai_kind_serde() {
        let yaml =
            "kind: vertex-ai\nproject: my-project\nlocation: us-central1\nmodel: gemini-3.1-pro\n";
        let parsed: ProviderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.kind, ProviderKind::VertexAi);
        assert_eq!(parsed.project.unwrap(), "my-project");
        assert_eq!(parsed.location.unwrap(), "us-central1");
    }

    #[test]
    fn test_microsoft_foundry_kind_serde() {
        let yaml = "kind: microsoft-foundry\nendpoint: https://test.services.ai.azure.com\nmodel: gpt-4o\n";
        let parsed: ProviderConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.kind, ProviderKind::MicrosoftFoundry);
        assert_eq!(parsed.model.unwrap(), "gpt-4o");
    }

    #[test]
    fn test_consents_roundtrip() {
        let config = Config {
            default_provider: None,
            defaults: BTreeMap::from([("chat".to_string(), "azure".to_string())]),
            providers: BTreeMap::new(),
            consents: BTreeMap::from([
                ("azure-cli".to_string(), true),
                ("gcloud-cli".to_string(), false),
            ]),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.consents.get("azure-cli"), Some(&true));
        assert_eq!(parsed.consents.get("gcloud-cli"), Some(&false));
    }

    #[test]
    fn test_consents_backward_compat() {
        // YAML without consents field should parse fine with empty default.
        let yaml = "providers: {}\n";
        let parsed: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.consents.is_empty());
    }

    #[test]
    fn test_consents_skip_serializing_when_empty() {
        let config = Config::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(!yaml.contains("consents"));
    }

    #[test]
    fn test_provider_kind_from_str() {
        assert_eq!(
            "openai".parse::<ProviderKind>().unwrap(),
            ProviderKind::OpenAi
        );
        assert_eq!(
            "open-ai".parse::<ProviderKind>().unwrap(),
            ProviderKind::OpenAi
        );
        assert_eq!(
            "anthropic".parse::<ProviderKind>().unwrap(),
            ProviderKind::Anthropic
        );
        assert_eq!(
            "azure-openai".parse::<ProviderKind>().unwrap(),
            ProviderKind::AzureOpenAi
        );
        assert_eq!(
            "azure-open-ai".parse::<ProviderKind>().unwrap(),
            ProviderKind::AzureOpenAi
        );
        assert_eq!(
            "microsoft-foundry".parse::<ProviderKind>().unwrap(),
            ProviderKind::MicrosoftFoundry
        );
        assert_eq!(
            "vertex-ai".parse::<ProviderKind>().unwrap(),
            ProviderKind::VertexAi
        );
        assert_eq!(
            "ollama".parse::<ProviderKind>().unwrap(),
            ProviderKind::Ollama
        );
        assert_eq!(
            "local-agent".parse::<ProviderKind>().unwrap(),
            ProviderKind::LocalAgent
        );
        assert!("invalid".parse::<ProviderKind>().is_err());
    }

    #[test]
    fn test_remove_provider() {
        let mut config = Config {
            default_provider: None,
            defaults: BTreeMap::from([
                ("chat".to_string(), "openai".to_string()),
                ("image".to_string(), "openai".to_string()),
                ("embedding".to_string(), "other".to_string()),
            ]),
            providers: BTreeMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    kind: ProviderKind::OpenAi,
                    api_key: None,
                    endpoint: None,
                    model: None,
                    deployment: None,
                    api_version: None,
                    binary: None,
                    task: None,
                    auth: None,
                    project: None,
                    location: None,
                    provider_defaults: None,
                },
            )]),
            consents: BTreeMap::new(),
        };

        assert!(config.remove_provider("openai"));
        assert!(config.providers.is_empty());
        // Defaults pointing to removed provider should be cleaned up
        assert!(!config.defaults.contains_key("chat"));
        assert!(!config.defaults.contains_key("image"));
        // Defaults pointing to other providers should be preserved
        assert_eq!(config.defaults.get("embedding").unwrap(), "other");

        // Removing nonexistent provider returns false
        assert!(!config.remove_provider("nonexistent"));
    }

    #[test]
    fn test_merge_uses_global_consents_only() {
        let global = Config {
            default_provider: None,
            defaults: BTreeMap::new(),
            providers: BTreeMap::new(),
            consents: BTreeMap::from([("azure-cli".to_string(), true)]),
        };
        let local = Config {
            default_provider: None,
            defaults: BTreeMap::new(),
            providers: BTreeMap::new(),
            consents: BTreeMap::from([("azure-cli".to_string(), false)]),
        };

        let merged = Config::merge(global, Some(local));
        // Global consents should win — local cannot override security decisions.
        assert_eq!(merged.consents.get("azure-cli"), Some(&true));
    }

    #[test]
    fn test_supports_task_chat() {
        // All providers support chat.
        assert!(ProviderKind::OpenAi.supports_task("chat"));
        assert!(ProviderKind::Anthropic.supports_task("chat"));
        assert!(ProviderKind::AzureOpenAi.supports_task("chat"));
        assert!(ProviderKind::MicrosoftFoundry.supports_task("chat"));
        assert!(ProviderKind::VertexAi.supports_task("chat"));
        assert!(ProviderKind::Ollama.supports_task("chat"));
        assert!(ProviderKind::LocalAgent.supports_task("chat"));
    }

    #[test]
    fn test_supports_task_image() {
        assert!(ProviderKind::OpenAi.supports_task("image"));
        assert!(!ProviderKind::Anthropic.supports_task("image"));
        assert!(ProviderKind::AzureOpenAi.supports_task("image"));
        assert!(ProviderKind::MicrosoftFoundry.supports_task("image"));
        assert!(ProviderKind::VertexAi.supports_task("image"));
        assert!(ProviderKind::Ollama.supports_task("image"));
        assert!(!ProviderKind::LocalAgent.supports_task("image"));
    }

    #[test]
    fn test_supports_task_embedding() {
        assert!(ProviderKind::OpenAi.supports_task("embedding"));
        assert!(!ProviderKind::Anthropic.supports_task("embedding"));
        assert!(ProviderKind::AzureOpenAi.supports_task("embedding"));
        assert!(ProviderKind::MicrosoftFoundry.supports_task("embedding"));
        assert!(ProviderKind::VertexAi.supports_task("embedding"));
        assert!(ProviderKind::Ollama.supports_task("embedding"));
        assert!(!ProviderKind::LocalAgent.supports_task("embedding"));
    }

    #[test]
    fn test_supports_task_unknown() {
        // Unknown task names should return false.
        assert!(!ProviderKind::OpenAi.supports_task("unknown"));
        assert!(!ProviderKind::OpenAi.supports_task(""));
    }
}
