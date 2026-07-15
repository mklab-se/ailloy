//! Input-widget state and the multi-field node form used by the config TUI.
//!
//! [`Editor`] is a minimal single-value editor (text or picker) reused by the
//! per-node default editor and the keychain-secret prompt. [`NodeForm`] is the
//! add/edit-node form: a provider selector followed by provider-specific fields,
//! an auth selector, capability toggles, and pseudo-fields for discovery and
//! saving. It stays free of terminal concerns so [`NodeForm::to_node`] and the
//! reducer that drives it are unit-testable without a terminal.

use anyhow::{Result, anyhow};

use crate::azure_discover::capabilities_for_deployment;
use crate::config::{AiNode, Auth, Capability, ProviderKind};

/// A single-value editor: either free-text with a cursor, or a selection from a
/// fixed list of choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    /// The current text value (also the resolved value for a selection).
    pub value: String,
    /// Cursor position within `value`, as a byte offset for text editing.
    pub cursor: usize,
    /// A validation error to surface next to the field, if any.
    pub error: Option<String>,
    /// When present, the editor is a picker: `(choices, selected_index)`.
    pub choices: Option<(Vec<String>, usize)>,
}

impl Editor {
    /// A free-text editor seeded with `initial`, cursor at the end.
    pub fn text(initial: impl Into<String>) -> Self {
        let value = initial.into();
        let cursor = value.len();
        Editor {
            value,
            cursor,
            error: None,
            choices: None,
        }
    }

    /// A selection editor over `choices`, starting at `current` (clamped).
    pub fn select(choices: Vec<String>, current: usize) -> Self {
        let current = current.min(choices.len().saturating_sub(1));
        let value = choices.get(current).cloned().unwrap_or_default();
        Editor {
            value,
            cursor: 0,
            error: None,
            choices: Some((choices, current)),
        }
    }
}

/// The seven configurable provider kinds, in display order.
pub const PROVIDER_ORDER: [ProviderKind; 7] = [
    ProviderKind::OpenAi,
    ProviderKind::Anthropic,
    ProviderKind::AzureOpenAi,
    ProviderKind::MicrosoftFoundry,
    ProviderKind::VertexAi,
    ProviderKind::Ollama,
    ProviderKind::LocalAgent,
];

/// The default Ollama endpoint offered when adding an Ollama node.
pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// The local-agent binaries offered in the binary picker.
const LOCAL_AGENT_BINARIES: [&str; 3] = ["claude", "codex", "copilot"];

/// Semantic identity of a [`FormField`], so the reducer and [`NodeForm::to_node`]
/// can find fields without relying on their display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKey {
    Provider,
    Model,
    Endpoint,
    Deployment,
    ApiVersion,
    Binary,
    Project,
    Location,
    Auth,
    ApiKey,
    Alias,
    Capabilities,
    Discover,
    Save,
}

/// The interactive shape of a form field.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// Free text with a byte-offset cursor.
    Text { value: String, cursor: usize },
    /// A one-of picker.
    Select {
        options: Vec<String>,
        selected: usize,
    },
    /// Capability checkboxes with a highlight cursor.
    Toggles {
        options: Vec<Capability>,
        checked: Vec<bool>,
        cursor: usize,
    },
    /// A non-value action (e.g. `[Save]`, `[Discover]`).
    Action,
}

/// A single labeled field within a [`NodeForm`].
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    pub key: FieldKey,
    pub label: String,
    pub kind: FieldKind,
}

impl FormField {
    fn text(key: FieldKey, label: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        FormField {
            key,
            label: label.to_string(),
            kind: FieldKind::Text { value, cursor },
        }
    }

    fn select(key: FieldKey, label: &str, options: Vec<String>, selected: usize) -> Self {
        let selected = selected.min(options.len().saturating_sub(1));
        FormField {
            key,
            label: label.to_string(),
            kind: FieldKind::Select { options, selected },
        }
    }

    fn action(key: FieldKey, label: &str) -> Self {
        FormField {
            key,
            label: label.to_string(),
            kind: FieldKind::Action,
        }
    }

    /// The current text value, if this is a text field.
    pub fn text_value(&self) -> Option<&str> {
        match &self.kind {
            FieldKind::Text { value, .. } => Some(value.as_str()),
            _ => None,
        }
    }

