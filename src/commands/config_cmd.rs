use anyhow::{Context, Result};
use colored::Colorize;
use inquire::Text;

use ailloy::config::{ALL_TASKS, Config, ProviderConfig, ProviderKind, consent_keys};

use crate::cli::ConfigCommands;
use crate::commands::azure_discover;
use crate::commands::consent;
use crate::commands::tui;
use crate::commands::tui::TuiAction;

/// Map io::Error from TUI helpers to anyhow.
fn tui_err(e: std::io::Error) -> anyhow::Error {
    anyhow::anyhow!("TUI error: {}", e)
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

// --- Interactive config (new entry point) ---

pub async fn run_interactive() -> Result<()> {
    let mut config = Config::load_global()?;
    let mut cursor_restore: Option<String> = None;

    loop {
        let (action, cursor_name) =
            tui::run_tui(&config, cursor_restore.as_deref()).map_err(tui_err)?;

        match action {
            TuiAction::Quit => break,

            TuiAction::EditProvider(name) => {
                cursor_restore = Some(name.clone());
                edit_provider_fields(&mut config, &name)?;
                config.save()?;
            }

            TuiAction::ToggleDefault { name, task } => {
                cursor_restore = Some(name.clone());
                let is_current = config.defaults.get(&task).is_some_and(|d| d == &name);
                if is_current {
                    config.defaults.remove(&task);
                } else {
                    config.defaults.insert(task, name);
                }
                config.save()?;
            }

            TuiAction::DeleteProvider(name) => {
                cursor_restore = Some(cursor_name);
                let confirm =
                    tui::tui_confirm(&format!("Delete provider '{}'?", name)).map_err(tui_err)?;
                if confirm {
                    config.remove_provider(&name);
                    config.save()?;
                    cursor_restore = None; // deleted, can't restore to it
                }
            }

            TuiAction::AddProvider(task_filter) => {
                cursor_restore = Some(cursor_name);
                if let Some(new_name) = add_provider(&mut config, task_filter.as_deref()).await? {
                    cursor_restore = Some(new_name);
                }
            }
        }
    }

    Ok(())
}

fn edit_provider_fields(config: &mut Config, name: &str) -> Result<()> {
    let provider = config.providers.get_mut(name).unwrap();

    loop {
        let fields = editable_fields(provider);
        let mut options: Vec<String> = fields
            .iter()
            .map(|(label, val)| format!("{}: {}", label, val.as_deref().unwrap_or("(not set)")))
            .collect();
        options.push("Done editing".to_string());

        let selected = tui::tui_select(&format!("Edit: {}", name), &options).map_err(tui_err)?;

        let Some(idx) = selected else {
            break; // ESC → done
        };

        if idx == options.len() - 1 {
            break; // "Done editing"
        }

        let field_name = fields[idx].0;
        edit_field(provider, field_name)?;
    }

    Ok(())
}

// --- Add Provider ---

/// Add a new provider. Returns the name of the new provider if one was added.
async fn add_provider(config: &mut Config, task_filter: Option<&str>) -> Result<Option<String>> {
    let Some((default_name, provider_config)) =
        prompt_provider_setup_filtered(config, task_filter).await?
    else {
        return Ok(None); // User cancelled
    };

    let name = Text::new("Provider name:")
        .with_default(&default_name)
        .prompt()?;

    if config.providers.contains_key(&name) {
        let overwrite =
            tui::tui_confirm(&format!("Provider '{}' already exists. Overwrite?", name))
                .map_err(tui_err)?;
        if !overwrite {
            return Ok(None);
        }
    }

    let kind = provider_config.kind.clone();
    config.providers.insert(name.clone(), provider_config);

    // Auto-set as default for tasks where no default exists and this provider supports it
    for &(task_key, _) in ALL_TASKS {
        if !config.defaults.contains_key(task_key) && kind.supports_task(task_key) {
            config.defaults.insert(task_key.to_string(), name.clone());
        }
    }

    config.save()?;

    Ok(Some(name))
}

/// All provider kinds with their display labels.
const PROVIDER_KINDS: &[(&str, ProviderKind)] = &[
    ("OpenAI", ProviderKind::OpenAi),
    ("Anthropic", ProviderKind::Anthropic),
    ("Azure OpenAI", ProviderKind::AzureOpenAi),
    ("Microsoft Foundry", ProviderKind::MicrosoftFoundry),
    ("Google Vertex AI", ProviderKind::VertexAi),
    ("Ollama", ProviderKind::Ollama),
    (
        "Local Agent (Claude, Codex, Copilot)",
        ProviderKind::LocalAgent,
    ),
];

/// Returns None if user cancelled.
async fn prompt_provider_setup_filtered(
    config: &mut Config,
    task_filter: Option<&str>,
) -> Result<Option<(String, ProviderConfig)>> {
    let kind_options: Vec<(&str, &ProviderKind)> = PROVIDER_KINDS
        .iter()
        .filter(|(_, kind)| task_filter.map(|t| kind.supports_task(t)).unwrap_or(true))
        .map(|(label, kind)| (*label, kind))
        .collect();

    let labels: Vec<String> = kind_options.iter().map(|(l, _)| l.to_string()).collect();

    let Some(idx) = tui::tui_select("Select AI provider", &labels).map_err(tui_err)? else {
        return Ok(None);
    };

    let kind_label = kind_options[idx].0;

    match kind_label {
        "OpenAI" => {
            let api_key = Text::new("API key:")
                .with_help_message("Or set OPENAI_API_KEY env var")
                .prompt()?;

            let model = Text::new("Model:").with_default("gpt-4o").prompt()?;

            let endpoint = Text::new("Endpoint (leave empty for default):")
                .with_default("")
                .prompt()?;

            let pc = ProviderConfig {
                kind: ProviderKind::OpenAi,
                api_key: if api_key.is_empty() {
                    None
                } else {
                    Some(api_key)
                },
                endpoint: if endpoint.is_empty() {
                    None
                } else {
                    Some(endpoint)
                },
                model: Some(model),
                deployment: None,
                api_version: None,
                binary: None,
                task: None,
                auth: None,
                project: None,
                location: None,
                provider_defaults: None,
            };
            Ok(Some(("openai".to_string(), pc)))
        }
        "Anthropic" => {
            let api_key = Text::new("API key:")
                .with_help_message("Or set ANTHROPIC_API_KEY env var")
                .prompt()?;

            let model = Text::new("Model:")
                .with_default("claude-sonnet-4-6")
                .prompt()?;

            let pc = ProviderConfig {
                kind: ProviderKind::Anthropic,
                api_key: if api_key.is_empty() {
                    None
                } else {
                    Some(api_key)
                },
                endpoint: None,
                model: Some(model),
                deployment: None,
                api_version: None,
                binary: None,
                task: None,
                auth: None,
                project: None,
                location: None,
                provider_defaults: None,
            };
            Ok(Some(("anthropic".to_string(), pc)))
        }
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
                        azure_manual_flow().map(Some)
                    }
                }
            } else {
                azure_manual_flow().map(Some)
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
                        foundry_manual_flow().map(Some)
                    }
                }
            } else {
                foundry_manual_flow().map(Some)
            }
        }
        "Google Vertex AI" => {
            let project = Text::new("GCP project:").prompt()?;

            let location = Text::new("Location:")
                .with_default("us-central1")
                .prompt()?;

            let model = Text::new("Model:")
                .with_default("gemini-3.1-pro")
                .prompt()?;

            let pc = ProviderConfig {
                kind: ProviderKind::VertexAi,
                api_key: None,
                endpoint: None,
                model: Some(model),
                deployment: None,
                api_version: None,
                binary: None,
                task: None,
                auth: Some("gcloud-cli".to_string()),
                project: Some(project),
                location: Some(location),
                provider_defaults: None,
            };
            Ok(Some(("vertex".to_string(), pc)))
        }
        "Ollama" => {
            let model = Text::new("Model:").with_default("llama3.2").prompt()?;

            let endpoint = Text::new("Endpoint:")
                .with_default("http://localhost:11434")
                .prompt()?;

            let pc = ProviderConfig {
                kind: ProviderKind::Ollama,
                api_key: None,
                endpoint: if endpoint == "http://localhost:11434" {
                    None
                } else {
                    Some(endpoint)
                },
                model: Some(model),
                deployment: None,
                api_version: None,
                binary: None,
                task: None,
                auth: None,
                project: None,
                location: None,
                provider_defaults: None,
            };
            Ok(Some(("ollama".to_string(), pc)))
        }
        _ => {
            // Local Agent
            let binary_options: Vec<String> =
                vec!["claude".into(), "codex".into(), "copilot".into()];
            let Some(idx) = tui::tui_select("Select agent", &binary_options).map_err(tui_err)?
            else {
                return Ok(None);
            };
            let binary = &binary_options[idx];

            let pc = ProviderConfig {
                kind: ProviderKind::LocalAgent,
                api_key: None,
                endpoint: None,
                model: None,
                deployment: None,
                api_version: None,
                binary: Some(binary.to_string()),
                task: None,
                auth: None,
                project: None,
                location: None,
                provider_defaults: None,
            };
            Ok(Some((binary.to_string(), pc)))
        }
    }
}

