//! Configuration types and loading.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
            (_, "chat")
                | (
                    Self::OpenAi | Self::AzureOpenAi | Self::VertexAi | Self::MicrosoftFoundry,
                    "image"
                )
                | (Self::AzureOpenAi | Self::MicrosoftFoundry, "video")
                | (
                    Self::OpenAi
                        | Self::AzureOpenAi
                        | Self::Ollama
                        | Self::VertexAi
                        | Self::MicrosoftFoundry,
                    "embedding"
                )
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
        if self.supports_task("video") {
            caps.push(Capability::Video);
        }
        if self.supports_task("embedding") {
            caps.push(Capability::Embedding);
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
    Embedding,
    Video,
}

impl Capability {
    /// Returns the config key for this capability (used in `defaults` map).
    pub fn config_key(&self) -> &str {
        match self {
            Self::Chat => "chat",
            Self::Image => "image",
            Self::Embedding => "embedding",
            Self::Video => "video",
        }
    }

    /// Returns the human-readable label for this capability.
    pub fn label(&self) -> &str {
        match self {
            Self::Chat => "Chat",
            Self::Image => "Image Generation",
            Self::Embedding => "Embedding",
            Self::Video => "Video Generation",
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
            "embedding" => Ok(Self::Embedding),
            "video" => Ok(Self::Video),
            _ => Err(format!(
                "Unknown capability '{}'. Valid: chat, image, embedding, video",
                s
            )),
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
    /// Inline API key (discouraged — prefer env or keychain).
    ApiKey(String),
    /// Read the API key from the OS keychain (service `ailloy`,
    /// account = the node id). Store with `ailloy ai config set-key <node>`
    /// or [`Config::set_keychain_secret`]. Requires the `keychain` feature.
    Keychain(bool),
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
    keychain: Option<bool>,
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
                keychain: None,
                azure_cli: None,
                gcloud_cli: None,
            },
            Auth::ApiKey(v) => AuthHelper {
                env: None,
                api_key: Some(v.clone()),
                keychain: None,
                azure_cli: None,
                gcloud_cli: None,
            },
            Auth::Keychain(v) => AuthHelper {
                env: None,
                api_key: None,
                keychain: Some(*v),
                azure_cli: None,
                gcloud_cli: None,
            },
            Auth::AzureCli(v) => AuthHelper {
                env: None,
                api_key: None,
                keychain: None,
                azure_cli: Some(*v),
                gcloud_cli: None,
            },
            Auth::GcloudCli(v) => AuthHelper {
                env: None,
                api_key: None,
                keychain: None,
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
        } else if let Some(v) = helper.keychain {
            Ok(Auth::Keychain(v))
        } else if let Some(v) = helper.azure_cli {
            Ok(Auth::AzureCli(v))
        } else if let Some(v) = helper.gcloud_cli {
            Ok(Auth::GcloudCli(v))
        } else {
            Err(serde::de::Error::custom(
                "auth must have one of: env, api_key, keychain, azure_cli, gcloud_cli",
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
    /// A bare node for the given provider — the starting point for
    /// programmatic configuration (fill in model/auth/capabilities and pass
    /// to [`Config::ensure_node`] / [`Config::upsert_node`]).
    pub fn new(provider: ProviderKind) -> Self {
        AiNode {
            provider,
            alias: None,
            capabilities: Vec::new(),
            auth: None,
            model: None,
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        }
    }

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

    /// Returns embedding metadata for this node.
    ///
    /// Dimensions are resolved in order: explicit `defaults.dimensions` in the
    /// node config, then a lookup of well-known model dimensions by model or
    /// deployment name.
    pub fn embedding_metadata(&self) -> EmbeddingMetadata {
        let explicit_dims = self
            .node_defaults
            .as_ref()
            .and_then(|d| d.get("dimensions"))
            .and_then(|v| v.parse::<u32>().ok());

        let dimensions = explicit_dims.or_else(|| {
            let name = self.model.as_deref().or(self.deployment.as_deref())?;
            well_known_embedding_dimensions(name)
        });

        EmbeddingMetadata {
            provider: self.provider.clone(),
            model: self.model.clone(),
            endpoint: self.endpoint.clone(),
            deployment: self.deployment.clone(),
            dimensions,
            auth: self.auth.clone(),
        }
    }
}

/// Returns the default output dimensions for well-known embedding models.
fn well_known_embedding_dimensions(model: &str) -> Option<u32> {
    // Normalize: lowercase, strip version suffixes and date tags
    let m = model.to_lowercase();
    match m.as_str() {
        // OpenAI / Azure OpenAI
        "text-embedding-3-large" => Some(3072),
        "text-embedding-3-small" => Some(1536),
        "text-embedding-ada-002" => Some(1536),
        // Google Vertex AI
        "text-embedding-004" | "text-embedding-005" => Some(768),
        "textembedding-gecko"
        | "textembedding-gecko@003"
        | "textembedding-gecko@002"
        | "textembedding-gecko@001" => Some(768),
        "text-multilingual-embedding-002" => Some(768),
        // Ollama common models
        "nomic-embed-text" => Some(768),
        "all-minilm" | "all-minilm:latest" => Some(384),
        "mxbai-embed-large" | "mxbai-embed-large:latest" => Some(1024),
        "snowflake-arctic-embed" | "snowflake-arctic-embed:latest" => Some(1024),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// EmbeddingMetadata
// ---------------------------------------------------------------------------

/// Metadata about an embedding node, useful for configuring downstream systems.
#[derive(Debug, Clone)]
pub struct EmbeddingMetadata {
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub deployment: Option<String>,
    pub dimensions: Option<u32>,
    pub auth: Option<Auth>,
}

impl EmbeddingMetadata {
    /// Generate Azure AI Search vectorizer configuration JSON.
    ///
    /// Works with Azure OpenAI and Microsoft Foundry nodes — both are backed
    /// by Azure AI Services resources that expose an OpenAI-compatible endpoint.
    /// For Foundry nodes, the `.services.ai.azure.com` endpoint is converted
    /// to the `.openai.azure.com` variant that Azure AI Search expects.
    pub fn to_azure_search_vectorizer(&self, name: &str) -> anyhow::Result<serde_json::Value> {
        if self.provider != ProviderKind::AzureOpenAi
            && self.provider != ProviderKind::MicrosoftFoundry
        {
            anyhow::bail!(
                "Azure AI Search vectorizers require Azure OpenAI or Microsoft Foundry nodes, \
                 but this node uses '{}'. Configure an Azure-hosted embedding node instead.",
                self.provider
            );
        }
        let endpoint = self
            .endpoint
            .as_deref()
            .context("Embedding node has no endpoint configured.")?;

        // Azure AI Search expects the .openai.azure.com endpoint variant.
        // Foundry nodes use .services.ai.azure.com (or .cognitiveservices.azure.com),
        // which is the same underlying resource — convert to the OpenAI endpoint.
        let resource_uri = endpoint
            .replace(".services.ai.azure.com", ".openai.azure.com")
            .replace(".cognitiveservices.azure.com", ".openai.azure.com");
        let resource_uri = resource_uri.trim_end_matches('/');

        let deployment = self
            .deployment
            .as_deref()
            .or(self.model.as_deref())
            .context("Embedding node has no deployment or model name configured.")?;
        let model_name = self.model.as_deref().unwrap_or(deployment);

        let mut params = serde_json::json!({
            "resourceUri": resource_uri,
            "deploymentId": deployment,
            "modelName": model_name,
        });

        // Include API key if the node uses key-based auth.
        // If using Azure CLI auth, omit the key — Azure AI Search will use
        // its managed identity or system-assigned identity instead.
        match &self.auth {
            Some(Auth::ApiKey(key)) => {
                params["apiKey"] = serde_json::json!(key);
            }
            Some(Auth::Env(var_name)) => {
                if let Ok(key) = std::env::var(var_name) {
                    params["apiKey"] = serde_json::json!(key);
                }
            }
            _ => {} // Azure CLI or no auth — no apiKey in vectorizer config
        }

        Ok(serde_json::json!({
            "name": name,
            "kind": "azureOpenAI",
            "azureOpenAIParameters": params,
        }))
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Ordered list of capability keys with human-readable labels.
pub const ALL_CAPABILITIES: &[(&str, &str)] = &[
    ("chat", "Chat"),
    ("image", "Image Generation"),
    ("embedding", "Embedding"),
    ("video", "Video Generation"),
];

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

/// Where an effective config came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConfigSource {
    /// Machine-wide `~/.config/ailloy/config.yaml` (or empty default).
    #[default]
    Global,
    /// A repository/folder-local `.ailloy.yaml` that fully replaces the
    /// global nodes/defaults.
    Local(PathBuf),
    /// A local `.ailloy.yaml` with `extends: global`, merged over global.
    LocalExtendsGlobal(PathBuf),
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigSource::Global => write!(f, "global"),
            ConfigSource::Local(p) => write!(f, "local: {}", p.display()),
            ConfigSource::LocalExtendsGlobal(p) => {
                write!(f, "local (extends global): {}", p.display())
            }
        }
    }
}

/// Top-level ailloy configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Local-config inheritance: a `.ailloy.yaml` with `extends: global`
    /// merges over the machine-wide config (the pre-1.1 behavior); without
    /// it, the local file fully replaces nodes/defaults (closest file wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,

    /// Which file this config was loaded from (not serialized).
    #[serde(skip)]
    pub source: ConfigSource,

    /// AI nodes: maps node IDs (e.g. `openai/gpt-5.4-mini`) to their configuration.
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

    /// Load the effective config: the **closest** `.ailloy.yaml` (walking up
    /// from the current directory) wins entirely; without one, the
    /// machine-wide config is used. A local file may opt back into merging
    /// with `extends: global`. Consents are always taken from the global
    /// config (security decisions are never overridable per folder).
    pub fn load() -> Result<Self> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load_from_dir(&cwd)
    }

    /// `load()` with an explicit starting directory (testable).
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let local = Self::load_local_from(dir)?;
        match local {
            None => Self::load_global(),
            Some((mut local, path)) => {
                let global = Self::load_global()?;
                if local.extends.as_deref() == Some("global") {
                    let mut merged = Self::merge(global, Some(local));
                    merged.source = ConfigSource::LocalExtendsGlobal(path);
                    Ok(merged)
                } else {
                    // Closest file wins: nodes/defaults entirely local,
                    // consents always global.
                    local.consents = global.consents;
                    local.source = ConfigSource::Local(path);
                    Ok(local)
                }
            }
        }
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

    /// Locate and load the nearest local `.ailloy.yaml`, walking up from `start`.
    pub fn load_local_from(start: &Path) -> Result<Option<(Self, PathBuf)>> {
        let mut dir = Some(start.to_path_buf());
        while let Some(d) = dir {
            let path = d.join(".ailloy.yaml");
            if path.exists() {
                let content = std::fs::read_to_string(&path).with_context(|| {
                    format!("Failed to read local config from {}", path.display())
                })?;
                let config: Config = serde_yaml::from_str(&content).with_context(|| {
                    format!("Failed to parse local config from {}", path.display())
                })?;
                return Ok(Some((config, path)));
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
            extends: None,
            source: ConfigSource::Global,
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
    /// Insert `node` only when no node with `id` exists yet.
    /// Returns true when the node was inserted.
    ///
    /// The embedding pattern for dependent tools:
    /// ```no_run
    /// # use ailloy::config::{AiNode, Auth, Capability, Config, ProviderKind};
    /// let mut config = Config::load_global().unwrap_or_default();
    /// let mut node = AiNode::new(ProviderKind::OpenAi);
    /// node.model = Some("gpt-5.4-mini".into());
    /// node.auth = Some(Auth::Env("OPENAI_API_KEY".into()));
    /// node.capabilities = vec![Capability::Chat];
    /// if config.ensure_node("openai/gpt-5.4-mini".into(), node) {
    ///     config.set_default_for("chat", "openai/gpt-5.4-mini").ok();
    ///     config.save().unwrap();
    /// }
    /// ```
    pub fn ensure_node(&mut self, id: String, node: AiNode) -> bool {
        if self.nodes.contains_key(&id) {
            return false;
        }
        self.nodes.insert(id, node);
        true
    }

    /// Insert or replace the node with `id`.
    pub fn upsert_node(&mut self, id: String, node: AiNode) {
        self.nodes.insert(id, node);
    }

    /// Set the default node for a capability, validating both exist.
    pub fn set_default_for(&mut self, capability: &str, node_id: &str) -> anyhow::Result<()> {
        if !ALL_CAPABILITIES.iter().any(|(k, _)| *k == capability) {
            anyhow::bail!(
                "unknown capability '{capability}' (valid: {})",
                ALL_CAPABILITIES
                    .iter()
                    .map(|(k, _)| *k)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let (canonical, _) = self
            .get_node(node_id)
            .ok_or_else(|| anyhow::anyhow!("unknown node '{node_id}'"))?;
        let canonical = canonical.to_string();
        self.defaults.insert(capability.to_string(), canonical);
        Ok(())
    }

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

/// Read a node's secret from the OS keychain (service `ailloy`).
#[cfg(feature = "keychain")]
pub fn keychain_secret(node_id: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    let entry = keyring::Entry::new("ailloy", node_id)
        .with_context(|| format!("cannot access the OS keychain for node '{node_id}'"))?;
    entry.get_password().with_context(|| {
        format!(
            "no keychain secret for node '{node_id}'. Store one with: ailloy ai config set-key {node_id}"
        )
    })
}

/// Store a node's secret in the OS keychain (service `ailloy`).
#[cfg(feature = "keychain")]
pub fn set_keychain_secret(node_id: &str, secret: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    let entry = keyring::Entry::new("ailloy", node_id)
        .with_context(|| format!("cannot access the OS keychain for node '{node_id}'"))?;
    entry
        .set_password(secret)
        .with_context(|| format!("failed to store the keychain secret for node '{node_id}'"))
}

/// Delete a node's secret from the OS keychain (ignores missing entries).
#[cfg(feature = "keychain")]
pub fn delete_keychain_secret(node_id: &str) -> anyhow::Result<()> {
    if let Ok(entry) = keyring::Entry::new("ailloy", node_id) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

#[cfg(not(feature = "keychain"))]
pub fn keychain_secret(node_id: &str) -> anyhow::Result<String> {
    anyhow::bail!(
        "node '{node_id}' uses keychain auth, but ailloy was built without the `keychain` feature"
    )
}

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
            extends: None,
            source: ConfigSource::Global,
            nodes: BTreeMap::from([(
                "openai/gpt-5.4-mini".to_string(),
                AiNode {
                    provider: ProviderKind::OpenAi,
                    alias: None,
                    capabilities: vec![Capability::Chat, Capability::Image],
                    auth: Some(Auth::Env("OPENAI_API_KEY".to_string())),
                    model: Some("gpt-5.4-mini".to_string()),
                    endpoint: None,
                    deployment: None,
                    api_version: None,
                    binary: None,
                    project: None,
                    location: None,
                    node_defaults: None,
                },
            )]),
            defaults: BTreeMap::from([("chat".to_string(), "openai/gpt-5.4-mini".to_string())]),
            consents: BTreeMap::new(),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.defaults.get("chat").unwrap(), "openai/gpt-5.4-mini");
        assert!(parsed.nodes.contains_key("openai/gpt-5.4-mini"));
        assert_eq!(
            parsed.nodes["openai/gpt-5.4-mini"].provider,
            ProviderKind::OpenAi
        );
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

        let node = sample_node(ProviderKind::OpenAi, "gpt-5.4-mini", vec![Capability::Chat]);
        config.add_node("openai/gpt-5.4-mini".to_string(), node);

        assert!(config.get_node("openai/gpt-5.4-mini").is_some());
        assert!(config.get_node("nonexistent").is_none());

        assert!(config.remove_node("openai/gpt-5.4-mini"));
        assert!(config.get_node("openai/gpt-5.4-mini").is_none());
        assert!(!config.remove_node("nonexistent"));
    }

    #[test]
    fn test_node_alias_resolution() {
        let mut config = Config::default();

        let mut node = sample_node(ProviderKind::OpenAi, "gpt-5.4-mini", vec![Capability::Chat]);
        node.alias = Some("gpt".to_string());
        config.add_node("openai/gpt-5.4-mini".to_string(), node);

        // Lookup by alias
        let (id, _) = config.get_node("gpt").unwrap();
        assert_eq!(id, "openai/gpt-5.4-mini");

        // Resolve by alias
        assert_eq!(config.resolve_node("gpt"), Some("openai/gpt-5.4-mini"));

        // Resolve by canonical ID
        assert_eq!(
            config.resolve_node("openai/gpt-5.4-mini"),
            Some("openai/gpt-5.4-mini")
        );

        // Unknown
        assert_eq!(config.resolve_node("nonexistent"), None);
    }

    #[test]
    fn test_nodes_for_capability() {
        let mut config = Config::default();
        config.add_node(
            "openai/gpt-5.4-mini".to_string(),
            sample_node(
                ProviderKind::OpenAi,
                "gpt-5.4-mini",
                vec![Capability::Chat, Capability::Image],
            ),
        );
        config.add_node(
            "anthropic/claude".to_string(),
            sample_node(
                ProviderKind::Anthropic,
                "claude-sonnet-5",
                vec![Capability::Chat],
            ),
        );

        let chat_nodes = config.nodes_for_capability(&Capability::Chat);
        assert_eq!(chat_nodes.len(), 2);

        let image_nodes = config.nodes_for_capability(&Capability::Image);
        assert_eq!(image_nodes.len(), 1);
        assert_eq!(image_nodes[0].0, "openai/gpt-5.4-mini");
    }

    #[test]
    fn test_default_node_management() {
        let mut config = Config::default();
        config.add_node(
            "openai/gpt-5.4-mini".to_string(),
            sample_node(ProviderKind::OpenAi, "gpt-5.4-mini", vec![Capability::Chat]),
        );

        config.set_default("chat", "openai/gpt-5.4-mini");
        let (id, _) = config.default_node_for("chat").unwrap();
        assert_eq!(id, "openai/gpt-5.4-mini");

        config.unset_default("chat");
        assert!(config.default_node_for("chat").is_err());
    }

    #[test]
    fn test_remove_node_cleans_defaults() {
        let mut config = Config::default();
        config.add_node(
            "openai/gpt-5.4-mini".to_string(),
            sample_node(
                ProviderKind::OpenAi,
                "gpt-5.4-mini",
                vec![Capability::Chat, Capability::Image],
            ),
        );
        config.set_default("chat", "openai/gpt-5.4-mini");
        config.set_default("image", "openai/gpt-5.4-mini");

        assert!(config.remove_node("openai/gpt-5.4-mini"));
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
        assert_eq!(
            "embedding".parse::<Capability>().unwrap(),
            Capability::Embedding
        );
        assert_eq!("video".parse::<Capability>().unwrap(), Capability::Video);
        assert!("invalid".parse::<Capability>().is_err());
    }

    #[test]
    fn test_capability_video_serde_roundtrip() {
        let json = serde_json::to_string(&Capability::Video).unwrap();
        assert_eq!(json, "\"video\"");
        let parsed: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Capability::Video);
        assert_eq!(Capability::Video.config_key(), "video");
        assert_eq!(Capability::Video.label(), "Video Generation");
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
        let mut node = sample_node(ProviderKind::OpenAi, "gpt-5.4-mini", vec![]);
        assert_eq!(node.detail(), "gpt-5.4-mini");

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
            extends: None,
            source: ConfigSource::Global,
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
            extends: None,
            source: ConfigSource::Global,
            nodes: BTreeMap::new(),
            defaults: BTreeMap::new(),
            consents: BTreeMap::from([("azure-cli".to_string(), true)]),
        };
        let local = Config {
            extends: None,
            source: ConfigSource::Global,
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
            extends: None,
            source: ConfigSource::Global,
            nodes: BTreeMap::from([(
                "openai/gpt-5.4-mini".to_string(),
                sample_node(ProviderKind::OpenAi, "gpt-5.4-mini", vec![Capability::Chat]),
            )]),
            defaults: BTreeMap::from([("chat".to_string(), "openai/gpt-5.4-mini".to_string())]),
            consents: BTreeMap::new(),
        };
        let local = Config {
            extends: None,
            source: ConfigSource::Global,
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
        assert!(merged.nodes.contains_key("openai/gpt-5.4-mini"));
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
        assert!(ProviderKind::MicrosoftFoundry.supports_task("image"));
        assert!(ProviderKind::VertexAi.supports_task("image"));
        assert!(!ProviderKind::Ollama.supports_task("image"));
        assert!(!ProviderKind::LocalAgent.supports_task("image"));
    }

    #[test]
    fn foundry_supports_image() {
        assert!(ProviderKind::MicrosoftFoundry.supports_task("image"));
    }

    #[test]
    fn azure_and_foundry_support_video() {
        assert!(ProviderKind::AzureOpenAi.supports_task("video"));
        assert!(ProviderKind::MicrosoftFoundry.supports_task("video"));
    }

    #[test]
    fn others_do_not_support_video() {
        assert!(!ProviderKind::OpenAi.supports_task("video"));
        assert!(!ProviderKind::Anthropic.supports_task("video"));
        assert!(!ProviderKind::VertexAi.supports_task("video"));
        assert!(!ProviderKind::Ollama.supports_task("video"));
        assert!(!ProviderKind::LocalAgent.supports_task("video"));
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
  openai/gpt-5.4-mini:
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
  chat: openai/gpt-5.4-mini
  image: openai/gpt-5.4-mini
consents:
  azure-cli: true
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.nodes.len(), 2);
        assert_eq!(
            config.nodes["openai/gpt-5.4-mini"].auth,
            Some(Auth::Env("OPENAI_API_KEY".to_string()))
        );
        assert_eq!(
            config.nodes["openai/gpt-5.4-mini"].capabilities,
            vec![Capability::Chat, Capability::Image]
        );
        assert_eq!(
            config.nodes["ollama/llama3.2"].endpoint,
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(config.defaults.get("chat").unwrap(), "openai/gpt-5.4-mini");
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

    #[test]
    fn test_supports_task_embedding() {
        assert!(ProviderKind::OpenAi.supports_task("embedding"));
        assert!(ProviderKind::AzureOpenAi.supports_task("embedding"));
        assert!(ProviderKind::Ollama.supports_task("embedding"));
        assert!(ProviderKind::VertexAi.supports_task("embedding"));
        assert!(ProviderKind::MicrosoftFoundry.supports_task("embedding"));
        assert!(!ProviderKind::Anthropic.supports_task("embedding"));
        assert!(!ProviderKind::LocalAgent.supports_task("embedding"));
    }

    #[test]
    fn test_supported_capabilities_includes_embedding() {
        let caps = ProviderKind::OpenAi.supported_capabilities();
        assert!(caps.contains(&Capability::Embedding));
        let caps = ProviderKind::Anthropic.supported_capabilities();
        assert!(!caps.contains(&Capability::Embedding));
    }

    #[test]
    fn test_embedding_metadata_azure() {
        let node = AiNode {
            provider: ProviderKind::AzureOpenAi,
            alias: None,
            capabilities: vec![Capability::Embedding],
            auth: None,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: Some("https://myresource.openai.azure.com".to_string()),
            deployment: Some("text-embedding-3-large".to_string()),
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: Some(BTreeMap::from([(
                "dimensions".to_string(),
                "3072".to_string(),
            )])),
        };
        let meta = node.embedding_metadata();
        assert_eq!(meta.provider, ProviderKind::AzureOpenAi);
        assert_eq!(meta.model.as_deref(), Some("text-embedding-3-large"));
        assert_eq!(
            meta.endpoint.as_deref(),
            Some("https://myresource.openai.azure.com")
        );
        assert_eq!(meta.deployment.as_deref(), Some("text-embedding-3-large"));
        assert_eq!(meta.dimensions, Some(3072));
    }

    #[test]
    fn test_azure_search_vectorizer_no_auth() {
        let meta = EmbeddingMetadata {
            provider: ProviderKind::AzureOpenAi,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: Some("https://myresource.openai.azure.com".to_string()),
            deployment: Some("text-embedding-3-large".to_string()),
            dimensions: Some(3072),
            auth: None,
        };
        let vectorizer = meta.to_azure_search_vectorizer("my-vectorizer").unwrap();
        assert_eq!(vectorizer["name"], "my-vectorizer");
        assert_eq!(vectorizer["kind"], "azureOpenAI");
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["resourceUri"],
            "https://myresource.openai.azure.com"
        );
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["deploymentId"],
            "text-embedding-3-large"
        );
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["modelName"],
            "text-embedding-3-large"
        );
        // No apiKey when auth is None (Azure CLI / managed identity)
        assert!(vectorizer["azureOpenAIParameters"].get("apiKey").is_none());
    }

    #[test]
    fn test_azure_search_vectorizer_with_api_key() {
        let meta = EmbeddingMetadata {
            provider: ProviderKind::AzureOpenAi,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: Some("https://myresource.openai.azure.com".to_string()),
            deployment: Some("text-embedding-3-large".to_string()),
            dimensions: Some(3072),
            auth: Some(Auth::ApiKey("my-secret-key".to_string())),
        };
        let vectorizer = meta.to_azure_search_vectorizer("my-vectorizer").unwrap();
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["apiKey"],
            "my-secret-key"
        );
    }

    #[test]
    fn test_azure_search_vectorizer_azure_cli_no_api_key() {
        let meta = EmbeddingMetadata {
            provider: ProviderKind::AzureOpenAi,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: Some("https://myresource.openai.azure.com".to_string()),
            deployment: Some("text-embedding-3-large".to_string()),
            dimensions: Some(3072),
            auth: Some(Auth::AzureCli(true)),
        };
        let vectorizer = meta.to_azure_search_vectorizer("my-vectorizer").unwrap();
        // Azure CLI auth → no apiKey in vectorizer config
        assert!(vectorizer["azureOpenAIParameters"].get("apiKey").is_none());
    }

    #[test]
    fn test_azure_search_vectorizer_foundry_endpoint_conversion() {
        let meta = EmbeddingMetadata {
            provider: ProviderKind::MicrosoftFoundry,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: Some("https://mklabaifndr.services.ai.azure.com".to_string()),
            deployment: None,
            dimensions: Some(3072),
            auth: None,
        };
        let vectorizer = meta.to_azure_search_vectorizer("my-vectorizer").unwrap();
        assert_eq!(vectorizer["name"], "my-vectorizer");
        assert_eq!(vectorizer["kind"], "azureOpenAI");
        // Foundry endpoint converted to .openai.azure.com
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["resourceUri"],
            "https://mklabaifndr.openai.azure.com"
        );
        // Model used as deploymentId when no deployment is set
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["deploymentId"],
            "text-embedding-3-large"
        );
        assert_eq!(
            vectorizer["azureOpenAIParameters"]["modelName"],
            "text-embedding-3-large"
        );
    }

    #[test]
    fn test_azure_search_vectorizer_non_azure_fails() {
        let meta = EmbeddingMetadata {
            provider: ProviderKind::OpenAi,
            model: Some("text-embedding-3-small".to_string()),
            endpoint: None,
            deployment: None,
            dimensions: None,
            auth: None,
        };
        assert!(meta.to_azure_search_vectorizer("test").is_err());
    }

    #[test]
    fn test_well_known_embedding_dimensions() {
        assert_eq!(
            well_known_embedding_dimensions("text-embedding-3-large"),
            Some(3072)
        );
        assert_eq!(
            well_known_embedding_dimensions("text-embedding-3-small"),
            Some(1536)
        );
        assert_eq!(
            well_known_embedding_dimensions("text-embedding-ada-002"),
            Some(1536)
        );
        assert_eq!(
            well_known_embedding_dimensions("text-embedding-004"),
            Some(768)
        );
        assert_eq!(
            well_known_embedding_dimensions("nomic-embed-text"),
            Some(768)
        );
        assert_eq!(well_known_embedding_dimensions("all-minilm"), Some(384));
        assert_eq!(well_known_embedding_dimensions("unknown-model"), None);
    }

    #[test]
    fn test_embedding_metadata_auto_dimensions() {
        // No explicit defaults.dimensions — should auto-detect from model name
        let node = AiNode {
            provider: ProviderKind::MicrosoftFoundry,
            alias: None,
            capabilities: vec![Capability::Embedding],
            auth: None,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: Some("https://mklabaifndr.services.ai.azure.com".to_string()),
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        };
        let meta = node.embedding_metadata();
        assert_eq!(meta.dimensions, Some(3072));
    }

    #[test]
    fn test_embedding_metadata_explicit_overrides_auto() {
        // Explicit dimensions should override auto-detection
        let node = AiNode {
            provider: ProviderKind::OpenAi,
            alias: None,
            capabilities: vec![Capability::Embedding],
            auth: None,
            model: Some("text-embedding-3-large".to_string()),
            endpoint: None,
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: Some(BTreeMap::from([(
                "dimensions".to_string(),
                "256".to_string(),
            )])),
        };
        let meta = node.embedding_metadata();
        // Explicit 256 wins over auto-detected 3072
        assert_eq!(meta.dimensions, Some(256));
    }
}

#[cfg(test)]
mod programmatic_api_tests {
    use super::*;

