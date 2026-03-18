mod banner;
mod cli;
mod commands;
mod update;

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Commands, ConfigCommands, KNOWN_SUBCOMMANDS};

#[tokio::main]
async fn main() -> Result<()> {
    // Pre-parse: if argv[1] is not a known subcommand and doesn't start with '-',
    // insert "chat" so it becomes: ailloy chat "message"
    let args: Vec<String> = std::env::args().collect();
    let effective_args = if args.len() > 1 {
        let first = &args[1];
        if !first.starts_with('-') && !KNOWN_SUBCOMMANDS.contains(&first.as_str()) {
            let mut new_args = vec![args[0].clone(), "chat".to_string()];
            new_args.extend(args[1..].iter().cloned());
            new_args
        } else {
            args
        }
    } else {
        args
    };

    let cli = Cli::parse_from(effective_args);

    let filter = match cli.verbose {
        0 => "warn",
        1 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .init();

    // --raw on chat/image implies --quiet and --no-color
    let is_raw = matches!(&cli.command, Commands::Chat(args) if args.raw)
        || matches!(&cli.command, Commands::Image(args) if args.raw);
    let quiet = cli.quiet || is_raw;

    if cli.no_color || is_raw {
        colored::control::set_override(false);
    }

    let update_handle = if !quiet {
        Some(tokio::spawn(update::check_for_update()))
    } else {
        None
    };

    let result = match cli.command {
        Commands::Chat(args) => commands::chat::run(args, quiet).await,
        Commands::Image(args) => commands::image::run(args, quiet).await,
        Commands::Ai { command } => commands::ai::run(command).await,
        Commands::Completion(args) => commands::completion::run(args),
        Commands::Version => {
            banner::print_banner();
            Ok(())
        }

        // Backward-compat: deprecated top-level commands
        Commands::Config(args) => {
            eprintln!(
                "{}",
                "Note: 'ailloy config' is deprecated, use 'ailloy ai config' instead."
                    .yellow()
                    .dimmed()
            );
            match args.command {
                None | Some(ConfigCommands::Init) => {
                    let mut config = ailloy::config::Config::load_global()?;
                    ailloy::config_tui::run_interactive_config(&mut config, &["chat", "image"])
                        .await?;
                    Ok(())
                }
                Some(ConfigCommands::Show) => commands::config_cmd::run_show(),
                Some(ConfigCommands::Set { key, value }) => {
                    commands::config_cmd::run_set(&key, &value)
                }
                Some(ConfigCommands::Get { key }) => commands::config_cmd::run_get(&key),
                Some(ConfigCommands::Unset { key }) => commands::config_cmd::run_unset(&key),
            }
        }
        Commands::Nodes(cmd) => commands::ai::run_legacy_nodes(cmd).await,
        Commands::Discover(_) => {
            eprintln!(
                "{}",
                "Note: 'ailloy discover' has been removed. Discovery is now part of 'ailloy ai config'."
                    .yellow()
                    .dimmed()
            );
            eprintln!(
                "Run {} to configure AI providers.",
                "'ailloy ai config'".bold()
            );
            Ok(())
        }
    };

    if let Some(handle) = update_handle {
        if let Ok(Some(latest)) = handle.await {
            let current = env!("CARGO_PKG_VERSION");
            if latest != current && !update::is_running_from_source() {
                let hint = update::upgrade_hint();
                eprintln!(
                    "\n{} {} -> {} ({})",
                    "Update available:".yellow().bold(),
                    current.dimmed(),
                    latest.green(),
                    hint.dimmed()
                );
            }
        }
    }

    result
}
