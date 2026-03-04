use anyhow::{Context, Result};
use colored::Colorize;

use ailloy::config::{
    ALL_CAPABILITIES, AiNode, Auth, Capability, Config, ProviderKind, consent_keys,
};

use crate::cli::ConfigCommands;
use crate::commands::azure_discover;
use crate::commands::consent;
use crate::commands::nodes;

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

/// Select with a pre-selected default index.
fn prompt_select_with_default(
    message: &str,
    options: &[String],
    default: usize,
) -> Result<Option<usize>> {
    match inquire::Select::new(message, options.to_vec())
        .with_starting_cursor(default)
        .prompt()
    {
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

fn print_nodes_summary(config: &Config) {
    if config.nodes.is_empty() {
        println!("{}", "No nodes configured.".dimmed());
        return;
    }

    println!("{}", "Current nodes:".bold());
    for (id, node) in &config.nodes {
        let is_default = config.defaults.values().any(|d| d == id);
        let marker = if is_default { " (default)" } else { "" };
        let caps: Vec<_> = node.capabilities.iter().map(|c| c.to_string()).collect();
        let caps_str = if caps.is_empty() {
            String::new()
        } else {
            format!(" [{}]", caps.join(", "))
        };
        println!(
            "  {} ({}, {}){}{}",
            id.bold(),
            node.provider.to_string().dimmed(),
            node.detail(),
            caps_str.dimmed(),
            marker.dimmed(),
        );
    }

    if !config.defaults.is_empty() {
        println!();
        println!("{}", "Defaults:".bold());
        for (task, node_id) in &config.defaults {
            println!("  {} {}", format!("{}:", task).dimmed(), node_id);
        }
    }
}

pub async fn run(cmd: ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::Init => unreachable!("Init dispatched from main.rs"),
        ConfigCommands::Show => run_show(),
        ConfigCommands::Set { key, value } => run_set(&key, &value),
        ConfigCommands::Get { key } => run_get(&key),
        ConfigCommands::Unset { key } => run_unset(&key),
    }
}

// --- Interactive config ---

pub async fn run_interactive() -> Result<()> {
    let mut config = Config::load_global()?;

    loop {
        println!();
        print_nodes_summary(&config);
        println!();

        let has_nodes = !config.nodes.is_empty();
        let mut actions = vec!["Add a node"];
        if has_nodes {
            actions.push("Edit a node");
            actions.push("Remove a node");
            actions.push("Set a default");
        }
        actions.push("Quit");

        let action = match inquire::Select::new("What would you like to do?", actions).prompt() {
            Ok(a) => a,
            Err(
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted,
            ) => break,
            Err(e) => return Err(e.into()),
        };

        match action {
            "Add a node" => {
                if let Some(name) = add_node(&mut config, None).await? {
                    println!("{} Added node '{}'", "✓".green().bold(), name.bold());
                }
            }
            "Edit a node" => {
                let ids: Vec<String> = config.nodes.keys().cloned().collect();
                let Some(idx) = prompt_select("Select node to edit", &ids)? else {
                    continue;
                };
                let id = ids[idx].clone();
                edit_node_fields(&mut config, &id)?;
                config.save()?;
                println!("{} Updated node '{}'", "✓".green().bold(), id.bold());
            }
            "Remove a node" => {
                let ids: Vec<String> = config.nodes.keys().cloned().collect();
                let Some(idx) = prompt_select("Select node to remove", &ids)? else {
                    continue;
                };
                let id = ids[idx].clone();
                if prompt_confirm(&format!("Delete node '{}'?", id))? {
                    config.remove_node(&id);
                    config.save()?;
                    println!("{} Removed node '{}'", "✓".green().bold(), id.bold());
                }
            }
            "Set a default" => {
                let cap_labels: Vec<String> = ALL_CAPABILITIES
                    .iter()
                    .map(|(key, label)| format!("{} ({})", label, key))
                    .collect();
                let Some(cap_idx) = prompt_select("Select capability", &cap_labels)? else {
                    continue;
                };
                let cap_key = ALL_CAPABILITIES[cap_idx].0;

                let cap: Capability = cap_key.parse().unwrap();
                let cap_nodes: Vec<_> = config.nodes_for_capability(&cap);
                if cap_nodes.is_empty() {
                    println!("No nodes with '{}' capability.", cap_key);
                    continue;
                }

                let node_labels: Vec<String> =
                    cap_nodes.iter().map(|(id, _)| id.to_string()).collect();
                let Some(node_idx) = prompt_select("Select default node", &node_labels)? else {
                    continue;
                };
                let selected_node = &node_labels[node_idx];
                config.set_default(cap_key, selected_node);
                config.save()?;
                println!(
                    "{} Default for '{}': {}",
                    "✓".green().bold(),
                    cap_key,
                    selected_node.bold()
                );
            }
            "Quit" => break,
            _ => unreachable!(),
        }
    }

    Ok(())
}

fn edit_node_fields(config: &mut Config, id: &str) -> Result<()> {
    let node = config.get_node_mut(id).unwrap();

    loop {
        let fields = editable_fields(node);
        let mut options: Vec<String> = fields
            .iter()
            .map(|(label, val)| format!("{}: {}", label, val.as_deref().unwrap_or("(not set)")))
            .collect();
        options.push("Done editing".to_string());

        let selected = prompt_select(&format!("Edit: {}", id), &options)?;

        let Some(idx) = selected else {
            break;
        };

        if idx == options.len() - 1 {
            break;
        }

        let field_name = fields[idx].0;
        edit_field(node, field_name)?;
    }

    Ok(())
}

// --- Add Node ---

async fn add_node(config: &mut Config, task_filter: Option<&str>) -> Result<Option<String>> {
    let Some((suggested_id, node)) = add_node_interactive(config, task_filter).await? else {
        return Ok(None);
    };

    let name = inquire::Text::new("Node ID:")
        .with_default(&suggested_id)
        .prompt()?;

    if config.nodes.contains_key(&name)
        && !prompt_confirm(&format!("Node '{}' already exists. Overwrite?", name))?
    {
        return Ok(None);
    }

    let caps = node.capabilities.clone();
    config.add_node(name.clone(), node);

    // Auto-set as default for capabilities where no default exists
    for cap in &caps {
        let cap_key = cap.config_key();
        if !config.defaults.contains_key(cap_key) {
            config.set_default(cap_key, &name);
        }
    }

    config.save()?;

    Ok(Some(name))
}

/// Interactive node setup for the config wizard. Handles Azure/Foundry discovery flows.
async fn add_node_interactive(
    config: &mut Config,
    task_filter: Option<&str>,
) -> Result<Option<(String, AiNode)>> {
    // For Azure/Foundry, we need special handling for discovery
    let kind_options: Vec<(&str, ProviderKind)> = [
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
    ]
    .iter()
    .filter(|(_, kind)| task_filter.map(|t| kind.supports_task(t)).unwrap_or(true))
    .cloned()
    .collect();

    let labels: Vec<String> = kind_options.iter().map(|(l, _)| l.to_string()).collect();

    let Some(idx) = prompt_select("Select AI provider", &labels)? else {
        return Ok(None);
    };

    let kind_label = kind_options[idx].0;

    match kind_label {
        "Azure OpenAI" => {
            let allowed = consent::ensure_consent(
                config,
                consent_keys::AZURE_CLI,
                "the Azure CLI (az)",
                "automatically discover your subscriptions, resources, and deployments",
            )?;

            if allowed {
                match azure_discover_flow().await {
                    Ok(result) => Ok(Some(result)),
                    Err(e) => {
                        eprintln!(
                            "{} Auto-discovery failed: {}",
                            "Warning:".yellow().bold(),
                            e
                        );
                        eprintln!("Falling back to manual configuration.\n");
                        nodes::prompt_node_for_kind(config, kind_label).await
                    }
                }
            } else {
                nodes::prompt_node_for_kind(config, kind_label).await
            }
        }
        "Microsoft Foundry" => {
            let allowed = consent::ensure_consent(
                config,
                consent_keys::AZURE_CLI,
                "the Azure CLI (az)",
                "automatically discover your subscriptions, resources, and deployments",
            )?;

            if allowed {
                match foundry_discover_flow().await {
                    Ok(result) => Ok(Some(result)),
                    Err(e) => {
                        eprintln!(
                            "{} Auto-discovery failed: {}",
                            "Warning:".yellow().bold(),
                            e
                        );
                        eprintln!("Falling back to manual configuration.\n");
                        nodes::prompt_node_for_kind(config, kind_label).await
                    }
                }
            } else {
                nodes::prompt_node_for_kind(config, kind_label).await
            }
        }
        _ => nodes::prompt_node_for_kind(config, kind_label).await,
    }
}

// --- Azure / Foundry discovery flows ---

async fn azure_discover_flow() -> Result<(String, AiNode)> {
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("No enabled Azure subscriptions found");
    }

    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
    let Some(idx) = prompt_select_with_default("Select Azure subscription", &labels, default_idx)?
    else {
        anyhow::bail!("No subscription selected");
    };
    let sub = &subs[idx];

    azure_discover::set_subscription(&sub.id).await?;

    let resources = azure_discover::list_openai_resources().await?;
    if resources.is_empty() {
        anyhow::bail!(
            "No Azure OpenAI resources found in subscription '{}'",
            sub.name
        );
    }

    let labels: Vec<String> = resources.iter().map(|r| r.to_string()).collect();
    let Some(idx) = prompt_select("Select Azure OpenAI resource", &labels)? else {
        anyhow::bail!("No resource selected");
    };
    let resource = &resources[idx];
    let endpoint = resource
        .endpoint()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no endpoint", resource.name))?
        .trim_end_matches('/')
        .to_string();

    let rg = resource
        .resource_group
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no resource group", resource.name))?;

    let deployments = azure_discover::list_deployments(rg, &resource.name).await?;
    if deployments.is_empty() {
        anyhow::bail!("No deployments found on resource '{}'", resource.name);
    }

    let labels: Vec<String> = deployments.iter().map(|d| d.to_string()).collect();
    let Some(idx) = prompt_select("Select deployment", &labels)? else {
        anyhow::bail!("No deployment selected");
    };
    let deployment = &deployments[idx];

    let api_version = inquire::Text::new("API version:")
        .with_default("2025-04-01-preview")
        .prompt()?;

    let auth_options: Vec<String> = vec![
        "Azure CLI (az login) (Recommended)".into(),
        "API Key".into(),
    ];
    let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
        anyhow::bail!("No auth method selected");
    };

    let auth = if auth_idx == 1 {
        let key = inquire::Text::new("API key:").prompt()?;
        if key.is_empty() {
            Auth::AzureCli(true)
        } else {
            Auth::ApiKey(key)
        }
    } else {
        Auth::AzureCli(true)
    };

    let node = AiNode {
        provider: ProviderKind::AzureOpenAi,
        alias: None,
        capabilities: vec![Capability::Chat, Capability::Image],
        auth: Some(auth),
        model: None,
        endpoint: Some(endpoint),
        deployment: Some(deployment.name.clone()),
        api_version: Some(api_version),
        binary: None,
        project: None,
        location: None,
        node_defaults: None,
    };
    Ok((format!("azure-openai/{}", deployment.name), node))
}

