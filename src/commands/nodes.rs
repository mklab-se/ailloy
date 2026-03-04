use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use colored::Colorize;
use inquire::Text;

use ailloy::config::{ALL_CAPABILITIES, AiNode, Auth, Capability, Config, ProviderKind};

use crate::cli::NodeCommands;

// --- Inquire helpers ---

/// Select from a list of options. Returns the index, or None if cancelled.
fn prompt_select(message: &str, options: &[String]) -> Result<Option<usize>> {
    match inquire::Select::new(message, options.to_vec()).prompt() {
        Ok(selected) => Ok(options.iter().position(|o| *o == selected)),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Confirm prompt. Returns false on cancel.
fn prompt_confirm(message: &str) -> Result<bool> {
    match inquire::Confirm::new(message).with_default(false).prompt() {
        Ok(val) => Ok(val),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Describe an auth configuration for display in the reuse prompt.
fn auth_summary(node: &AiNode) -> String {
    let auth_part = match &node.auth {
        Some(Auth::Env(var)) => format!("env: {}", var),
        Some(Auth::ApiKey(_)) => "api_key: ********".to_string(),
        Some(Auth::AzureCli(_)) => "azure_cli".to_string(),
        Some(Auth::GcloudCli(_)) => "gcloud_cli".to_string(),
        None => "none".to_string(),
    };
    if let Some(ep) = &node.endpoint {
        format!("{}, endpoint: {}", auth_part, ep)
    } else {
        auth_part
    }
}

/// Holds reusable connection config from an existing node.
struct ReusedConnection {
    auth: Option<Auth>,
    endpoint: Option<String>,
}

/// Find existing nodes matching the given provider kind and optional ID prefix.
fn find_reusable_nodes<'a>(
    config: &'a Config,
    provider: &ProviderKind,
    id_prefix: Option<&str>,
) -> Vec<(&'a String, &'a AiNode)> {
    config
        .nodes
        .iter()
        .filter(|(id, n)| {
            &n.provider == provider && id_prefix.is_none_or(|prefix| id.starts_with(prefix))
        })
        .collect()
}

/// Check if there are existing nodes matching the given filter and offer to reuse
/// their connection config. Returns `None` if the user wants new configuration
/// (or there are no matching nodes).
///
/// `id_prefix` filters nodes by ID prefix (e.g. "openai/" or "lm-studio/").
/// If `None`, all nodes matching the provider kind are included.
fn prompt_reuse_connection(
    config: &Config,
    provider: &ProviderKind,
    id_prefix: Option<&str>,
) -> Result<Option<ReusedConnection>> {
    let existing = find_reusable_nodes(config, provider, id_prefix);

    if existing.is_empty() {
        return Ok(None);
    }

    let mut options: Vec<String> = existing
        .iter()
        .map(|(id, node)| format!("{} ({})", id, auth_summary(node)))
        .collect();
    options.push("New configuration".to_string());

    let Some(idx) = prompt_select(
        "You have existing nodes for this provider. Reuse connection settings?",
        &options,
    )?
    else {
        return Ok(None);
    };

    if idx == existing.len() {
        // User chose "New configuration"
        return Ok(None);
    }

    let (_, node) = existing[idx];
    Ok(Some(ReusedConnection {
        auth: node.auth.clone(),
        endpoint: node.endpoint.clone(),
    }))
}

/// Fetch models from a provider API and let the user select one.
/// Falls back to manual text input on timeout or error.
async fn select_model(
    models_future: impl Future<Output = Result<Vec<String>>>,
    default: &str,
) -> Result<String> {
    let result = tokio::time::timeout(Duration::from_secs(5), models_future).await;

    match result {
        Ok(Ok(mut models)) if !models.is_empty() => {
            models.sort();
            let manual_option = "[ Enter manually ]".to_string();
            models.push(manual_option.clone());

            match inquire::Select::new("Select model:", models).prompt() {
                Ok(selected) if selected == manual_option => {
                    Ok(Text::new("Model:").with_default(default).prompt()?)
                }
                Ok(selected) => Ok(selected),
                Err(
                    inquire::InquireError::OperationCanceled
                    | inquire::InquireError::OperationInterrupted,
                ) => Ok(default.to_string()),
                Err(e) => Err(e.into()),
            }
        }
        Ok(Ok(_)) => {
            println!(
                "{}",
                "No models found from the API. Enter model name manually.".dimmed()
            );
            Ok(Text::new("Model:").with_default(default).prompt()?)
        }
        Ok(Err(e)) => {
            println!(
                "{} {}",
                "Could not fetch models:".dimmed(),
                format!("{:#}", e).dimmed()
            );
            Ok(Text::new("Model:").with_default(default).prompt()?)
        }
        Err(_) => {
            println!(
                "{}",
                "Timed out fetching models. Enter model name manually.".dimmed()
            );
            Ok(Text::new("Model:").with_default(default).prompt()?)
        }
    }
}

/// Prompt user to select capabilities for a node.
/// Only shows capabilities the provider supports.
fn prompt_capabilities(provider: &ProviderKind) -> Result<Vec<Capability>> {
    let supported = provider.supported_capabilities();

    // Skip prompt when there's only one possible capability
    if supported.len() <= 1 {
        return Ok(supported);
    }

    let labels: Vec<String> = supported.iter().map(|c| c.label().to_string()).collect();

    match inquire::MultiSelect::new("What can this model do?", labels.clone()).prompt() {
        Ok(selected) => {
            let caps = selected
                .iter()
                .filter_map(|s| {
                    labels
                        .iter()
                        .position(|l| l == s)
                        .map(|i| supported[i].clone())
                })
                .collect();
            Ok(caps)
        }
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

pub async fn run(cmd: NodeCommands) -> Result<()> {
    match cmd {
        NodeCommands::List => run_list(),
        NodeCommands::Add => run_add().await,
        NodeCommands::Edit { id } => run_edit(&id),
        NodeCommands::Remove { id } => run_remove(&id),
        NodeCommands::Default {
            capability,
            node_id,
        } => run_default(&capability, node_id.as_deref()),
        NodeCommands::Show { id } => run_show(&id),
    }
}

fn run_list() -> Result<()> {
    let config = Config::load()?;

    if config.nodes.is_empty() {
        println!("{}", "No nodes configured.".dimmed());
        println!("Run {} to add one.", "ailloy nodes add".bold());
        println!();
        println!("{}", "Available Provider Types".bold());
        println!();
        println!("  {} — OpenAI API (GPT-4o, etc.)", "openai".bold());
        println!(
            "  {} — Anthropic API (Claude Sonnet, etc.)",
            "anthropic".bold()
        );
        println!("  {} — Azure OpenAI Service", "azure-openai".bold());
        println!(
            "  {} — Microsoft Foundry (GPT, Llama, Mistral, etc.)",
            "microsoft-foundry".bold()
        );
        println!("  {} — Google Vertex AI (Gemini, etc.)", "vertex-ai".bold());
        println!("  {} — Local LLMs via Ollama", "ollama".bold());
        println!("  {} — Local LLMs via LM Studio", "lm-studio".bold());
        println!(
            "  {} — CLI agents (Claude, Codex, Copilot)",
            "local-agent".bold()
        );
        return Ok(());
    }

    println!("{}", "Configured Nodes".bold());
    println!();

    for &(cap_key, cap_label) in ALL_CAPABILITIES {
        let default_id = config.defaults.get(cap_key);
        let cap: Capability = cap_key.parse().unwrap();
        let nodes: Vec<_> = config.nodes_for_capability(&cap);

        if nodes.is_empty() {
            continue;
        }

        println!("  {} {}", cap_label.bold(), "Nodes:".dimmed());

        for (id, node) in &nodes {
            let is_default = default_id.is_some_and(|d| d == *id);
            let marker = if is_default {
                " (default)".green().to_string()
            } else {
                String::new()
            };
            let alias = node
                .alias
                .as_ref()
                .map(|a| format!(" [{}]", a))
                .unwrap_or_default();

            println!(
                "    {} ({}, {}){}{}",
                id.bold(),
                node.provider.to_string().dimmed(),
                node.detail(),
                alias.dimmed(),
                marker,
            );
        }
        println!();
    }

    // Show nodes that don't match any capability
    let uncategorized: Vec<_> = config
        .nodes
        .iter()
        .filter(|(_, n)| n.capabilities.is_empty())
        .collect();
    if !uncategorized.is_empty() {
        println!("  {} {}", "Uncategorized".bold(), "Nodes:".dimmed());
        for (id, node) in &uncategorized {
            println!(
                "    {} ({}, {})",
                id.bold(),
                node.provider.to_string().dimmed(),
                node.detail(),
            );
        }
        println!();
    }

    Ok(())
}

async fn run_add() -> Result<()> {
    let mut config = Config::load_global()?;

    let Some((suggested_id, node)) = prompt_node_setup(&config).await? else {
        return Ok(()); // User cancelled
    };

    let id = Text::new("Node ID:").with_default(&suggested_id).prompt()?;

    if config.nodes.contains_key(&id)
        && !prompt_confirm(&format!("Node '{}' already exists. Overwrite?", id))?
    {
        return Ok(());
    }

    let caps = node.capabilities.clone();
    config.add_node(id.clone(), node);

    // Auto-set as default for capabilities where no default exists
    for cap in &caps {
        let cap_key = cap.config_key();
        if !config.defaults.contains_key(cap_key) {
            config.set_default(cap_key, &id);
        }
    }

    config.save()?;
    println!("{} Added node '{}'", "✓".green().bold(), id.bold());
    Ok(())
}

fn run_edit(id_or_alias: &str) -> Result<()> {
    let mut config = Config::load_global()?;
    let canonical_id = config
        .resolve_node(id_or_alias)
        .map(|s| s.to_string())
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;

    let node = config.get_node_mut(&canonical_id).unwrap();

    loop {
        let fields = editable_fields(node);
        let mut options: Vec<String> = fields
            .iter()
            .map(|(label, val)| format!("{}: {}", label, val.as_deref().unwrap_or("(not set)")))
            .collect();
        options.push("Done editing".to_string());

        let selected = prompt_select(&format!("Edit: {}", canonical_id), &options)?;

        let Some(idx) = selected else {
            break;
        };

        if idx == options.len() - 1 {
            break;
        }

        let field_name = fields[idx].0;
        edit_field(node, field_name)?;
    }

    config.save()?;
    Ok(())
}

fn run_remove(id_or_alias: &str) -> Result<()> {
    let mut config = Config::load_global()?;
    let canonical_id = config
        .resolve_node(id_or_alias)
        .map(|s| s.to_string())
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;

    if !prompt_confirm(&format!("Remove node '{}'?", canonical_id))? {
        return Ok(());
    }

    config.remove_node(&canonical_id);
    config.save()?;
    println!("{} Removed node '{}'", "✓".green().bold(), canonical_id);
    Ok(())
}

fn run_default(capability: &str, node_id: Option<&str>) -> Result<()> {
    let mut config = Config::load_global()?;

    // Validate capability
    let _: Capability = capability.parse().map_err(|e: String| anyhow::anyhow!(e))?;

    if let Some(id) = node_id {
        // Set default
        let canonical = config
            .resolve_node(id)
            .map(|s| s.to_string())
            .with_context(|| format!("Node '{}' not found", id))?;
        config.set_default(capability, &canonical);
        config.save()?;
        println!(
            "{} Default for '{}': {}",
            "✓".green().bold(),
            capability,
            canonical.bold()
        );
    } else {
        // Show current default
        match config.defaults.get(capability) {
            Some(id) => println!("{}: {}", capability, id.bold()),
            None => println!("{}: {}", capability, "(not set)".dimmed()),
        }
    }

    Ok(())
}

fn run_show(id_or_alias: &str) -> Result<()> {
    let config = Config::load()?;
    let (canonical_id, node) = config
        .get_node(id_or_alias)
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;

    println!("{}", canonical_id.bold());
    println!("  {} {}", "Provider:".dimmed(), node.provider);
    if let Some(alias) = &node.alias {
        println!("  {} {}", "Alias:".dimmed(), alias);
    }
    if !node.capabilities.is_empty() {
        let caps: Vec<_> = node.capabilities.iter().map(|c| c.to_string()).collect();
        println!("  {} {}", "Capabilities:".dimmed(), caps.join(", "));
    }
    if let Some(model) = &node.model {
        println!("  {} {}", "Model:".dimmed(), model);
    }
    if let Some(endpoint) = &node.endpoint {
        println!("  {} {}", "Endpoint:".dimmed(), endpoint);
    }
    if let Some(deployment) = &node.deployment {
        println!("  {} {}", "Deployment:".dimmed(), deployment);
    }
    if let Some(api_version) = &node.api_version {
        println!("  {} {}", "API version:".dimmed(), api_version);
    }
    if let Some(binary) = &node.binary {
        println!("  {} {}", "Binary:".dimmed(), binary);
    }
    if let Some(project) = &node.project {
        println!("  {} {}", "Project:".dimmed(), project);
    }
    if let Some(location) = &node.location {
        println!("  {} {}", "Location:".dimmed(), location);
    }
    match &node.auth {
        Some(Auth::Env(var)) => println!("  {} env: {}", "Auth:".dimmed(), var),
        Some(Auth::ApiKey(_)) => println!("  {} api_key: ********", "Auth:".dimmed()),
        Some(Auth::AzureCli(_)) => println!("  {} azure_cli", "Auth:".dimmed()),
        Some(Auth::GcloudCli(_)) => println!("  {} gcloud_cli", "Auth:".dimmed()),
        None => {}
    }

    // Show if this node is a default for any capability
    for (cap, default_id) in &config.defaults {
        if default_id == canonical_id {
            println!("  {} default for '{}'", "★".green().bold(), cap);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Interactive node setup
// ---------------------------------------------------------------------------

/// All provider kinds with their display labels.
/// LM Studio uses the OpenAI-compatible API so it maps to OpenAi provider kind.
const PROVIDER_KINDS: &[(&str, ProviderKind)] = &[
    ("OpenAI", ProviderKind::OpenAi),
    ("Anthropic", ProviderKind::Anthropic),
    ("Azure OpenAI", ProviderKind::AzureOpenAi),
    ("Microsoft Foundry", ProviderKind::MicrosoftFoundry),
    ("Google Vertex AI", ProviderKind::VertexAi),
    ("Ollama", ProviderKind::Ollama),
    ("LM Studio", ProviderKind::OpenAi),
    (
        "Local Agent (Claude, Codex, Copilot)",
        ProviderKind::LocalAgent,
    ),
];

/// Returns None if user cancelled.
async fn prompt_node_setup(config: &Config) -> Result<Option<(String, AiNode)>> {
    prompt_node_setup_filtered(config, None).await
}

/// Returns None if user cancelled. Optionally filter provider kinds by task.
pub async fn prompt_node_setup_filtered(
    config: &Config,
    task_filter: Option<&str>,
) -> Result<Option<(String, AiNode)>> {
    let kind_options: Vec<(&str, &ProviderKind)> = PROVIDER_KINDS
        .iter()
        .filter(|(_, kind)| task_filter.map(|t| kind.supports_task(t)).unwrap_or(true))
        .map(|(label, kind)| (*label, kind))
        .collect();

    let labels: Vec<String> = kind_options.iter().map(|(l, _)| l.to_string()).collect();

    let Some(idx) = prompt_select("Select AI provider", &labels)? else {
        return Ok(None);
    };

    let kind_label = kind_options[idx].0;
    prompt_node_for_kind(config, kind_label).await
}

/// Prompt for provider-specific fields given a pre-selected provider label.
/// Returns None if user cancelled.
pub async fn prompt_node_for_kind(
    config: &Config,
    kind_label: &str,
) -> Result<Option<(String, AiNode)>> {
    match kind_label {
        "OpenAI" => {
            let (auth, endpoint_opt) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::OpenAi, Some("openai/"))?
            {
                (reused.auth, reused.endpoint)
            } else {
                let api_key = Text::new("API key:")
                    .with_help_message("Or set OPENAI_API_KEY env var")
                    .prompt()?;
                let endpoint = Text::new("Endpoint (leave empty for default):")
                    .with_default("")
                    .prompt()?;

                let auth = if api_key.is_empty() {
                    Some(Auth::Env("OPENAI_API_KEY".to_string()))
                } else {
                    Some(Auth::ApiKey(api_key))
                };
                let endpoint_opt = if endpoint.is_empty() {
                    None
                } else {
                    Some(endpoint)
                };
                (auth, endpoint_opt)
            };

            let effective_key = match &auth {
                Some(Auth::ApiKey(k)) => k.clone(),
                Some(Auth::Env(var)) => std::env::var(var).unwrap_or_default(),
                _ => String::new(),
            };

            let model = if !effective_key.is_empty() {
                let client =
                    ailloy::openai::OpenAiClient::new(&effective_key, "_", endpoint_opt.clone());
                select_model(client.list_models(), "gpt-4o").await?
            } else {
                Text::new("Model:").with_default("gpt-4o").prompt()?
            };

            let capabilities = prompt_capabilities(&ProviderKind::OpenAi)?;

            let node = AiNode {
                provider: ProviderKind::OpenAi,
                alias: None,
                capabilities,
                auth,
                model: Some(model.clone()),
                endpoint: endpoint_opt,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("openai/{}", model), node)))
        }
        "Anthropic" => {
            let auth = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::Anthropic, None)?
            {
                reused.auth
            } else {
                let api_key = Text::new("API key:")
                    .with_help_message("Or set ANTHROPIC_API_KEY env var")
                    .prompt()?;

                if api_key.is_empty() {
                    Some(Auth::Env("ANTHROPIC_API_KEY".to_string()))
                } else {
                    Some(Auth::ApiKey(api_key))
                }
            };

            let effective_key = match &auth {
                Some(Auth::ApiKey(k)) => k.clone(),
                Some(Auth::Env(var)) => std::env::var(var).unwrap_or_default(),
                _ => String::new(),
            };

            let model = if !effective_key.is_empty() {
                let client = ailloy::anthropic::AnthropicClient::new(&effective_key, "_");
                select_model(client.list_models(), "claude-sonnet-4-6").await?
            } else {
                Text::new("Model:")
                    .with_default("claude-sonnet-4-6")
                    .prompt()?
            };

            let capabilities = prompt_capabilities(&ProviderKind::Anthropic)?;

            let node = AiNode {
                provider: ProviderKind::Anthropic,
                alias: None,
                capabilities,
                auth,
                model: Some(model.clone()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("anthropic/{}", model), node)))
        }
        "Azure OpenAI" => {
            let (auth, endpoint, api_version) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::AzureOpenAi, None)?
            {
                let ep = reused
                    .endpoint
                    .unwrap_or_else(|| "https://my-instance.openai.azure.com".to_string());
                (reused.auth.unwrap_or(Auth::AzureCli(true)), ep, None)
            } else {
                let endpoint =
                    Text::new("Endpoint (e.g. https://my-instance.openai.azure.com):").prompt()?;
                let api_version = Text::new("API version:")
                    .with_default("2025-04-01-preview")
                    .prompt()?;

                let auth_options: Vec<String> = vec![
                    "Azure CLI (az login) (Recommended)".into(),
                    "API Key".into(),
                ];
                let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
                    return Ok(None);
                };

                let auth = if auth_idx == 1 {
                    let key = Text::new("API key:").prompt()?;
                    if key.is_empty() {
                        Auth::AzureCli(true)
                    } else {
                        Auth::ApiKey(key)
                    }
                } else {
                    Auth::AzureCli(true)
                };
                (auth, endpoint, Some(api_version))
            };

            let deployment = Text::new("Deployment name:").prompt()?;
            let capabilities = prompt_capabilities(&ProviderKind::AzureOpenAi)?;

            let node = AiNode {
                provider: ProviderKind::AzureOpenAi,
                alias: None,
                capabilities,
                auth: Some(auth),
                model: None,
                endpoint: Some(endpoint),
                deployment: Some(deployment.clone()),
                api_version: api_version.or(Some("2025-04-01-preview".to_string())),
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("azure-openai/{}", deployment), node)))
        }
        "Microsoft Foundry" => {
            let (auth, endpoint, api_version) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::MicrosoftFoundry, None)?
            {
                let ep = reused
                    .endpoint
                    .unwrap_or_else(|| "https://my-instance.services.ai.azure.com".to_string());
                (reused.auth.unwrap_or(Auth::AzureCli(true)), ep, None)
            } else {
                let endpoint =
                    Text::new("Endpoint (e.g. https://my-instance.services.ai.azure.com):")
                        .prompt()?;
                let api_version = Text::new("API version:")
                    .with_default("2024-05-01-preview")
                    .prompt()?;

                let auth_options: Vec<String> = vec![
                    "Azure CLI (az login) (Recommended)".into(),
                    "API Key".into(),
                ];
                let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
                    return Ok(None);
                };

                let auth = if auth_idx == 1 {
                    let key = Text::new("API key:").prompt()?;
                    if key.is_empty() {
                        Auth::AzureCli(true)
                    } else {
                        Auth::ApiKey(key)
                    }
                } else {
                    Auth::AzureCli(true)
                };
                (auth, endpoint, Some(api_version))
            };

            let model = Text::new("Model (e.g. gpt-4o):").prompt()?;
            let capabilities = prompt_capabilities(&ProviderKind::MicrosoftFoundry)?;

            let node = AiNode {
                provider: ProviderKind::MicrosoftFoundry,
                alias: None,
                capabilities,
                auth: Some(auth),
                model: Some(model.clone()),
                endpoint: Some(endpoint),
                deployment: None,
                api_version: api_version.or(Some("2024-05-01-preview".to_string())),
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("microsoft-foundry/{}", model), node)))
        }
        "Google Vertex AI" => {
            // Vertex AI uses gcloud CLI auth and project/location instead of endpoint.
            // Check for existing Vertex nodes to reuse project/location.
            let existing: Vec<(&String, &AiNode)> = config
                .nodes
                .iter()
                .filter(|(_, n)| n.provider == ProviderKind::VertexAi)
                .collect();

            let (project, location) = if !existing.is_empty() {
                let mut options: Vec<String> = existing
                    .iter()
                    .map(|(id, node)| {
                        format!(
                            "{} (project: {}, location: {})",
                            id,
                            node.project.as_deref().unwrap_or("?"),
                            node.location.as_deref().unwrap_or("?"),
                        )
                    })
                    .collect();
                options.push("New configuration".to_string());

                let selected = prompt_select(
                    "You have existing Vertex AI nodes. Reuse project settings?",
                    &options,
                )?;

                if let Some(idx) = selected {
                    if idx < existing.len() {
                        let (_, node) = existing[idx];
                        (
                            node.project.clone().unwrap_or_default(),
                            node.location
                                .clone()
                                .unwrap_or_else(|| "us-central1".to_string()),
                        )
                    } else {
                        let project = Text::new("GCP project:").prompt()?;
                        let location = Text::new("Location:")
                            .with_default("us-central1")
                            .prompt()?;
                        (project, location)
                    }
                } else {
                    let project = Text::new("GCP project:").prompt()?;
                    let location = Text::new("Location:")
                        .with_default("us-central1")
                        .prompt()?;
                    (project, location)
                }
            } else {
                let project = Text::new("GCP project:").prompt()?;
                let location = Text::new("Location:")
                    .with_default("us-central1")
                    .prompt()?;
                (project, location)
            };

            let model = Text::new("Model:")
                .with_default("gemini-3.1-pro")
                .prompt()?;
            let capabilities = prompt_capabilities(&ProviderKind::VertexAi)?;

            let node = AiNode {
                provider: ProviderKind::VertexAi,
                alias: None,
                capabilities,
                auth: Some(Auth::GcloudCli(true)),
                model: Some(model.clone()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: Some(project),
                location: Some(location),
                node_defaults: None,
            };
            Ok(Some((format!("vertex-ai/{}", model), node)))
        }
        "Ollama" => {
            let endpoint_opt = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::Ollama, None)?
            {
                reused.endpoint
            } else {
                let endpoint = Text::new("Endpoint:")
                    .with_default("http://localhost:11434")
                    .prompt()?;

                if endpoint == "http://localhost:11434" {
                    None
                } else {
                    Some(endpoint)
                }
            };

            let client = ailloy::ollama::OllamaClient::new("_", endpoint_opt.clone());
            let model = select_model(client.list_models(), "llama3.2").await?;
            let capabilities = prompt_capabilities(&ProviderKind::Ollama)?;

            let node = AiNode {
                provider: ProviderKind::Ollama,
                alias: None,
                capabilities,
                auth: None,
                model: Some(model.clone()),
                endpoint: endpoint_opt,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("ollama/{}", model), node)))
        }
        "LM Studio" => {
            let (auth, endpoint_opt) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::OpenAi, Some("lm-studio/"))?
            {
                (reused.auth, reused.endpoint)
            } else {
                let endpoint = Text::new("Endpoint:")
                    .with_default("http://localhost:1234")
                    .prompt()?;
                let api_key = Text::new("API key (leave empty if not required):")
                    .with_default("")
                    .prompt()?;

                let auth = if api_key.is_empty() {
                    None
                } else {
                    Some(Auth::ApiKey(api_key))
                };
                (auth, Some(endpoint))
            };

            let effective_key = match &auth {
                Some(Auth::ApiKey(k)) => k.clone(),
                Some(Auth::Env(var)) => std::env::var(var).unwrap_or_default(),
                _ => String::new(),
            };

            // LM Studio always needs an endpoint for model listing
            let client = ailloy::openai::OpenAiClient::new(
                if effective_key.is_empty() {
                    "lm-studio"
                } else {
                    &effective_key
                },
                "_",
                endpoint_opt.clone(),
            );
            let model = select_model(client.list_models(), "default").await?;
            let capabilities = prompt_capabilities(&ProviderKind::OpenAi)?;

            let node = AiNode {
                provider: ProviderKind::OpenAi,
                alias: None,
                capabilities,
                auth,
                model: Some(model.clone()),
                endpoint: endpoint_opt,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("lm-studio/{}", model), node)))
        }
        _ => {
            // Local Agent
            let binary_options: Vec<String> =
                vec!["claude".into(), "codex".into(), "copilot".into()];
            let Some(idx) = prompt_select("Select agent", &binary_options)? else {
                return Ok(None);
            };
            let binary = &binary_options[idx];

            let capabilities = prompt_capabilities(&ProviderKind::LocalAgent)?;

            let node = AiNode {
                provider: ProviderKind::LocalAgent,
                alias: None,
                capabilities,
                auth: None,
                model: None,
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: Some(binary.to_string()),
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("local-agent/{}", binary), node)))
        }
    }
}