// --- Field editing helpers ---

fn editable_fields(pc: &ProviderConfig) -> Vec<(&'static str, Option<String>)> {
    let masked_key = pc.api_key.as_ref().map(|_| "********".to_string());
    match pc.kind {
        ProviderKind::OpenAi => vec![
            ("Model", pc.model.clone()),
            ("Endpoint", pc.endpoint.clone()),
            ("API key", masked_key),
        ],
        ProviderKind::Anthropic => vec![("Model", pc.model.clone()), ("API key", masked_key)],
        ProviderKind::AzureOpenAi => vec![
            ("Endpoint", pc.endpoint.clone()),
            ("Deployment", pc.deployment.clone()),
            ("API version", pc.api_version.clone()),
            ("Auth", pc.auth.clone()),
            ("API key", masked_key),
        ],
        ProviderKind::MicrosoftFoundry => vec![
            ("Endpoint", pc.endpoint.clone()),
            ("Model", pc.model.clone()),
            ("API version", pc.api_version.clone()),
            ("Auth", pc.auth.clone()),
            ("API key", masked_key),
        ],
        ProviderKind::VertexAi => vec![
            ("Project", pc.project.clone()),
            ("Location", pc.location.clone()),
            ("Model", pc.model.clone()),
        ],
        ProviderKind::Ollama => vec![
            ("Model", pc.model.clone()),
            ("Endpoint", pc.endpoint.clone()),
        ],
        ProviderKind::LocalAgent => vec![("Binary", pc.binary.clone())],
    }
}