    fn node() -> AiNode {
        let mut n = AiNode::new(ProviderKind::OpenAi);
        n.model = Some("gpt-5.4-mini".into());
        n.capabilities = vec![Capability::Chat];
        n
    }

    #[test]
    fn ensure_node_inserts_once() {
        let mut c = Config::default();
        assert!(c.ensure_node("openai/x".into(), node()));
        assert!(
            !c.ensure_node("openai/x".into(), node()),
            "second insert is a no-op"
        );
        assert_eq!(c.nodes.len(), 1);
    }

    #[test]
    fn set_default_for_validates() {
        let mut c = Config::default();
        c.ensure_node("openai/x".into(), node());
        assert!(c.set_default_for("chat", "openai/x").is_ok());
        assert_eq!(c.defaults.get("chat").map(String::as_str), Some("openai/x"));
        assert!(c.set_default_for("chat", "missing").is_err());
        assert!(c.set_default_for("bogus-capability", "openai/x").is_err());
    }

    #[test]
    fn upsert_replaces() {
        let mut c = Config::default();
        c.ensure_node("openai/x".into(), node());
        let mut replacement = node();
        replacement.model = Some("gpt-5.5".into());
        c.upsert_node("openai/x".into(), replacement);
        assert_eq!(c.nodes["openai/x"].model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn keychain_auth_round_trips_in_yaml() {
        let mut c = Config::default();
        let mut n = node();
        n.auth = Some(Auth::Keychain(true));
        c.ensure_node("openai/x".into(), n);
        let yaml = serde_yaml::to_string(&c).unwrap();
        assert!(yaml.contains("keychain: true"), "{yaml}");
        let back: Config = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(
            back.nodes["openai/x"].auth,
            Some(Auth::Keychain(true))
        ));
    }
}

#[cfg(test)]
mod local_config_tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn closest_local_file_wins_entirely() {
        let tmp = tempfile::tempdir().unwrap();
        // outer local config
        write(
            &tmp.path().join(".ailloy.yaml"),
            "nodes:\n  outer/x:\n    provider: openai\n    model: outer\ndefaults:\n  chat: outer/x\n",
        );
        // inner local config — the closest one from `deep`
        write(
            &tmp.path().join("project/.ailloy.yaml"),
            "nodes:\n  inner/y:\n    provider: anthropic\n    model: inner\ndefaults:\n  chat: inner/y\n",
        );
        let deep = tmp.path().join("project/src/deep");
        std::fs::create_dir_all(&deep).unwrap();