// ---------------------------------------------------------------------------
// Field editing
// ---------------------------------------------------------------------------

fn editable_fields(node: &AiNode) -> Vec<(&'static str, Option<String>)> {
    let auth_display = match &node.auth {
        Some(Auth::Env(v)) => Some(format!("env: {}", v)),
        Some(Auth::ApiKey(_)) => Some("api_key: ********".to_string()),
        Some(Auth::AzureCli(_)) => Some("azure_cli".to_string()),
        Some(Auth::GcloudCli(_)) => Some("gcloud_cli".to_string()),
        None => None,
    };

    let caps_display = if node.capabilities.is_empty() {
        None
    } else {
        Some(
            node.capabilities
                .iter()
                .map(|c| c.config_key())
                .collect::<Vec<_>>()
                .join(", "),
        )
    };

    let mut fields = match node.provider {
        ProviderKind::OpenAi => vec![
            ("Model", node.model.clone()),
            ("Endpoint", node.endpoint.clone()),
            ("Auth", auth_display),
        ],
        ProviderKind::Anthropic => vec![("Model", node.model.clone()), ("Auth", auth_display)],
        ProviderKind::AzureOpenAi => vec![
            ("Endpoint", node.endpoint.clone()),
            ("Deployment", node.deployment.clone()),
            ("API version", node.api_version.clone()),
            ("Auth", auth_display),
        ],
        ProviderKind::MicrosoftFoundry => vec![
            ("Endpoint", node.endpoint.clone()),
            ("Model", node.model.clone()),
            ("API version", node.api_version.clone()),
            ("Auth", auth_display),
        ],
        ProviderKind::VertexAi => vec![
            ("Project", node.project.clone()),
            ("Location", node.location.clone()),
            ("Model", node.model.clone()),
        ],
        ProviderKind::Ollama => vec![
            ("Model", node.model.clone()),
            ("Endpoint", node.endpoint.clone()),
        ],
        ProviderKind::LocalAgent => vec![("Binary", node.binary.clone())],
    };

    fields.push(("Capabilities", caps_display));
    fields
}