fn edit_field(pc: &mut ProviderConfig, field: &str) -> Result<()> {
    let current = match field {
        "Model" => pc.model.as_deref().unwrap_or(""),
        "Endpoint" => pc.endpoint.as_deref().unwrap_or(""),
        "API key" => "",
        "Deployment" => pc.deployment.as_deref().unwrap_or(""),
        "API version" => pc.api_version.as_deref().unwrap_or(""),
        "Auth" => pc.auth.as_deref().unwrap_or(""),
        "Binary" => pc.binary.as_deref().unwrap_or(""),
        "Project" => pc.project.as_deref().unwrap_or(""),
        "Location" => pc.location.as_deref().unwrap_or(""),
        _ => return Ok(()),
    };

    let help = if field == "API key" {
        "Enter new key, or leave empty to clear"
    } else {
        "Enter new value, or leave empty to clear"
    };

    let label = format!("{}:", field);
    let mut prompt = Text::new(&label).with_help_message(help);
    if !current.is_empty() {
        prompt = prompt.with_default(current);
    }
    let value = prompt.prompt()?;

    let opt = if value.is_empty() { None } else { Some(value) };

    match field {
        "Model" => pc.model = opt,
        "Endpoint" => pc.endpoint = opt,
        "API key" => pc.api_key = opt,
        "Deployment" => pc.deployment = opt,
        "API version" => pc.api_version = opt,
        "Auth" => pc.auth = opt,
        "Binary" => pc.binary = opt,
        "Project" => pc.project = opt,
        "Location" => pc.location = opt,
        _ => {}
    }

    Ok(())
}

// --- Azure / Foundry discovery flows ---

