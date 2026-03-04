use anyhow::Result;
use colored::Colorize;

use ailloy::config::{Config, consent_keys};
use ailloy::discover;

use crate::cli::DiscoverArgs;
use crate::commands::consent;

pub async fn run(args: DiscoverArgs) -> Result<()> {
    let config = Config::load()?;
    let discover_all = args.all || (!args.locally && !args.azure);

    println!("{}", "Discovered AI Nodes".bold());
    println!();

    let mut all_discovered = Vec::new();

    // Environment variable keys (always checked)
    if discover_all || args.locally {
        let env_nodes = discover::discover_env_keys();
        for node in &env_nodes {
            println!(
                "  {} {}  {}",
                "✓".green().bold(),
                node.suggested_id.bold(),
                node.description.dimmed()
            );
        }
        if env_nodes.is_empty() {
            println!(
                "  {} {}  {}",
                "✗".red(),
                "openai".dimmed(),
                "OPENAI_API_KEY not set".dimmed()
            );
            println!(
                "  {} {}  {}",
                "✗".red(),
                "anthropic".dimmed(),
                "ANTHROPIC_API_KEY not set".dimmed()
            );
        }
        all_discovered.extend(env_nodes);
    }

    // Ollama
    if discover_all || args.locally {
        match discover::discover_ollama(None).await {
            Ok(nodes) => {
                for node in &nodes {
                    println!(
                        "  {} {}  {}",
                        "✓".green().bold(),
                        node.suggested_id.bold(),
                        node.description.dimmed()
                    );
                }
                all_discovered.extend(nodes);
            }
            Err(_) => {
                println!(
                    "  {} {}  {}",
                    "✗".red(),
                    "ollama".dimmed(),
                    "Not running at localhost:11434".dimmed()
                );
            }
        }
    }

    // Local agents
    if discover_all || args.locally {
        match discover::discover_local().await {
            Ok(nodes) => {
                for node in &nodes {
                    println!(
                        "  {} {}  {}",
                        "✓".green().bold(),
                        node.suggested_id.bold(),
                        node.description.dimmed()
                    );
                }
                all_discovered.extend(nodes);
            }
            Err(e) => {
                tracing::debug!("Local agent discovery failed: {}", e);
            }
        }
    }

    // Azure
    if discover_all || args.azure {
        if consent::check_consent(&config, consent_keys::AZURE_CLI) == Some(true) {
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
        } else {
            println!(
                "  {} {}  {}",
                "-".dimmed(),
                "azure".dimmed(),
                "skipped (not authorized — run 'ailloy config' to grant)".dimmed()
            );
        }

        // gcloud
        if consent::check_consent(&config, consent_keys::GCLOUD_CLI) == Some(true) {
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
        } else {
            println!(
                "  {} {}  {}",
                "-".dimmed(),
                "vertex".dimmed(),
                "skipped (not authorized — run 'ailloy config' to grant)".dimmed()
            );
        }
    }

    println!();

    if all_discovered.is_empty() {
        println!(
            "No nodes discovered. Run {} to configure manually.",
            "'ailloy nodes add'".bold()
        );
    } else {
        println!(
            "Run {} to add discovered nodes to your config.",
            "'ailloy nodes add'".bold()
        );
    }

    Ok(())
}
