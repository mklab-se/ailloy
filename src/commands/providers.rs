use anyhow::Result;
use colored::Colorize;

use ailloy::config::Config;

use crate::cli::ProviderCommands;

pub fn run(cmd: ProviderCommands) -> Result<()> {
    match cmd {
        ProviderCommands::List => run_list(),
    }
}

fn run_list() -> Result<()> {
    let config = Config::load()?;

    println!("{}", "Configured Providers".bold());
    println!();

    if config.providers.is_empty() {
        println!("  {}", "No providers configured.".dimmed());
        println!("  Run {} to add one.", "ailloy config init".bold());
        println!();
        println!("{}", "Available Provider Types".bold());
        println!();
        println!("  {} — OpenAI API (GPT-4o, etc.)", "openai".bold());
        println!("  {} — Azure OpenAI Service", "azure-openai".bold());
        println!("  {} — Local LLMs via Ollama", "ollama".bold());
        println!(
            "  {} — CLI agents (Claude, Codex, Copilot)",
            "local-agent".bold()
        );
        return Ok(());
    }

    for (name, provider) in &config.providers {
        let is_default = config
            .default_provider
            .as_deref()
            .is_some_and(|d| d == name);
        let marker = if is_default { " <- default" } else { "" };

        let model_info = provider
            .model
            .as_deref()
            .or(provider.binary.as_deref())
            .unwrap_or("-");

        println!(
            "  {} ({}) [{}]{}",
            name.bold(),
            provider.kind.to_string().dimmed(),
            model_info,
            marker.green()
        );
    }

    println!();
    Ok(())
}