async fn azure_discover_flow() -> Result<(String, ProviderConfig)> {
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("No enabled Azure subscriptions found");
    }

    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
    let Some(idx) = tui::tui_select_with_default("Select Azure subscription", &labels, default_idx)
        .map_err(tui_err)?
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
    let Some(idx) = tui::tui_select("Select Azure OpenAI resource", &labels).map_err(tui_err)?
    else {
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
    let Some(idx) = tui::tui_select("Select deployment", &labels).map_err(tui_err)? else {
        anyhow::bail!("No deployment selected");
    };
    let deployment = &deployments[idx];

    let api_version = Text::new("API version:")
        .with_default("2025-04-01-preview")
        .prompt()?;

    let auth_options: Vec<String> = vec![
        "Azure CLI (az login) (Recommended)".into(),
        "API Key".into(),
    ];
    let Some(auth_idx) =
        tui::tui_select("Authentication method", &auth_options).map_err(tui_err)?
    else {
        anyhow::bail!("No auth method selected");
    };

    let (api_key, auth) = if auth_idx == 1 {
        let key = Text::new("API key:").prompt()?;
        (if key.is_empty() { None } else { Some(key) }, None)
    } else {
        (None, Some("azure-cli".to_string()))
    };

    let pc = ProviderConfig {
        kind: ProviderKind::AzureOpenAi,
        api_key,
        endpoint: Some(endpoint),
        model: None,
        deployment: Some(deployment.name.clone()),
        api_version: Some(api_version),
        binary: None,
        task: None,
        auth,
        project: None,
        location: None,
        provider_defaults: None,
    };
    Ok(("azure".to_string(), pc))
}

fn azure_manual_flow() -> Result<(String, ProviderConfig)> {
    let endpoint = Text::new("Endpoint (e.g. https://my-instance.openai.azure.com):").prompt()?;

    let deployment = Text::new("Deployment name:").prompt()?;

    let api_version = Text::new("API version:")
        .with_default("2025-04-01-preview")
        .prompt()?;

    let auth_options: Vec<String> = vec!["API Key".into(), "Azure CLI (az login)".into()];
    let Some(auth_idx) =
        tui::tui_select("Authentication method", &auth_options).map_err(tui_err)?
    else {
        anyhow::bail!("No auth method selected");
    };

    let (api_key, auth) = if auth_idx == 0 {
        let key = Text::new("API key:").prompt()?;
        (if key.is_empty() { None } else { Some(key) }, None)
    } else {
        (None, Some("azure-cli".to_string()))
    };

    let pc = ProviderConfig {
        kind: ProviderKind::AzureOpenAi,
        api_key,
        endpoint: Some(endpoint),
        model: None,
        deployment: Some(deployment),
        api_version: Some(api_version),
        binary: None,
        task: None,
        auth,
        project: None,
        location: None,
        provider_defaults: None,
    };
    Ok(("azure".to_string(), pc))
}

async fn foundry_discover_flow() -> Result<(String, ProviderConfig)> {
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("No enabled Azure subscriptions found");
    }

    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
    let Some(idx) = tui::tui_select_with_default("Select Azure subscription", &labels, default_idx)
        .map_err(tui_err)?
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
    let Some(idx) =
        tui::tui_select("Select Microsoft Foundry resource", &labels).map_err(tui_err)?
    else {
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
    let Some(idx) = tui::tui_select("Select deployment", &labels).map_err(tui_err)? else {
        anyhow::bail!("No deployment selected");
    };
    let deployment = &deployments[idx];
    let model = deployment
        .properties
        .as_ref()
        .and_then(|p| p.model.as_ref())
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| deployment.name.clone());

    let api_version = Text::new("API version:")
        .with_default("2024-05-01-preview")
        .prompt()?;

    let auth_options: Vec<String> = vec![
        "Azure CLI (az login) (Recommended)".into(),
        "API Key".into(),
    ];
    let Some(auth_idx) =
        tui::tui_select("Authentication method", &auth_options).map_err(tui_err)?
    else {
        anyhow::bail!("No auth method selected");
    };

    let (api_key, auth) = if auth_idx == 1 {
        let key = Text::new("API key:").prompt()?;
        (if key.is_empty() { None } else { Some(key) }, None)
    } else {
        (None, Some("azure-cli".to_string()))
    };

    let pc = ProviderConfig {
        kind: ProviderKind::MicrosoftFoundry,
        api_key,
        endpoint: Some(endpoint),
        model: Some(model),
        deployment: None,
        api_version: Some(api_version),
        binary: None,
        task: None,
        auth,
        project: None,
        location: None,
        provider_defaults: None,
    };
    Ok(("foundry".to_string(), pc))
}