fn edit_field(node: &mut AiNode, field: &str) -> Result<()> {
    if field == "Auth" {
        return edit_auth(node);
    }
    if field == "Capabilities" {
        node.capabilities = prompt_capabilities(&node.provider)?;
        return Ok(());
    }

    let current = match field {
        "Model" => node.model.as_deref().unwrap_or(""),
        "Endpoint" => node.endpoint.as_deref().unwrap_or(""),
        "Deployment" => node.deployment.as_deref().unwrap_or(""),
        "API version" => node.api_version.as_deref().unwrap_or(""),
        "Binary" => node.binary.as_deref().unwrap_or(""),
        "Project" => node.project.as_deref().unwrap_or(""),
        "Location" => node.location.as_deref().unwrap_or(""),
        _ => return Ok(()),
    };

    let label = format!("{}:", field);
    let mut prompt =
        Text::new(&label).with_help_message("Enter new value, or leave empty to clear");
    if !current.is_empty() {
        prompt = prompt.with_default(current);
    }
    let value = prompt.prompt()?;
    let opt = if value.is_empty() { None } else { Some(value) };

    match field {
        "Model" => node.model = opt,
        "Endpoint" => node.endpoint = opt,
        "Deployment" => node.deployment = opt,
        "API version" => node.api_version = opt,
        "Binary" => node.binary = opt,
        "Project" => node.project = opt,
        "Location" => node.location = opt,
        _ => {}
    }

    Ok(())
}