async fn foundry_discover_flow() -> Result<(String, AiNode)> {
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("No enabled Azure subscriptions found");
    }

    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
    let Some(idx) = prompt_select_with_default("Select Azure subscription", &labels, default_idx)?
    else {
        anyhow::bail!("No subscription selected");
    };
    let sub = &subs[idx];

    azure_discover::set_subscription(&sub.id).await?;

    let resources = azure_discover::list_foundry_resources().await?;
    if resources.is_empty() {
        anyhow::bail!(
            "No Microsoft Foundry resources found in subscription '{}'",
            sub.name
        );
    }

    let labels: Vec<String> = resources.iter().map(|r| r.to_string()).collect();
    let Some(idx) = prompt_select("Select Microsoft Foundry resource", &labels)? else {
        anyhow::bail!("No resource selected");
    };
    let resource = &resources[idx];
    let raw_endpoint = resource
        .endpoint()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no endpoint", resource.name))?
        .trim_end_matches('/')
        .to_string();

    let endpoint = raw_endpoint.replace(".cognitiveservices.azure.com", ".services.ai.azure.com");

    let rg = resource
        .resource_group
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no resource group", resource.name))?;

    let deployments = azure_discover::list_deployments(rg, &resource.name).await?;
    if deployments.is_empty() {
        anyhow::bail!("No deployments found on resource '{}'", resource.name);
    }

    let labels: Vec<String> = deployments.iter().map(|d| d.to_string()).collect();
    let Some(idx) = prompt_select("Select deployment", &labels)? else {
        anyhow::bail!("No deployment selected");
    };
    let deployment = &deployments[idx];
    let model = deployment
        .properties
        .as_ref()
        .and_then(|p| p.model.as_ref())
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| deployment.name.clone());

    let api_version = inquire::Text::new("API version:")
        .with_default("2024-05-01-preview")
        .prompt()?;

    let auth_options: Vec<String> = vec![
        "Azure CLI (az login) (Recommended)".into(),
        "API Key".into(),
    ];
    let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
        anyhow::bail!("No auth method selected");
    };

    let auth = if auth_idx == 1 {
        let key = inquire::Text::new("API key:").prompt()?;
        if key.is_empty() {
            Auth::AzureCli(true)
        } else {
            Auth::ApiKey(key)
        }
    } else {
        Auth::AzureCli(true)
    };

    let node = AiNode {
        provider: ProviderKind::MicrosoftFoundry,
        alias: None,
        capabilities: vec![Capability::Chat],
        auth: Some(auth),
        model: Some(model.clone()),
        endpoint: Some(endpoint),
        deployment: None,
        api_version: Some(api_version),
        binary: None,
        project: None,
        location: None,
        node_defaults: None,
    };
    Ok((format!("microsoft-foundry/{}", model), node))
}