fn foundry_manual_flow() -> Result<(String, ProviderConfig)> {
    let endpoint =
        Text::new("Endpoint (e.g. https://my-instance.services.ai.azure.com):").prompt()?;

    let model = Text::new("Model (e.g. gpt-4o, Meta-Llama-3.1-405B-Instruct):").prompt()?;

    let api_version = Text::new("API version:")
        .with_default("2024-05-01-preview")
        .prompt()?;

    let auth_options: Vec<String> = vec!["API Key".into(), "Azure CLI (az login)".into()];
    let Some(auth_idx) =
        tui::tui_select("Authentication method", &auth_options).map_err(tui_err)?
    else {
        anyhow::bail!("No auth method selected");
    };

    let (api_key, auth) = if auth_idx == 0 {
        let key = Text::new("API key:").prompt()?;
        (if key.is_empty() { None } else { Some(key) }, None)
    } else {
        (None, Some("azure-cli".to_string()))
    };

    let pc = ProviderConfig {
        kind: ProviderKind::MicrosoftFoundry,
        api_key,
        endpoint: Some(endpoint),
        model: Some(model),
        deployment: None,
        api_version: Some(api_version),
        binary: None,
        task: None,
        auth,
        project: None,
        location: None,
        provider_defaults: None,
    };
    Ok(("foundry".to_string(), pc))
}

// --- Non-interactive config commands ---

/// Valid provider config field names.
const PROVIDER_FIELDS: &[&str] = &[
    "kind",
    "model",
    "endpoint",
    "api_key",
    "deployment",
    "api_version",
    "binary",
    "task",
    "auth",
    "project",
    "location",
];

fn get_provider_field(pc: &ProviderConfig, field: &str) -> Option<String> {
    match field {
        "kind" => Some(pc.kind.to_string()),
        "model" => pc.model.clone(),
        "endpoint" => pc.endpoint.clone(),
        "api_key" => pc.api_key.as_ref().map(|_| "********".to_string()),
        "deployment" => pc.deployment.clone(),
        "api_version" => pc.api_version.clone(),
        "binary" => pc.binary.clone(),
        "task" => pc.task.clone(),
        "auth" => pc.auth.clone(),
        "project" => pc.project.clone(),
        "location" => pc.location.clone(),
        _ => None,
    }
}