        let config = Config::load_from_dir(&deep).unwrap();
        assert!(config.nodes.contains_key("inner/y"));
        assert!(
            !config.nodes.contains_key("outer/x"),
            "closest file replaces, not merges"
        );
        assert!(
            matches!(config.source, ConfigSource::Local(ref p) if p.ends_with("project/.ailloy.yaml"))
        );

        // outside `project`, the outer file is the closest
        let config = Config::load_from_dir(&tmp.path().join("elsewhere-does-not-exist-parent"))
            .unwrap_or_else(|_| Config::load_from_dir(tmp.path()).unwrap());
        let config = if config.nodes.is_empty() {
            Config::load_from_dir(tmp.path()).unwrap()
        } else {
            config
        };
        assert!(config.nodes.contains_key("outer/x"));
    }

    #[test]
    fn extends_global_merges_instead() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join(".ailloy.yaml"),
            "extends: global\nnodes:\n  local/z:\n    provider: ollama\n    model: z\ndefaults:\n  chat: local/z\n",
        );
        let config = Config::load_from_dir(tmp.path()).unwrap();
        assert!(config.nodes.contains_key("local/z"));
        assert!(matches!(config.source, ConfigSource::LocalExtendsGlobal(_)));
        // global nodes (if any on this machine) are retained by merge — we
        // can't assert machine state, but the source proves the merge path ran
        // and defaults from local won:
        assert_eq!(
            config.defaults.get("chat").map(String::as_str),
            Some("local/z")
        );
    }

    #[test]
    fn no_local_file_falls_back_to_global_source() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();
        // NOTE: walking up from a tempdir eventually hits $HOME only if tmp is
        // under it — on macOS /var/folders is outside home, so this exercises
        // the pure-global fallback deterministically.
        let config = Config::load_from_dir(&deep).unwrap();
        assert!(matches!(config.source, ConfigSource::Global));
    }

    #[test]
    fn local_never_overrides_consents() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join(".ailloy.yaml"),
            "nodes: {}\ndefaults: {}\nconsents:\n  azure-cli: true\n",
        );
        let config = Config::load_from_dir(tmp.path()).unwrap();
        // consents come from the GLOBAL config, so the local `azure-cli: true`
        // must not appear unless the machine config already grants it — we
        // assert it matches the global config exactly.
        let global = Config::load_global().unwrap();
        assert_eq!(config.consents, global.consents);
    }
}
