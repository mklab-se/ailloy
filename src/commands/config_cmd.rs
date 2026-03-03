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

    let kind_options = vec!["OpenAI", "Ollama", "Local Agent (Claude, Codex, Copilot)"];
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
            };
            ("openai".to_string(), pc)
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
            };
            (binary.to_string(), pc)
        }
    };

    config.providers.insert(name.clone(), provider_config);

    if config.default_provider.is_none() || config.providers.len() == 1 {
        config.default_provider = Some(name.clone());
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

    if let Some(default) = &config.default_provider {
        println!("  {} {}", "Default provider:".dimmed(), default.bold());
    }

    println!();
    println!("  {}", "Providers:".dimmed());

    for (name, provider) in &config.providers {
        let is_default = config
            .default_provider
            .as_deref()
            .is_some_and(|d| d == name);
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
