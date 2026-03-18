//! Configuration types and loading.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ProviderKind
// ---------------------------------------------------------------------------

/// The kind of AI provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProviderKind {
    #[serde(rename = "openai", alias = "open-ai")]
    OpenAi,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "azure-openai", alias = "azure-open-ai")]
    AzureOpenAi,
    #[serde(rename = "microsoft-foundry")]
    MicrosoftFoundry,
    #[serde(rename = "vertex-ai")]
    VertexAi,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "local-agent")]
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

impl ProviderKind {
    /// Returns whether this provider kind supports a given task.
    pub fn supports_task(&self, task: &str) -> bool {
        matches!(
            (self, task),
            (_, "chat") | (Self::OpenAi | Self::AzureOpenAi | Self::VertexAi, "image",)
        )
    }

    /// Returns whether this provider kind supports a given capability.
    pub fn supports_capability(&self, cap: &Capability) -> bool {
        self.supports_task(cap.config_key())
    }

    /// Returns the capabilities this provider kind can potentially support.
    pub fn supported_capabilities(&self) -> Vec<Capability> {
        let mut caps = vec![Capability::Chat];
        if self.supports_task("image") {
            caps.push(Capability::Image);
        }
        caps
    }
}

// ---------------------------------------------------------------------------
// Capability
// ---------------------------------------------------------------------------

/// Capability of an AI node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Chat,
    Image,
}

impl Capability {
    /// Returns the config key for this capability (used in `defaults` map).
    pub fn config_key(&self) -> &str {
        match self {
            Self::Chat => "chat",
            Self::Image => "image",
        }
    }

    /// Returns the human-readable label for this capability.
    pub fn label(&self) -> &str {
        match self {
            Self::Chat => "Chat",
            Self::Image => "Image Generation",
        }
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.config_key())
    }
}

