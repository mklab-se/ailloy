use anyhow::{Context, Result};
use colored::Colorize;

use ailloy::config::{AiNode, Auth, Config, ProviderKind};

// ---------------------------------------------------------------------------
// Non-interactive config commands (show/set/get/unset)
// ---------------------------------------------------------------------------

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
            "Cannot unset 'provider' — it is required. Remove the entire node with: ailloy ai config unset nodes.<id>"
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

pub fn run_set(key: &str, value: &str) -> Result<()> {
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
                    "Node '{}' not found. Create it first with: ailloy ai config set nodes.{}.provider <kind>",
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

pub fn run_get(key: &str) -> Result<()> {
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

pub fn run_unset(key: &str) -> Result<()> {
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

pub fn run_show() -> Result<()> {
    let config = Config::load()?;

    if config.nodes.is_empty() {
        println!("{}", "No nodes configured.".dimmed());
        println!("Run {} to get started.", "ailloy ai config".bold());
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