    /// The current selected option string, if this is a select field.
    pub fn select_value(&self) -> Option<&str> {
        match &self.kind {
            FieldKind::Select { options, selected } => options.get(*selected).map(String::as_str),
            _ => None,
        }
    }
}

/// The auth options offered for a provider (empty = no auth field).
fn auth_options(provider: &ProviderKind) -> Vec<String> {
    match provider {
        ProviderKind::OpenAi | ProviderKind::Anthropic => {
            vec!["env".into(), "api_key".into(), "keychain".into()]
        }
        ProviderKind::AzureOpenAi | ProviderKind::MicrosoftFoundry => {
            vec!["api_key".into(), "keychain".into(), "azure_cli".into()]
        }
        ProviderKind::VertexAi => vec!["gcloud_cli".into()],
        ProviderKind::Ollama | ProviderKind::LocalAgent => vec![],
    }
}

/// The default auth option index for a provider.
fn default_auth_index(provider: &ProviderKind) -> usize {
    match provider {
        // env is the recommended default for the hosted API providers.
        ProviderKind::OpenAi | ProviderKind::Anthropic => 0,
        // azure_cli (index 2) is recommended for Azure/Foundry.
        ProviderKind::AzureOpenAi | ProviderKind::MicrosoftFoundry => 2,
        _ => 0,
    }
}

/// The conventional env var holding a provider's API key.
fn env_var_for(provider: &ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
        _ => "OPENAI_API_KEY",
    }
}

/// The auth option string that best represents an existing [`Auth`].
fn auth_choice_for(auth: &Option<Auth>) -> Option<&'static str> {
    match auth {
        Some(Auth::Env(_)) => Some("env"),
        Some(Auth::ApiKey(_)) => Some("api_key"),
        Some(Auth::Keychain(_)) => Some("keychain"),
        Some(Auth::AzureCli(_)) => Some("azure_cli"),
        Some(Auth::GcloudCli(_)) => Some("gcloud_cli"),
        None => None,
    }
}

/// A multi-field form for adding or editing a node.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeForm {
    /// The provider this form is building a node for.
    pub provider: ProviderKind,
    /// True when editing an existing node (provider is fixed, id is derived).
    pub editing: bool,
    /// The fields, in display order.
    pub fields: Vec<FormField>,
    /// Index of the active field.
    pub active: usize,
    /// A validation error from the last save attempt.
    pub error: Option<String>,
}

impl NodeForm {
    /// A fresh add-node form defaulting to the OpenAI provider.
    pub fn new() -> Self {
        let provider = ProviderKind::OpenAi;
        let fields = build_fields(&provider, false, None);
        NodeForm {
            provider,
            editing: false,
            fields,
            active: 0,
            error: None,
        }
    }

    /// An edit-node form prefilled from an existing node. The provider is fixed.
    pub fn from_node(_id: &str, node: &AiNode) -> Self {
        let fields = build_fields(&node.provider, true, Some(node));
        NodeForm {
            provider: node.provider.clone(),
            editing: true,
            fields,
            active: 0,
            error: None,
        }
    }

    /// Rebuild the field list for the current provider, preserving as much
    /// state as makes sense (used when the provider or auth selection changes).
    pub fn rebuild(&mut self) {
        // Snapshot the current auth selection so it survives the rebuild.
        let auth_choice = self
            .field(FieldKey::Auth)
            .and_then(FormField::select_value)
            .map(str::to_string);
        let api_key = self
            .field(FieldKey::ApiKey)
            .and_then(FormField::text_value)
            .map(str::to_string);
        self.fields =
            build_fields_with_auth(&self.provider, self.editing, None, auth_choice, api_key);
        if self.active >= self.fields.len() {
            self.active = self.fields.len().saturating_sub(1);
        }
    }

    /// The field with the given key, if present.
    pub fn field(&self, key: FieldKey) -> Option<&FormField> {
        self.fields.iter().find(|f| f.key == key)
    }

    fn text_of(&self, key: FieldKey) -> String {
        self.field(key)
            .and_then(FormField::text_value)
            .unwrap_or("")
            .trim()
            .to_string()
    }