impl std::str::FromStr for Capability {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chat" => Ok(Self::Chat),
            "image" => Ok(Self::Image),
            _ => Err(format!("Unknown capability '{}'. Valid: chat, image", s)),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Authentication strategy for an AI node.
#[derive(Debug, Clone, PartialEq)]
pub enum Auth {
    /// Read API key from an environment variable.
    Env(String),
    /// Inline API key (discouraged — prefer env).
    ApiKey(String),
    /// Authenticate via Azure CLI (`az login`).
    AzureCli(bool),
    /// Authenticate via Google Cloud CLI (`gcloud auth`).
    GcloudCli(bool),
}

/// Helper struct for map-based Auth serialization (`{env: "KEY"}`).
#[derive(Serialize, Deserialize)]
struct AuthHelper {
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azure_cli: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gcloud_cli: Option<bool>,
}

impl Serialize for Auth {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let helper = match self {
            Auth::Env(v) => AuthHelper {
                env: Some(v.clone()),
                api_key: None,
                azure_cli: None,
                gcloud_cli: None,
            },
            Auth::ApiKey(v) => AuthHelper {
                env: None,
                api_key: Some(v.clone()),
                azure_cli: None,
                gcloud_cli: None,
            },
            Auth::AzureCli(v) => AuthHelper {
                env: None,
                api_key: None,
                azure_cli: Some(*v),
                gcloud_cli: None,
            },
            Auth::GcloudCli(v) => AuthHelper {
                env: None,
                api_key: None,
                azure_cli: None,
                gcloud_cli: Some(*v),
            },
        };
        helper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Auth {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let helper = AuthHelper::deserialize(deserializer)?;
        if let Some(v) = helper.env {
            Ok(Auth::Env(v))
        } else if let Some(v) = helper.api_key {
            Ok(Auth::ApiKey(v))
        } else if let Some(v) = helper.azure_cli {
            Ok(Auth::AzureCli(v))
        } else if let Some(v) = helper.gcloud_cli {
            Ok(Auth::GcloudCli(v))
        } else {
            Err(serde::de::Error::custom(
                "auth must have one of: env, api_key, azure_cli, gcloud_cli",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// AiNode
// ---------------------------------------------------------------------------

/// An AI node — the atomic configuration unit for a specific model from a
/// specific provider, with all connection details and capability tags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiNode {
    pub provider: ProviderKind,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub deployment: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    #[serde(default, rename = "defaults", skip_serializing_if = "Option::is_none")]
    pub node_defaults: Option<BTreeMap<String, String>>,
}

impl AiNode {
    /// Human-readable detail string — deployment, model, or binary name.
    pub fn detail(&self) -> &str {
        self.deployment
            .as_deref()
            .or(self.model.as_deref())
            .or(self.binary.as_deref())
            .unwrap_or("?")
    }

    /// Returns whether this node has a given capability.
    pub fn has_capability(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Ordered list of capability keys with human-readable labels.
pub const ALL_CAPABILITIES: &[(&str, &str)] = &[("chat", "Chat"), ("image", "Image Generation")];

/// Ordered list of task keys with human-readable labels (backward-compatible alias).
pub const ALL_TASKS: &[(&str, &str)] = ALL_CAPABILITIES;

/// Well-known consent keys for external CLI tools.
pub mod consent_keys {
    /// Azure CLI (`az`) — used for Azure OpenAI discovery and authentication.
    pub const AZURE_CLI: &str = "azure-cli";
    /// Google Cloud CLI (`gcloud`) — used for Vertex AI authentication.
    pub const GCLOUD_CLI: &str = "gcloud-cli";
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Top-level ailloy configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// AI nodes: maps node IDs (e.g. `openai/gpt-4o`) to their configuration.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, AiNode>,

    /// Capability-level defaults: maps capability names ("chat", "image")
    /// to node IDs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, String>,

    /// User consent for external CLI tools (e.g. "azure-cli" -> true).
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

    /// Load config from the default location, returning an empty config if the
    /// file doesn't exist. Also merges local `.ailloy.yaml` if found.
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
        let config: Config = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", path.display()))?;
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
                let config: Config = serde_yaml::from_str(&content).with_context(|| {
                    format!("Failed to parse local config from {}", path.display())
                })?;
                return Ok(Some(config));
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        Ok(None)
    }

    /// Merge global and local configs. Local overrides nodes/defaults but never consents.
    fn merge(global: Self, local: Option<Self>) -> Self {
        let Some(local) = local else {
            return global;
        };

        let mut defaults = global.defaults;
        for (k, v) in local.defaults {
            defaults.insert(k, v);
        }

        let mut nodes = global.nodes;
        for (k, v) in local.nodes {
            nodes.insert(k, v);
        }

        // Consents are security decisions — always use global, never overridden by local config.
        let consents = global.consents;

        Self {
            nodes,
            defaults,
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

    // --- Node management -------------------------------------------------

    /// Add or replace a node.
    pub fn add_node(&mut self, id: String, node: AiNode) {
        self.nodes.insert(id, node);
    }

    /// Get a node by ID or alias. Returns `(canonical_id, node)`.
    pub fn get_node<'a>(&'a self, id_or_alias: &'a str) -> Option<(&'a str, &'a AiNode)> {
        // Direct ID lookup
        if let Some(node) = self.nodes.get(id_or_alias) {
            return Some((id_or_alias, node));
        }
        // Alias lookup
        for (id, node) in &self.nodes {
            if node.alias.as_deref() == Some(id_or_alias) {
                return Some((id.as_str(), node));
            }
        }
        None
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_node_mut(&mut self, id: &str) -> Option<&mut AiNode> {
        self.nodes.get_mut(id)
    }

    /// Resolve an ID or alias to the canonical node ID.
    pub fn resolve_node<'a>(&'a self, id_or_alias: &'a str) -> Option<&'a str> {
        if self.nodes.contains_key(id_or_alias) {
            return Some(id_or_alias);
        }
        for (id, node) in &self.nodes {
            if node.alias.as_deref() == Some(id_or_alias) {
                return Some(id.as_str());
            }
        }
        None
    }

    /// Remove a node by ID and clean up any defaults that reference it.
    pub fn remove_node(&mut self, id: &str) -> bool {
        if self.nodes.remove(id).is_some() {
            self.defaults.retain(|_, v| v != id);
            true
        } else {
            false
        }
    }

    /// List all nodes that have a given capability.
    pub fn nodes_for_capability(&self, cap: &Capability) -> Vec<(&str, &AiNode)> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.capabilities.contains(cap))
            .map(|(id, n)| (id.as_str(), n))
            .collect()
    }

    // --- Default management ----------------------------------------------

    /// Get the default node for a capability/task (e.g. "chat", "image").
    pub fn default_node_for(&self, cap: &str) -> Result<(&str, &AiNode)> {
        let node_id = self.defaults.get(cap).with_context(|| {
            format!(
                "No default node configured for '{}'. Run `ailloy ai config` to set one up.",
                cap
            )
        })?;
        self.get_node(node_id).with_context(|| {
            format!(
                "Default node '{}' for '{}' not found in config. Run `ailloy ai config` to fix.",
                node_id, cap
            )
        })
    }

    /// Convenience: get the default chat node.
    pub fn default_chat_node(&self) -> Result<(&str, &AiNode)> {
        self.default_node_for("chat")
    }

    /// Set the default node for a capability.
    pub fn set_default(&mut self, cap: &str, node_id: &str) {
        self.defaults.insert(cap.to_string(), node_id.to_string());
    }

    /// Remove the default for a capability.
    pub fn unset_default(&mut self, cap: &str) {
        self.defaults.remove(cap);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_node(provider: ProviderKind, model: &str, caps: Vec<Capability>) -> AiNode {
        AiNode {
            provider,
            alias: None,
            capabilities: caps,
            auth: None,
            model: Some(model.to_string()),
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        }
    }

    #[test]
    fn test_config_roundtrip() {
        let config = Config {
            nodes: BTreeMap::from([(
                "openai/gpt-4o".to_string(),
                AiNode {
                    provider: ProviderKind::OpenAi,
                    alias: None,
                    capabilities: vec![Capability::Chat, Capability::Image],
                    auth: Some(Auth::Env("OPENAI_API_KEY".to_string())),
                    model: Some("gpt-4o".to_string()),
                    endpoint: None,
                    deployment: None,
                    api_version: None,
                    binary: None,
                    project: None,
                    location: None,
                    node_defaults: None,
                },
            )]),
            defaults: BTreeMap::from([("chat".to_string(), "openai/gpt-4o".to_string())]),
            consents: BTreeMap::new(),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.defaults.get("chat").unwrap(), "openai/gpt-4o");
        assert!(parsed.nodes.contains_key("openai/gpt-4o"));
        assert_eq!(parsed.nodes["openai/gpt-4o"].provider, ProviderKind::OpenAi);
    }

    #[test]
    fn test_empty_config() {
        let config = Config::default();
        assert!(config.nodes.is_empty());
        assert!(config.defaults.is_empty());
        assert!(config.consents.is_empty());
    }

    #[test]
    fn test_default_chat_node_missing() {
        let config = Config::default();
        assert!(config.default_chat_node().is_err());
    }

    #[test]
    fn test_node_crud() {
        let mut config = Config::default();

        let node = sample_node(ProviderKind::OpenAi, "gpt-4o", vec![Capability::Chat]);
        config.add_node("openai/gpt-4o".to_string(), node);

        assert!(config.get_node("openai/gpt-4o").is_some());
        assert!(config.get_node("nonexistent").is_none());

        assert!(config.remove_node("openai/gpt-4o"));
        assert!(config.get_node("openai/gpt-4o").is_none());
        assert!(!config.remove_node("nonexistent"));
    }

    #[test]
    fn test_node_alias_resolution() {
        let mut config = Config::default();

        let mut node = sample_node(ProviderKind::OpenAi, "gpt-4o", vec![Capability::Chat]);
        node.alias = Some("gpt".to_string());
        config.add_node("openai/gpt-4o".to_string(), node);

        // Lookup by alias
        let (id, _) = config.get_node("gpt").unwrap();
        assert_eq!(id, "openai/gpt-4o");

        // Resolve by alias
        assert_eq!(config.resolve_node("gpt"), Some("openai/gpt-4o"));

        // Resolve by canonical ID
        assert_eq!(config.resolve_node("openai/gpt-4o"), Some("openai/gpt-4o"));

        // Unknown
        assert_eq!(config.resolve_node("nonexistent"), None);
    }

    #[test]
    fn test_nodes_for_capability() {
        let mut config = Config::default();
        config.add_node(
            "openai/gpt-4o".to_string(),
            sample_node(
                ProviderKind::OpenAi,
                "gpt-4o",
                vec![Capability::Chat, Capability::Image],
            ),
        );
        config.add_node(
            "anthropic/claude".to_string(),
            sample_node(
                ProviderKind::Anthropic,
                "claude-sonnet-4-6",
                vec![Capability::Chat],
            ),
        );

        let chat_nodes = config.nodes_for_capability(&Capability::Chat);
        assert_eq!(chat_nodes.len(), 2);

        let image_nodes = config.nodes_for_capability(&Capability::Image);
        assert_eq!(image_nodes.len(), 1);
        assert_eq!(image_nodes[0].0, "openai/gpt-4o");
    }

    #[test]
    fn test_default_node_management() {
        let mut config = Config::default();
        config.add_node(
            "openai/gpt-4o".to_string(),
            sample_node(ProviderKind::OpenAi, "gpt-4o", vec![Capability::Chat]),
        );

        config.set_default("chat", "openai/gpt-4o");
        let (id, _) = config.default_node_for("chat").unwrap();
        assert_eq!(id, "openai/gpt-4o");

        config.unset_default("chat");
        assert!(config.default_node_for("chat").is_err());
    }

    #[test]
    fn test_remove_node_cleans_defaults() {
        let mut config = Config::default();
        config.add_node(
            "openai/gpt-4o".to_string(),
            sample_node(
                ProviderKind::OpenAi,
                "gpt-4o",
                vec![Capability::Chat, Capability::Image],
            ),
        );
        config.set_default("chat", "openai/gpt-4o");
        config.set_default("image", "openai/gpt-4o");

        assert!(config.remove_node("openai/gpt-4o"));
        assert!(config.nodes.is_empty());
        assert!(!config.defaults.contains_key("chat"));
        assert!(!config.defaults.contains_key("image"));
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
    fn test_provider_kind_serde() {
        let yaml = "provider: openai\nmodel: gpt-4o\ncapabilities: [chat]\n";
        let parsed: AiNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.provider, ProviderKind::OpenAi);
    }

    #[test]
    fn test_provider_kind_serde_alias() {
        // Old kebab-case format should still parse via alias
        let yaml = "provider: open-ai\nmodel: gpt-4o\ncapabilities: [chat]\n";
        let parsed: AiNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.provider, ProviderKind::OpenAi);
    }

    #[test]
    fn test_capability_serde() {
        let yaml = "capabilities: [chat, image]\n";

        #[derive(Deserialize)]
        struct Wrapper {
            capabilities: Vec<Capability>,
        }
        let parsed: Wrapper = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            parsed.capabilities,
            vec![Capability::Chat, Capability::Image]
        );
    }

    #[test]
    fn test_capability_from_str() {
        assert_eq!("chat".parse::<Capability>().unwrap(), Capability::Chat);
        assert_eq!("image".parse::<Capability>().unwrap(), Capability::Image);
        assert!("embedding".parse::<Capability>().is_err());
        assert!("invalid".parse::<Capability>().is_err());
    }

    #[test]
    fn test_auth_serde_env() {
        let yaml = "env: OPENAI_API_KEY\n";
        let parsed: Auth = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed, Auth::Env("OPENAI_API_KEY".to_string()));

        let serialized = serde_yaml::to_string(&parsed).unwrap();
        assert!(serialized.contains("env: OPENAI_API_KEY"));
    }

    #[test]
    fn test_auth_serde_api_key() {
        let yaml = "api_key: sk-test\n";
        let parsed: Auth = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed, Auth::ApiKey("sk-test".to_string()));
    }

    #[test]
    fn test_auth_serde_azure_cli() {
        let yaml = "azure_cli: true\n";
        let parsed: Auth = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed, Auth::AzureCli(true));
    }

    #[test]
    fn test_auth_serde_gcloud_cli() {
        let yaml = "gcloud_cli: true\n";
        let parsed: Auth = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed, Auth::GcloudCli(true));
    }