fn edit_auth(node: &mut AiNode) -> Result<()> {
    let options: Vec<String> = vec![
        "Environment variable".into(),
        "API key (inline)".into(),
        "Azure CLI".into(),
        "gcloud CLI".into(),
        "None (clear)".into(),
    ];

    let Some(idx) = prompt_select("Authentication method", &options)? else {
        return Ok(());
    };

    node.auth = match idx {
        0 => {
            let var = Text::new("Environment variable:")
                .with_default("OPENAI_API_KEY")
                .prompt()?;
            Some(Auth::Env(var))
        }
        1 => {
            let key = Text::new("API key:").prompt()?;
            if key.is_empty() {
                None
            } else {
                Some(Auth::ApiKey(key))
            }
        }
        2 => Some(Auth::AzureCli(true)),
        3 => Some(Auth::GcloudCli(true)),
        _ => None,
    };

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(provider: ProviderKind, auth: Option<Auth>, endpoint: Option<&str>) -> AiNode {
        AiNode {
            provider,
            alias: None,
            capabilities: vec![],
            auth,
            model: Some("test-model".to_string()),
            endpoint: endpoint.map(|s| s.to_string()),
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        }
    }

    fn config_with_nodes(nodes: Vec<(&str, AiNode)>) -> Config {
        let mut config = Config::default();
        for (id, node) in nodes {
            config.add_node(id.to_string(), node);
        }
        config
    }

    #[test]
    fn test_find_reusable_nodes_empty_config() {
        let config = Config::default();
        let nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_find_reusable_nodes_matches_provider() {
        let config = config_with_nodes(vec![
            (
                "openai/gpt-4o",
                make_node(
                    ProviderKind::OpenAi,
                    Some(Auth::ApiKey("sk-123".into())),
                    None,
                ),
            ),
            (
                "anthropic/claude",
                make_node(
                    ProviderKind::Anthropic,
                    Some(Auth::ApiKey("ak-456".into())),
                    None,
                ),
            ),
        ]);

        let nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].0, "openai/gpt-4o");
    }

    #[test]
    fn test_find_reusable_nodes_openai_prefix_excludes_lm_studio() {
        let config = config_with_nodes(vec![
            (
                "openai/gpt-4o",
                make_node(
                    ProviderKind::OpenAi,
                    Some(Auth::ApiKey("sk-123".into())),
                    None,
                ),
            ),
            (
                "lm-studio/llama",
                make_node(ProviderKind::OpenAi, None, Some("http://localhost:1234")),
            ),
        ]);

        // OpenAI prefix should only find openai/ nodes
        let openai_nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, Some("openai/"));
        assert_eq!(openai_nodes.len(), 1);
        assert_eq!(openai_nodes[0].0, "openai/gpt-4o");

        // LM Studio prefix should only find lm-studio/ nodes
        let lm_nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, Some("lm-studio/"));
        assert_eq!(lm_nodes.len(), 1);
        assert_eq!(lm_nodes[0].0, "lm-studio/llama");
    }

    #[test]
    fn test_find_reusable_nodes_no_prefix_finds_all() {
        let config = config_with_nodes(vec![
            (
                "openai/gpt-4o",
                make_node(
                    ProviderKind::OpenAi,
                    Some(Auth::ApiKey("sk-123".into())),
                    None,
                ),
            ),
            (
                "lm-studio/llama",
                make_node(ProviderKind::OpenAi, None, Some("http://localhost:1234")),
            ),
        ]);

        // No prefix finds all OpenAi-kind nodes
        let all_nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert_eq!(all_nodes.len(), 2);
    }

    #[test]
    fn test_find_reusable_nodes_prefix_no_match() {
        let config = config_with_nodes(vec![(
            "openai/gpt-4o",
            make_node(
                ProviderKind::OpenAi,
                Some(Auth::ApiKey("sk-123".into())),
                None,
            ),
        )]);

        let nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, Some("lm-studio/"));
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_find_reusable_nodes_different_provider_not_matched() {
        let config = config_with_nodes(vec![(
            "ollama/llama",
            make_node(ProviderKind::Ollama, None, None),
        )]);

        let nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_auth_summary_api_key() {
        let node = make_node(
            ProviderKind::OpenAi,
            Some(Auth::ApiKey("sk-123".into())),
            None,
        );
        assert_eq!(auth_summary(&node), "api_key: ********");
    }

    #[test]
    fn test_auth_summary_env() {
        let node = make_node(
            ProviderKind::OpenAi,
            Some(Auth::Env("OPENAI_API_KEY".into())),
            None,
        );
        assert_eq!(auth_summary(&node), "env: OPENAI_API_KEY");
    }

    #[test]
    fn test_auth_summary_with_endpoint() {
        let node = make_node(
            ProviderKind::OpenAi,
            Some(Auth::ApiKey("sk-123".into())),
            Some("http://localhost:1234"),
        );
        assert_eq!(
            auth_summary(&node),
            "api_key: ********, endpoint: http://localhost:1234"
        );
    }

    #[test]
    fn test_auth_summary_none() {
        let node = make_node(ProviderKind::Ollama, None, None);
        assert_eq!(auth_summary(&node), "none");
    }
}