    /// The currently selected auth option string, if any.
    pub fn auth_choice(&self) -> Option<String> {
        self.field(FieldKey::Auth)
            .and_then(FormField::select_value)
            .map(str::to_string)
    }

    /// The capability toggles' currently-checked capabilities.
    fn selected_capabilities(&self) -> Vec<Capability> {
        match self.field(FieldKey::Capabilities).map(|f| &f.kind) {
            Some(FieldKind::Toggles {
                options, checked, ..
            }) => options
                .iter()
                .zip(checked.iter())
                .filter(|(_, on)| **on)
                .map(|(c, _)| c.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Build the `(id, node)` pair from the form, validating required fields.
    ///
    /// The node id follows `{provider}/{model|deployment|binary}`. Errors are
    /// actionable, naming the missing field and what to enter.
    pub fn to_node(&self) -> Result<(String, AiNode)> {
        let provider = self.provider.clone();
        let mut node = AiNode::new(provider.clone());

        // --- auth ---------------------------------------------------------
        node.auth = self.resolve_auth()?;

        // --- alias --------------------------------------------------------
        let alias = self.text_of(FieldKey::Alias);
        node.alias = if alias.is_empty() { None } else { Some(alias) };

        // --- capabilities -------------------------------------------------
        let caps = self.selected_capabilities();
        if caps.is_empty() {
            return Err(anyhow!(
                "select at least one capability (Space toggles the highlighted one)"
            ));
        }
        node.capabilities = caps;

        // --- provider-specific fields + id --------------------------------
        let id = match provider {
            ProviderKind::OpenAi => {
                let model = self.require(FieldKey::Model, "model")?;
                node.model = Some(model.clone());
                let endpoint = self.text_of(FieldKey::Endpoint);
                node.endpoint = (!endpoint.is_empty()).then_some(endpoint);
                format!("openai/{model}")
            }
            ProviderKind::Anthropic => {
                let model = self.require(FieldKey::Model, "model")?;
                node.model = Some(model.clone());
                format!("anthropic/{model}")
            }
            ProviderKind::AzureOpenAi => {
                let endpoint = self.require(
                    FieldKey::Endpoint,
                    "endpoint (e.g. https://my-instance.openai.azure.com)",
                )?;
                let deployment = self.require(FieldKey::Deployment, "deployment name")?;
                node.endpoint = Some(endpoint);
                node.deployment = Some(deployment.clone());
                node.api_version = self.optional_api_version();
                format!("azure-openai/{deployment}")
            }
            ProviderKind::MicrosoftFoundry => {
                let endpoint = self.require(
                    FieldKey::Endpoint,
                    "endpoint (e.g. https://my-instance.services.ai.azure.com)",
                )?;
                let model = self.require(FieldKey::Model, "model (e.g. gpt-4o)")?;
                node.endpoint = Some(endpoint);
                node.model = Some(model.clone());
                node.api_version = self.optional_api_version();
                format!("microsoft-foundry/{model}")
            }
            ProviderKind::VertexAi => {
                let project = self.require(FieldKey::Project, "GCP project")?;
                let location = self.require(FieldKey::Location, "location (e.g. us-central1)")?;
                let model = self.require(FieldKey::Model, "model")?;
                node.project = Some(project);
                node.location = Some(location);
                node.model = Some(model.clone());
                format!("vertex-ai/{model}")
            }
            ProviderKind::Ollama => {
                let model = self.require(FieldKey::Model, "model")?;
                node.model = Some(model.clone());
                let endpoint = self.text_of(FieldKey::Endpoint);
                node.endpoint = (!endpoint.is_empty() && endpoint != OLLAMA_DEFAULT_ENDPOINT)
                    .then_some(endpoint);
                format!("ollama/{model}")
            }
            ProviderKind::LocalAgent => {
                let binary = self.require(FieldKey::Binary, "agent binary")?;
                node.binary = Some(binary.clone());
                format!("local-agent/{binary}")
            }
        };

        Ok((id, node))
    }

    /// Read a required field, erroring with an actionable message if empty.
    fn require(&self, key: FieldKey, what: &str) -> Result<String> {
        let value = self
            .field(key)
            .and_then(|f| f.text_value().or_else(|| f.select_value()))
            .unwrap_or("")
            .trim()
            .to_string();
        if value.is_empty() {
            Err(anyhow!("{what} is required"))
        } else {
            Ok(value)
        }
    }

    /// The optional api_version → `None` (unified v1 surface) when blank.
    fn optional_api_version(&self) -> Option<String> {
        let v = self.text_of(FieldKey::ApiVersion);
        (!v.is_empty()).then_some(v)
    }

    /// Resolve the [`Auth`] from the auth selector and (for api_key) the key.
    fn resolve_auth(&self) -> Result<Option<Auth>> {
        let Some(choice) = self.auth_choice() else {
            return Ok(None);
        };
        let auth = match choice.as_str() {
            "env" => Auth::Env(env_var_for(&self.provider).to_string()),
            "api_key" => {
                let key = self.text_of(FieldKey::ApiKey);
                if key.is_empty() {
                    return Err(anyhow!(
                        "API key is required for api_key auth (or choose env/keychain)"
                    ));
                }
                Auth::ApiKey(key)
            }
            "keychain" => Auth::Keychain(true),
            "azure_cli" => Auth::AzureCli(true),
            "gcloud_cli" => Auth::GcloudCli(true),
            other => return Err(anyhow!("unknown auth method '{other}'")),
        };
        Ok(Some(auth))
    }
}

impl Default for NodeForm {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the fields for a provider with default auth (used by `new`/`from_node`).
fn build_fields(
    provider: &ProviderKind,
    editing: bool,
    prefill: Option<&AiNode>,
) -> Vec<FormField> {
    let auth_choice = prefill
        .and_then(|n| auth_choice_for(&n.auth))
        .map(str::to_string);
    let api_key = prefill.and_then(|n| match &n.auth {
        Some(Auth::ApiKey(k)) => Some(k.clone()),
        _ => None,
    });
    build_fields_with_auth(provider, editing, prefill, auth_choice, api_key)
}

/// Build the fields for a provider, honoring an explicit auth selection.
fn build_fields_with_auth(
    provider: &ProviderKind,
    editing: bool,
    prefill: Option<&AiNode>,
    auth_choice: Option<String>,
    api_key: Option<String>,
) -> Vec<FormField> {
    let mut fields: Vec<FormField> = Vec::new();

    // Provider selector (only when adding — editing keeps the provider fixed).
    if !editing {
        let selected = PROVIDER_ORDER
            .iter()
            .position(|p| p == provider)
            .unwrap_or(0);
        let options = PROVIDER_ORDER.iter().map(|p| p.to_string()).collect();
        fields.push(FormField::select(
            FieldKey::Provider,
            "provider",
            options,
            selected,
        ));
    }

    let model = prefill.and_then(|n| n.model.clone()).unwrap_or_default();
    let endpoint = prefill.and_then(|n| n.endpoint.clone());
    let deployment = prefill
        .and_then(|n| n.deployment.clone())
        .unwrap_or_default();
    let api_version = prefill
        .and_then(|n| n.api_version.clone())
        .unwrap_or_default();
    let project = prefill.and_then(|n| n.project.clone()).unwrap_or_default();
    let location = prefill.and_then(|n| n.location.clone()).unwrap_or_default();

    match provider {
        ProviderKind::OpenAi => {
            fields.push(FormField::text(FieldKey::Model, "model", model.clone()));
            fields.push(FormField::text(
                FieldKey::Endpoint,
                "endpoint (optional)",
                endpoint.unwrap_or_default(),
            ));
        }
        ProviderKind::Anthropic => {
            fields.push(FormField::text(FieldKey::Model, "model", model.clone()));
        }
        ProviderKind::AzureOpenAi => {
            fields.push(FormField::text(
                FieldKey::Endpoint,
                "endpoint",
                endpoint.unwrap_or_default(),
            ));
            fields.push(FormField::text(
                FieldKey::Deployment,
                "deployment",
                deployment,
            ));
            fields.push(FormField::text(
                FieldKey::ApiVersion,
                "api_version (optional, blank = unified v1)",
                api_version,
            ));
        }
        ProviderKind::MicrosoftFoundry => {
            fields.push(FormField::text(
                FieldKey::Endpoint,
                "endpoint",
                endpoint.unwrap_or_default(),
            ));
            fields.push(FormField::text(FieldKey::Model, "model", model.clone()));
            fields.push(FormField::text(
                FieldKey::ApiVersion,
                "api_version (optional, blank = unified v1)",
                api_version,
            ));
        }
        ProviderKind::VertexAi => {
            fields.push(FormField::text(FieldKey::Project, "project", project));
            fields.push(FormField::text(
                FieldKey::Location,
                "location",
                if location.is_empty() {
                    "us-central1".to_string()
                } else {
                    location
                },
            ));
            fields.push(FormField::text(FieldKey::Model, "model", model.clone()));
        }
        ProviderKind::Ollama => {
            fields.push(FormField::text(
                FieldKey::Endpoint,
                "endpoint",
                endpoint.unwrap_or_else(|| OLLAMA_DEFAULT_ENDPOINT.to_string()),
            ));
            fields.push(FormField::text(FieldKey::Model, "model", model.clone()));
        }
        ProviderKind::LocalAgent => {
            let binary = prefill.and_then(|n| n.binary.clone()).unwrap_or_default();
            let selected = LOCAL_AGENT_BINARIES
                .iter()
                .position(|b| *b == binary)
                .unwrap_or(0);
            fields.push(FormField::select(
                FieldKey::Binary,
                "binary",
                LOCAL_AGENT_BINARIES.iter().map(|s| s.to_string()).collect(),
                selected,
            ));
        }
    }

    // Auth selector (providers that have auth options).
    let opts = auth_options(provider);
    if !opts.is_empty() {
        let selected = auth_choice
            .as_deref()
            .and_then(|c| opts.iter().position(|o| o == c))
            .unwrap_or_else(|| default_auth_index(provider));
        let is_api_key = opts.get(selected).map(String::as_str) == Some("api_key");
        fields.push(FormField::select(FieldKey::Auth, "auth", opts, selected));
        if is_api_key {
            fields.push(FormField::text(
                FieldKey::ApiKey,
                "api key",
                api_key.unwrap_or_default(),
            ));
        }
    }

    // Alias (optional, all providers).
    let alias = prefill.and_then(|n| n.alias.clone()).unwrap_or_default();
    fields.push(FormField::text(FieldKey::Alias, "alias (optional)", alias));

    // Capability toggles, constrained by what the provider supports.
    let supported = provider.supported_capabilities();
    let initial: Vec<Capability> = match prefill {
        Some(n) => n.capabilities.clone(),
        None => {
            if model.is_empty() {
                vec![Capability::Chat]
            } else {
                capabilities_for_deployment(&model)
            }
        }
    };
    let checked = supported.iter().map(|c| initial.contains(c)).collect();
    fields.push(FormField {
        key: FieldKey::Capabilities,
        label: "capabilities".to_string(),
        kind: FieldKind::Toggles {
            options: supported,
            checked,
            cursor: 0,
        },
    });

    // Discovery pseudo-field for Azure/Foundry (add-node only).
    if !editing
        && matches!(
            provider,
            ProviderKind::AzureOpenAi | ProviderKind::MicrosoftFoundry
        )
    {
        fields.push(FormField::action(
            FieldKey::Discover,
            "[ Discover via az CLI ]",
        ));
    }

    // Save action.
    fields.push(FormField::action(FieldKey::Save, "[ Save ]"));

    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_editor_places_cursor_at_end() {
        let ed = Editor::text("hello");
        assert_eq!(ed.value, "hello");
        assert_eq!(ed.cursor, 5);
        assert!(ed.error.is_none());
        assert!(ed.choices.is_none());
    }

    #[test]
    fn select_editor_seeds_value_from_choice() {
        let ed = Editor::select(vec!["a".into(), "b".into(), "c".into()], 1);
        assert_eq!(ed.value, "b");
        assert_eq!(ed.choices.unwrap().1, 1);
    }

    #[test]
    fn select_editor_clamps_out_of_range_index() {
        let ed = Editor::select(vec!["a".into(), "b".into()], 9);
        assert_eq!(ed.value, "b");
        assert_eq!(ed.choices.unwrap().1, 1);
    }

    /// Set the text value of a field by key (test helper).
    fn set_text(form: &mut NodeForm, key: FieldKey, value: &str) {
        let field = form.fields.iter_mut().find(|f| f.key == key).unwrap();
        field.kind = FieldKind::Text {
            value: value.to_string(),
            cursor: value.len(),
        };
    }

    /// Select an option string on a select field by key (test helper).
    fn set_select(form: &mut NodeForm, key: FieldKey, value: &str) {
        let field = form.fields.iter_mut().find(|f| f.key == key).unwrap();
        if let FieldKind::Select { options, selected } = &mut field.kind {
            *selected = options.iter().position(|o| o == value).unwrap();
        } else {
            panic!("field {key:?} is not a select");
        }
    }

    fn check_all_caps(form: &mut NodeForm) {
        let field = form
            .fields
            .iter_mut()
            .find(|f| f.key == FieldKey::Capabilities)
            .unwrap();
        if let FieldKind::Toggles { checked, .. } = &mut field.kind {
            for c in checked.iter_mut() {
                *c = true;
            }
        }
    }

    fn openai_form() -> NodeForm {
        let mut form = NodeForm::new();
        set_text(&mut form, FieldKey::Model, "gpt-5.4-mini");
        form
    }

    #[test]
    fn openai_to_node_maps_env_auth_and_id() {
        let form = openai_form();
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "openai/gpt-5.4-mini");
        assert_eq!(node.provider, ProviderKind::OpenAi);
        assert_eq!(node.model.as_deref(), Some("gpt-5.4-mini"));
        assert_eq!(node.auth, Some(Auth::Env("OPENAI_API_KEY".into())));
        assert_eq!(node.capabilities, vec![Capability::Chat]);
    }

    #[test]
    fn openai_api_key_auth_requires_a_key() {
        let mut form = openai_form();
        // Selecting api_key inserts the ApiKey field on rebuild.
        set_select(&mut form, FieldKey::Auth, "api_key");
        form.rebuild();
        // Model was cleared by rebuild (fresh, no prefill); re-enter it.
        set_text(&mut form, FieldKey::Model, "gpt-5.4-mini");
        let err = form.to_node().unwrap_err().to_string();
        assert!(err.contains("API key is required"), "message: {err}");

        set_text(&mut form, FieldKey::ApiKey, "sk-123");
        let (_, node) = form.to_node().unwrap();
        assert_eq!(node.auth, Some(Auth::ApiKey("sk-123".into())));
    }

    #[test]
    fn openai_keychain_auth_maps_to_keychain_true() {
        let mut form = openai_form();
        set_select(&mut form, FieldKey::Auth, "keychain");
        let (_, node) = form.to_node().unwrap();
        assert_eq!(node.auth, Some(Auth::Keychain(true)));
    }

    #[test]
    fn anthropic_uses_anthropic_env_var() {
        let mut form = NodeForm::new();
        set_select(&mut form, FieldKey::Provider, "anthropic");
        form.provider = ProviderKind::Anthropic;
        form.rebuild();
        set_text(&mut form, FieldKey::Model, "claude-sonnet-5");
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "anthropic/claude-sonnet-5");
        assert_eq!(node.auth, Some(Auth::Env("ANTHROPIC_API_KEY".into())));
    }

    #[test]
    fn azure_openai_builds_deployment_id_and_optional_api_version() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::AzureOpenAi;
        form.rebuild();
        set_text(&mut form, FieldKey::Endpoint, "https://x.openai.azure.com");
        set_text(&mut form, FieldKey::Deployment, "gpt-4o");
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "azure-openai/gpt-4o");
        assert_eq!(node.deployment.as_deref(), Some("gpt-4o"));
        assert!(node.api_version.is_none(), "blank api_version → unified v1");
        assert_eq!(node.auth, Some(Auth::AzureCli(true)));
    }