    #[test]
    fn test_node_detail() {
        let mut node = sample_node(ProviderKind::OpenAi, "gpt-4o", vec![]);
        assert_eq!(node.detail(), "gpt-4o");

        node.deployment = Some("my-deploy".to_string());
        assert_eq!(node.detail(), "my-deploy"); // deployment takes priority

        let agent = AiNode {
            provider: ProviderKind::LocalAgent,
            alias: None,
            capabilities: vec![Capability::Chat],
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
        assert_eq!(agent.detail(), "claude");
    }

    #[test]
    fn test_consents_roundtrip() {
        let config = Config {
            nodes: BTreeMap::new(),
            defaults: BTreeMap::new(),
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
        // YAML without any known fields should parse to empty config.
        let yaml = "something_old: true\n";
        let parsed: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.consents.is_empty());
        assert!(parsed.nodes.is_empty());
    }

    #[test]
    fn test_consents_skip_serializing_when_empty() {
        let config = Config::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(!yaml.contains("consents"));
    }

    #[test]
    fn test_merge_uses_global_consents_only() {
        let global = Config {
            nodes: BTreeMap::new(),
            defaults: BTreeMap::new(),
            consents: BTreeMap::from([("azure-cli".to_string(), true)]),
        };
        let local = Config {
            nodes: BTreeMap::new(),
            defaults: BTreeMap::new(),
            consents: BTreeMap::from([("azure-cli".to_string(), false)]),
        };

        let merged = Config::merge(global, Some(local));
        assert_eq!(merged.consents.get("azure-cli"), Some(&true));
    }

    #[test]
    fn test_merge_overrides_nodes_and_defaults() {
        let global = Config {
            nodes: BTreeMap::from([(
                "openai/gpt-4o".to_string(),
                sample_node(ProviderKind::OpenAi, "gpt-4o", vec![Capability::Chat]),
            )]),
            defaults: BTreeMap::from([("chat".to_string(), "openai/gpt-4o".to_string())]),
            consents: BTreeMap::new(),
        };
        let local = Config {
            nodes: BTreeMap::from([(
                "ollama/llama".to_string(),
                sample_node(ProviderKind::Ollama, "llama3.2", vec![Capability::Chat]),
            )]),
            defaults: BTreeMap::from([("chat".to_string(), "ollama/llama".to_string())]),
            consents: BTreeMap::new(),
        };

        let merged = Config::merge(global, Some(local));
        // Local default overrides global
        assert_eq!(merged.defaults.get("chat").unwrap(), "ollama/llama");
        // Both nodes present
        assert!(merged.nodes.contains_key("openai/gpt-4o"));
        assert!(merged.nodes.contains_key("ollama/llama"));
    }

    #[test]
    fn test_supports_task_chat() {
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
        assert!(!ProviderKind::MicrosoftFoundry.supports_task("image"));
        assert!(ProviderKind::VertexAi.supports_task("image"));
        assert!(!ProviderKind::Ollama.supports_task("image"));
        assert!(!ProviderKind::LocalAgent.supports_task("image"));
    }

    #[test]
    fn test_supports_task_unknown() {
        assert!(!ProviderKind::OpenAi.supports_task("unknown"));
        assert!(!ProviderKind::OpenAi.supports_task(""));
    }

    #[test]
    fn test_full_config_yaml() {
        let yaml = r#"
nodes:
  openai/gpt-4o:
    provider: openai
    model: gpt-4o
    auth:
      env: OPENAI_API_KEY
    capabilities:
    - chat
    - image
  ollama/llama3.2:
    provider: ollama
    model: llama3.2
    endpoint: http://localhost:11434
    capabilities:
    - chat
defaults:
  chat: openai/gpt-4o
  image: openai/gpt-4o
consents:
  azure-cli: true
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.nodes.len(), 2);
        assert_eq!(
            config.nodes["openai/gpt-4o"].auth,
            Some(Auth::Env("OPENAI_API_KEY".to_string()))
        );
        assert_eq!(
            config.nodes["openai/gpt-4o"].capabilities,
            vec![Capability::Chat, Capability::Image]
        );
        assert_eq!(
            config.nodes["ollama/llama3.2"].endpoint,
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(config.defaults.get("chat").unwrap(), "openai/gpt-4o");
        assert_eq!(config.consents.get("azure-cli"), Some(&true));
    }

    #[test]
    fn test_supported_capabilities_openai() {
        let caps = ProviderKind::OpenAi.supported_capabilities();
        assert!(caps.contains(&Capability::Chat));
        assert!(caps.contains(&Capability::Image));
    }

    #[test]
    fn test_supported_capabilities_anthropic() {
        let caps = ProviderKind::Anthropic.supported_capabilities();
        assert!(caps.contains(&Capability::Chat));
        assert!(!caps.contains(&Capability::Image));
    }

    #[test]
    fn test_supported_capabilities_ollama() {
        let caps = ProviderKind::Ollama.supported_capabilities();
        assert!(caps.contains(&Capability::Chat));
        assert!(!caps.contains(&Capability::Image));
    }

    #[test]
    fn test_supported_capabilities_local_agent() {
        let caps = ProviderKind::LocalAgent.supported_capabilities();
        assert!(caps.contains(&Capability::Chat));
        assert!(!caps.contains(&Capability::Image));
    }
}
