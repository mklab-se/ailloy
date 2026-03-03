use anyhow::Result;
use colored::Colorize;

use ailloy::config::Config;

use crate::cli::ProviderCommands;

pub async fn run(cmd: ProviderCommands) -> Result<()> {
    match cmd {
        ProviderCommands::List => run_list(),
        ProviderCommands::Detect => run_detect().await,
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
        println!(
            "  {} — Anthropic API (Claude Sonnet, etc.)",
            "anthropic".bold()
        );
        println!("  {} — Azure OpenAI Service", "azure-openai".bold());
        println!("  {} — Google Vertex AI (Gemini, etc.)", "vertex-ai".bold());
        println!("  {} — Local LLMs via Ollama", "ollama".bold());
        println!(
            "  {} — CLI agents (Claude, Codex, Copilot)",
            "local-agent".bold()
        );
        return Ok(());
    }

    let default_chat = config.defaults.get("chat");

    for (name, provider) in &config.providers {
        let is_default = default_chat.is_some_and(|d| d == name);
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

async fn run_detect() -> Result<()> {
    println!("{}", "Detected Providers".bold());
    println!();

    // Check OPENAI_API_KEY
    if std::env::var("OPENAI_API_KEY").is_ok() {
        println!(
            "  {} {}  {}",
            "✓".green().bold(),
            "openai".bold(),
            "OPENAI_API_KEY is set".dimmed()
        );
    } else {
        println!(
            "  {} {}  {}",
            "✗".red(),
            "openai".dimmed(),
            "OPENAI_API_KEY not set".dimmed()
        );
    }

    // Check ANTHROPIC_API_KEY
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!(
            "  {} {}  {}",
            "✓".green().bold(),
            "anthropic".bold(),
            "ANTHROPIC_API_KEY is set".dimmed()
        );
    } else {
        println!(
            "  {} {}  {}",
            "✗".red(),
            "anthropic".dimmed(),
            "ANTHROPIC_API_KEY not set".dimmed()
        );
    }

    // Check Ollama
    let ollama_check = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reqwest::get("http://localhost:11434/api/tags"),
    )
    .await;
    match ollama_check {
        Ok(Ok(resp)) if resp.status().is_success() => {
            println!(
                "  {} {}  {}",
                "✓".green().bold(),
                "ollama".bold(),
                "Running at localhost:11434".dimmed()
            );
        }
        _ => {
            println!(
                "  {} {}  {}",
                "✗".red(),
                "ollama".dimmed(),
                "Not running at localhost:11434".dimmed()
            );
        }
    }

    // Check Azure CLI
    let az_check = tokio::process::Command::new("az")
        .args(["account", "show", "--query", "name", "-o", "tsv"])
        .output()
        .await;
    match az_check {
        Ok(output) if output.status.success() => {
            let name = String::from_utf8_lossy(&output.stdout);
            println!(
                "  {} {}  {}",
                "✓".green().bold(),
                "azure".bold(),
                format!("az CLI authenticated ({})", name.trim()).dimmed()
            );
        }
        _ => {
            println!(
                "  {} {}  {}",
                "✗".red(),
                "azure".dimmed(),
                "az CLI not authenticated or not installed".dimmed()
            );
        }
    }

    // Check gcloud CLI
    let gcloud_check = tokio::process::Command::new("gcloud")
        .args(["auth", "print-access-token"])
        .output()
        .await;
    match gcloud_check {
        Ok(output) if output.status.success() => {
            println!(
                "  {} {}  {}",
                "✓".green().bold(),
                "vertex".bold(),
                "gcloud CLI authenticated".dimmed()
            );
        }
        _ => {
            println!(
                "  {} {}  {}",
                "✗".red(),
                "vertex".dimmed(),
                "gcloud CLI not authenticated or not installed".dimmed()
            );
        }
    }

    // Check local agents
    for binary in &["claude", "codex", "copilot"] {
        let which_check = tokio::process::Command::new("which")
            .arg(binary)
            .output()
            .await;
        match which_check {
            Ok(output) if output.status.success() => {
                println!(
                    "  {} {}  {}",
                    "✓".green().bold(),
                    binary.to_string().bold(),
                    format!("{} found in PATH", binary).dimmed()
                );
            }
            _ => {
                println!(
                    "  {} {}  {}",
                    "✗".red(),
                    binary.dimmed(),
                    format!("{} not found", binary).dimmed()
                );
            }
        }
    }

    println!();
    println!(
        "Run {} to configure detected providers.",
        "'ailloy config init'".bold()
    );

    Ok(())
}