// --- Field editing helpers ---

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
        return edit_capabilities(node);
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

    let help = "Enter new value, or leave empty to clear";
    let label = format!("{}:", field);
    let mut prompt = inquire::Text::new(&label).with_help_message(help);
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
            let var = inquire::Text::new("Environment variable:")
                .with_default("OPENAI_API_KEY")
                .prompt()?;
            Some(Auth::Env(var))
        }
        1 => {
            let key = inquire::Text::new("API key:").prompt()?;
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

fn edit_capabilities(node: &mut AiNode) -> Result<()> {
    let supported = node.provider.supported_capabilities();
    let labels: Vec<String> = supported.iter().map(|c| c.label().to_string()).collect();
    let defaults: Vec<usize> = supported
        .iter()
        .enumerate()
        .filter(|(_, c)| node.capabilities.contains(c))
        .map(|(i, _)| i)
        .collect();

    match inquire::MultiSelect::new("What can this model do?", labels.clone())
        .with_default(&defaults)
        .prompt()
    {
        Ok(selected) => {
            node.capabilities = selected
                .iter()
                .filter_map(|s| {
                    labels
                        .iter()
                        .position(|l| l == s)
                        .map(|i| supported[i].clone())
                })
                .collect();
        }
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => {}
        Err(e) => return Err(e.into()),
    }

    Ok(())
}

// --- Non-interactive config commands ---

/// Valid node config field names.
const NODE_FIELDS: &[&str] = &[
    "provider",
    "model",
    "endpoint",
    "deployment",
    "api_version",
    "binary",
    "project",
    "location",
    "alias",
];

fn get_node_field(node: &AiNode, field: &str) -> Option<String> {
    match field {
        "provider" => Some(node.provider.to_string()),
        "model" => node.model.clone(),
        "endpoint" => node.endpoint.clone(),
        "deployment" => node.deployment.clone(),
        "api_version" => node.api_version.clone(),
        "binary" => node.binary.clone(),
        "project" => node.project.clone(),
        "location" => node.location.clone(),
        "alias" => node.alias.clone(),
        _ => None,
    }
}

fn set_node_field(node: &mut AiNode, field: &str, value: &str) -> Result<()> {
    match field {
        "provider" => {
            node.provider = value
                .parse::<ProviderKind>()
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        "model" => node.model = Some(value.to_string()),
        "endpoint" => node.endpoint = Some(value.to_string()),
        "deployment" => node.deployment = Some(value.to_string()),
        "api_version" => node.api_version = Some(value.to_string()),
        "binary" => node.binary = Some(value.to_string()),
        "project" => node.project = Some(value.to_string()),
        "location" => node.location = Some(value.to_string()),
        "alias" => node.alias = Some(value.to_string()),
        _ => anyhow::bail!(
            "Unknown field '{}'. Valid fields: {}",
            field,
            NODE_FIELDS.join(", ")
        ),
    }
    Ok(())
}

fn unset_node_field(node: &mut AiNode, field: &str) -> Result<()> {
    match field {
        "provider" => anyhow::bail!(
            "Cannot unset 'provider' — it is required. Remove the entire node with: ailloy config unset nodes.<id>"
        ),
        "model" => node.model = None,
        "endpoint" => node.endpoint = None,
        "deployment" => node.deployment = None,
        "api_version" => node.api_version = None,
        "binary" => node.binary = None,
        "project" => node.project = None,
        "location" => node.location = None,
        "alias" => node.alias = None,
        _ => anyhow::bail!(
            "Unknown field '{}'. Valid fields: {}",
            field,
            NODE_FIELDS.join(", ")
        ),
    }
    Ok(())
}

fn run_set(key: &str, value: &str) -> Result<()> {
    let segments: Vec<&str> = key.splitn(3, '.').collect();
    let mut config = Config::load_global()?;

    match segments.as_slice() {
        ["defaults", task] => {
            config.defaults.insert(task.to_string(), value.to_string());
        }
        ["nodes", id, field] => {
            if let Some(node) = config.get_node_mut(id) {
                set_node_field(node, field, value)?;
            } else if *field == "provider" {
                let provider = value
                    .parse::<ProviderKind>()
                    .map_err(|e| anyhow::anyhow!(e))?;
                config.add_node(
                    id.to_string(),
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
                    },
                );
            } else {
                anyhow::bail!(
                    "Node '{}' not found. Create it first with: ailloy config set nodes.{}.provider <kind>",
                    id,
                    id
                );
            }
        }
        _ => {
            anyhow::bail!(
                "Invalid key '{}'. Keys must start with 'defaults.' or 'nodes.'",
                key
            );
        }
    }

    config.save()?;
    Ok(())
}

fn run_get(key: &str) -> Result<()> {
    let segments: Vec<&str> = key.splitn(3, '.').collect();
    let config = Config::load_global()?;

    match segments.as_slice() {
        ["defaults", task] => {
            if let Some(value) = config.defaults.get(*task) {
                println!("{}", value);
            }
        }
        ["nodes", id] => {
            let (_, node) = config
                .get_node(id)
                .with_context(|| format!("Node '{}' not found", id))?;
            println!("provider: {}", node.provider);
            for &field in &NODE_FIELDS[1..] {
                if let Some(val) = get_node_field(node, field) {
                    println!("{}: {}", field, val);
                }
            }
        }
        ["nodes", id, field] => {
            if let Some((_, node)) = config.get_node(id) {
                if *field == "provider" {
                    println!("{}", node.provider);
                } else if let Some(val) = get_node_field(node, field) {
                    println!("{}", val);
                }
            }
        }
        _ => {
            anyhow::bail!(
                "Invalid key '{}'. Keys must start with 'defaults.' or 'nodes.'",
                key
            );
        }
    }

    Ok(())
}

fn run_unset(key: &str) -> Result<()> {
    let segments: Vec<&str> = key.splitn(3, '.').collect();
    let mut config = Config::load_global()?;

    match segments.as_slice() {
        ["defaults", task] => {
            config.unset_default(task);
        }
        ["nodes", id] => {
            config.remove_node(id);
        }
        ["nodes", id, field] => {
            let node = config
                .get_node_mut(id)
                .with_context(|| format!("Node '{}' not found", id))?;
            unset_node_field(node, field)?;
        }
        _ => {
            anyhow::bail!(
                "Invalid key '{}'. Keys must start with 'defaults.' or 'nodes.'",
                key
            );
        }
    }

    config.save()?;
    Ok(())
}

// --- Config show ---

fn run_show() -> Result<()> {
    let config = Config::load()?;

    if config.nodes.is_empty() {
        println!("{}", "No nodes configured.".dimmed());
        println!("Run {} to get started.", "ailloy config".bold());
        return Ok(());
    }

    println!("{}", "Configuration".bold());
    println!();

    if !config.defaults.is_empty() {
        println!("  {}", "Defaults:".dimmed());
        for (task, node_id) in &config.defaults {
            println!("    {} {}", format!("{}:", task).dimmed(), node_id.bold());
        }
    }

    println!();
    println!("  {}", "Nodes:".dimmed());

    for (id, node) in &config.nodes {
        let is_default = config.defaults.values().any(|d| d == id);
        let marker = if is_default { " (default)" } else { "" };

        println!();
        println!("    {}{}", id.bold(), marker.dimmed());
        println!("      {} {}", "Provider:".dimmed(), node.provider);

        if let Some(alias) = &node.alias {
            println!("      {} {}", "Alias:".dimmed(), alias);
        }
        if !node.capabilities.is_empty() {
            let caps: Vec<_> = node.capabilities.iter().map(|c| c.to_string()).collect();
            println!("      {} {}", "Capabilities:".dimmed(), caps.join(", "));
        }
        if let Some(model) = &node.model {
            println!("      {} {}", "Model:".dimmed(), model);
        }
        if let Some(endpoint) = &node.endpoint {
            println!("      {} {}", "Endpoint:".dimmed(), endpoint);
        }
        if let Some(deployment) = &node.deployment {
            println!("      {} {}", "Deployment:".dimmed(), deployment);
        }
        if let Some(binary) = &node.binary {
            println!("      {} {}", "Binary:".dimmed(), binary);
        }
        if let Some(project) = &node.project {
            println!("      {} {}", "Project:".dimmed(), project);
        }
        if let Some(location) = &node.location {
            println!("      {} {}", "Location:".dimmed(), location);
        }
        match &node.auth {
            Some(Auth::Env(var)) => println!("      {} env: {}", "Auth:".dimmed(), var),
            Some(Auth::ApiKey(_)) => println!("      {} api_key: ********", "Auth:".dimmed()),
            Some(Auth::AzureCli(_)) => println!("      {} azure_cli", "Auth:".dimmed()),
            Some(Auth::GcloudCli(_)) => println!("      {} gcloud_cli", "Auth:".dimmed()),
            None => {}
        }
    }

    if !config.consents.is_empty() {
        println!();
        println!("  {}", "Tool Consents:".dimmed());
        for (tool, allowed) in &config.consents {
            let status = if *allowed { "allowed" } else { "denied" };
            println!("    {} {}", format!("{}:", tool).dimmed(), status.bold());
        }
    }

    println!();
    println!(
        "  {} {}",
        "Config file:".dimmed(),
        Config::config_path()?.display().to_string().dimmed()
    );

    Ok(())
}