fn set_provider_field(pc: &mut ProviderConfig, field: &str, value: &str) -> Result<()> {
    match field {
        "kind" => {
            pc.kind = value
                .parse::<ProviderKind>()
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        "model" => pc.model = Some(value.to_string()),
        "endpoint" => pc.endpoint = Some(value.to_string()),
        "api_key" => pc.api_key = Some(value.to_string()),
        "deployment" => pc.deployment = Some(value.to_string()),
        "api_version" => pc.api_version = Some(value.to_string()),
        "binary" => pc.binary = Some(value.to_string()),
        "task" => pc.task = Some(value.to_string()),
        "auth" => pc.auth = Some(value.to_string()),
        "project" => pc.project = Some(value.to_string()),
        "location" => pc.location = Some(value.to_string()),
        _ => anyhow::bail!(
            "Unknown field '{}'. Valid fields: {}",
            field,
            PROVIDER_FIELDS.join(", ")
        ),
    }
    Ok(())
}

fn unset_provider_field(pc: &mut ProviderConfig, field: &str) -> Result<()> {
    match field {
        "kind" => anyhow::bail!(
            "Cannot unset 'kind' — it is required. Remove the entire provider with: ailloy config unset providers.<name>"
        ),
        "model" => pc.model = None,
        "endpoint" => pc.endpoint = None,
        "api_key" => pc.api_key = None,
        "deployment" => pc.deployment = None,
        "api_version" => pc.api_version = None,
        "binary" => pc.binary = None,
        "task" => pc.task = None,
        "auth" => pc.auth = None,
        "project" => pc.project = None,
        "location" => pc.location = None,
        _ => anyhow::bail!(
            "Unknown field '{}'. Valid fields: {}",
            field,
            PROVIDER_FIELDS.join(", ")
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
        ["providers", name, field] => {
            if let Some(pc) = config.providers.get_mut(*name) {
                set_provider_field(pc, field, value)?;
            } else if *field == "kind" {
                let kind = value
                    .parse::<ProviderKind>()
                    .map_err(|e| anyhow::anyhow!(e))?;
                config.providers.insert(
                    name.to_string(),
                    ProviderConfig {
                        kind,
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
                );
            } else {
                anyhow::bail!(
                    "Provider '{}' not found. Create it first with: ailloy config set providers.{}.kind <kind>",
                    name,
                    name
                );
            }
        }
        _ => {
            anyhow::bail!(
                "Invalid key '{}'. Keys must start with 'defaults.' or 'providers.'",
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
        ["providers", name] => {
            let pc = config
                .providers
                .get(*name)
                .with_context(|| format!("Provider '{}' not found", name))?;
            println!("kind: {}", pc.kind);
            for &field in &PROVIDER_FIELDS[1..] {
                if let Some(val) = get_provider_field(pc, field) {
                    println!("{}: {}", field, val);
                }
            }
        }
        ["providers", name, field] => {
            if let Some(pc) = config.providers.get(*name) {
                if *field == "kind" {
                    println!("{}", pc.kind);
                } else if let Some(val) = get_provider_field(pc, field) {
                    println!("{}", val);
                }
            }
        }
        _ => {
            anyhow::bail!(
                "Invalid key '{}'. Keys must start with 'defaults.' or 'providers.'",
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
            config.defaults.remove(*task);
        }
        ["providers", name] => {
            config.remove_provider(name);
        }
        ["providers", name, field] => {
            let pc = config
                .providers
                .get_mut(*name)
                .with_context(|| format!("Provider '{}' not found", name))?;
            unset_provider_field(pc, field)?;
        }
        _ => {
            anyhow::bail!(
                "Invalid key '{}'. Keys must start with 'defaults.' or 'providers.'",
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

    if config.providers.is_empty() {
        println!("{}", "No providers configured.".dimmed());
        println!("Run {} to get started.", "ailloy config".bold());
        return Ok(());
    }

    println!("{}", "Configuration".bold());
    println!();

    if !config.defaults.is_empty() {
        println!("  {}", "Defaults:".dimmed());
        for (task, provider) in &config.defaults {
            println!("    {} {}", format!("{}:", task).dimmed(), provider.bold());
        }
    }

    println!();
    println!("  {}", "Providers:".dimmed());

    let default_chat = config.defaults.get("chat");

    for (name, provider) in &config.providers {
        let is_default = default_chat.is_some_and(|d| d == name);
        let marker = if is_default { " (default)" } else { "" };

        println!();
        println!("    {}{}", name.bold(), marker.dimmed());
        println!("      {} {}", "Kind:".dimmed(), provider.kind);

        if let Some(model) = &provider.model {
            println!("      {} {}", "Model:".dimmed(), model);
        }
        if let Some(endpoint) = &provider.endpoint {
            println!("      {} {}", "Endpoint:".dimmed(), endpoint);
        }
        if let Some(binary) = &provider.binary {
            println!("      {} {}", "Binary:".dimmed(), binary);
        }
        if let Some(task) = &provider.task {
            println!("      {} {}", "Task:".dimmed(), task);
        }
        if let Some(auth) = &provider.auth {
            println!("      {} {}", "Auth:".dimmed(), auth);
        }
        if let Some(project) = &provider.project {
            println!("      {} {}", "Project:".dimmed(), project);
        }
        if let Some(location) = &provider.location {
            println!("      {} {}", "Location:".dimmed(), location);
        }
        if provider.api_key.is_some() {
            println!("      {} {}", "API key:".dimmed(), "********".dimmed());
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
