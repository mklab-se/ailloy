mod banner;
mod cli;
mod commands;
mod update;

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = match cli.verbose {
        0 => "warn",
        1 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .init();

    if cli.no_color {
        colored::control::set_override(false);
    }

    let update_handle = if !cli.quiet {
        Some(tokio::spawn(update::check_for_update()))
    } else {
        None
    };

    let result = match cli.command {
        Commands::Chat(args) => commands::chat::run(args, cli.quiet).await,
        Commands::Config(cmd) => commands::config_cmd::run(cmd).await,
        Commands::Providers(cmd) => commands::providers::run(cmd),
        Commands::Completion(args) => commands::completion::run(args),
        Commands::Version => {
            banner::print_banner();
            Ok(())
        }
    };

    if let Some(handle) = update_handle {
        if let Ok(Some(latest)) = handle.await {
            let current = env!("CARGO_PKG_VERSION");
            if latest != current {
                eprintln!(
                    "\n{} {} -> {} ({})",
                    "Update available:".yellow().bold(),
                    current.dimmed(),
                    latest.green(),
                    "brew upgrade ailloy".dimmed()
                );
            }
        }
    }

    result
}