    #[test]
    fn azure_openai_missing_endpoint_errors_actionably() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::AzureOpenAi;
        form.rebuild();
        set_text(&mut form, FieldKey::Deployment, "gpt-4o");
        let err = form.to_node().unwrap_err().to_string();
        assert!(err.contains("endpoint"), "message: {err}");
    }

    #[test]
    fn foundry_builds_model_id_and_endpoint() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::MicrosoftFoundry;
        form.rebuild();
        set_text(
            &mut form,
            FieldKey::Endpoint,
            "https://x.services.ai.azure.com",
        );
        set_text(&mut form, FieldKey::Model, "gpt-4o");
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "microsoft-foundry/gpt-4o");
        assert_eq!(node.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn vertex_requires_project_location_model_and_uses_gcloud() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::VertexAi;
        form.rebuild();
        set_text(&mut form, FieldKey::Project, "my-proj");
        set_text(&mut form, FieldKey::Model, "gemini-3-pro");
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "vertex-ai/gemini-3-pro");
        assert_eq!(node.project.as_deref(), Some("my-proj"));
        // Location has a us-central1 default prefilled.
        assert_eq!(node.location.as_deref(), Some("us-central1"));
        assert_eq!(node.auth, Some(Auth::GcloudCli(true)));
    }

    #[test]
    fn ollama_default_endpoint_is_dropped() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::Ollama;
        form.rebuild();
        set_text(&mut form, FieldKey::Model, "llama3.2");
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "ollama/llama3.2");
        assert!(node.endpoint.is_none(), "default endpoint not persisted");
        assert!(node.auth.is_none());
    }

    #[test]
    fn ollama_custom_endpoint_is_kept() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::Ollama;
        form.rebuild();
        set_text(&mut form, FieldKey::Model, "llama3.2");
        set_text(&mut form, FieldKey::Endpoint, "http://box:11434");
        let (_, node) = form.to_node().unwrap();
        assert_eq!(node.endpoint.as_deref(), Some("http://box:11434"));
    }

    #[test]
    fn local_agent_builds_binary_id() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::LocalAgent;
        form.rebuild();
        // default binary select = claude
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "local-agent/claude");
        assert_eq!(node.binary.as_deref(), Some("claude"));
        assert!(node.auth.is_none());
    }

    #[test]
    fn alias_and_multiple_capabilities_round_trip() {
        let mut form = openai_form();
        set_text(&mut form, FieldKey::Alias, "fast");
        check_all_caps(&mut form);
        let (_, node) = form.to_node().unwrap();
        assert_eq!(node.alias.as_deref(), Some("fast"));
        assert!(node.capabilities.contains(&Capability::Chat));
        assert!(node.capabilities.contains(&Capability::Image));
    }

    #[test]
    fn from_node_round_trips_an_existing_node() {
        let mut node = AiNode::new(ProviderKind::AzureOpenAi);
        node.endpoint = Some("https://x.openai.azure.com".into());
        node.deployment = Some("gpt-4o".into());
        node.api_version = Some("2025-04-01-preview".into());
        node.auth = Some(Auth::ApiKey("sk-xyz".into()));
        node.alias = Some("azure-fast".into());
        node.capabilities = vec![Capability::Chat, Capability::Embedding];

        let form = NodeForm::from_node("azure-openai/gpt-4o", &node);
        assert!(form.editing);
        assert!(
            form.field(FieldKey::Provider).is_none(),
            "provider fixed when editing"
        );
        let (id, rebuilt) = form.to_node().unwrap();
        assert_eq!(id, "azure-openai/gpt-4o");
        assert_eq!(rebuilt.endpoint, node.endpoint);
        assert_eq!(rebuilt.deployment, node.deployment);
        assert_eq!(rebuilt.api_version, node.api_version);
        assert_eq!(rebuilt.auth, node.auth);
        assert_eq!(rebuilt.alias, node.alias);
        assert_eq!(rebuilt.capabilities, node.capabilities);
    }

    #[test]
    fn no_capabilities_selected_errors() {
        let mut form = openai_form();
        let field = form
            .fields
            .iter_mut()
            .find(|f| f.key == FieldKey::Capabilities)
            .unwrap();
        if let FieldKind::Toggles { checked, .. } = &mut field.kind {
            for c in checked.iter_mut() {
                *c = false;
            }
        }
        let err = form.to_node().unwrap_err().to_string();
        assert!(err.contains("capability"), "message: {err}");
    }

    #[test]
    fn azure_form_has_discover_pseudo_field() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::AzureOpenAi;
        form.rebuild();
        assert!(form.field(FieldKey::Discover).is_some());
        // Editing must not offer discovery.
        let mut node = AiNode::new(ProviderKind::AzureOpenAi);
        node.deployment = Some("d".into());
        let edit = NodeForm::from_node("azure-openai/d", &node);
        assert!(edit.field(FieldKey::Discover).is_none());
    }
}
