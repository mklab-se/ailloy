use anyhow::Result;
use colored::Colorize;
use inquire::{Select, Text};

use ailloy::config::{Config, ProviderConfig, ProviderKind};

use crate::cli::ConfigCommands;

pub async fn run(cmd: ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::Init => run_init().await,
        ConfigCommands::Show => run_show(),
    }
}

async fn run_init() -> Result<()> {
    println!("{}", "Ailloy Configuration Setup".bold());
    println!();

    let mut config = Config::load()?;

    let kind_options = vec![
        "OpenAI",
        "Anthropic",
        "Azure OpenAI",
        "Google Vertex AI",
        "Ollama",
        "Local Agent (Claude, Codex, Copilot)",
    ];
    let kind = Select::new("Select AI provider:", kind_options).prompt()?;

    let (name, provider_config) = match kind {
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
            ("openai".to_string(), pc)
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
            ("anthropic".to_string(), pc)
        }
        "Azure OpenAI" => {
            let endpoint =
                Text::new("Endpoint (e.g. https://my-instance.openai.azure.com):").prompt()?;

            let deployment = Text::new("Deployment name:").prompt()?;

            let api_version = Text::new("API version:")
                .with_default("2025-01-01")
                .prompt()?;

            let auth_options = vec!["API Key", "Azure CLI (az login)"];
            let auth_choice = Select::new("Authentication method:", auth_options).prompt()?;

            let (api_key, auth) = match auth_choice {
                "API Key" => {
                    let key = Text::new("API key:").prompt()?;
                    (if key.is_empty() { None } else { Some(key) }, None)
                }
                _ => (None, Some("azure-cli".to_string())),
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
            ("azure".to_string(), pc)
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
            ("vertex".to_string(), pc)
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
            ("ollama".to_string(), pc)
        }
        _ => {
            let binary_options = vec!["claude", "codex", "copilot"];
            let binary = Select::new("Select agent:", binary_options).prompt()?;

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
            (binary.to_string(), pc)
        }
    };

    config.providers.insert(name.clone(), provider_config);

    if !config.defaults.contains_key("chat") || config.providers.len() == 1 {
        config.defaults.insert("chat".to_string(), name.clone());
    }

    config.save()?;

    println!();
    println!(
        "{} Provider '{}' configured as default.",
        "Done!".green().bold(),
        name.bold()
    );
    println!(
        "  Config saved to {}",
        Config::config_path()?.display().to_string().dimmed()
    );
    println!();
    println!("  Try it: {}", "ailloy chat \"Hello!\"".bold());

    Ok(())
}

fn run_show() -> Result<()> {
    let config = Config::load()?;

    if config.providers.is_empty() {
        println!("{}", "No providers configured.".dimmed());
        println!("Run {} to get started.", "ailloy config init".bold());
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

    println!();
    println!(
        "  {} {}",
        "Config file:".dimmed(),
        Config::config_path()?.display().to_string().dimmed()
    );

    Ok(())
}
