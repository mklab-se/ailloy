use anyhow::{Context, Result};
use colored::Colorize;

use ailloy::config::{Capability, Config};
use ailloy::config_tui;

use crate::cli::{AiCommands, AiConfigCommands, NodeCommands};

pub async fn run(command: Option<AiCommands>) -> Result<()> {
    match command {
        None => config_tui::print_ai_status("ailloy", &["chat", "image"]),
        Some(AiCommands::Config { command }) => run_config(command).await,
        Some(AiCommands::Test { message }) => config_tui::run_test_chat("ailloy", message).await,
        Some(AiCommands::Enable) => config_tui::enable_ai("ailloy"),
        Some(AiCommands::Disable) => config_tui::disable_ai("ailloy"),
        Some(AiCommands::Status) => config_tui::print_ai_status("ailloy", &["chat", "image"]),
        Some(AiCommands::Skill { emit, reference }) => {
            crate::commands::skill::run(emit, reference);
            Ok(())
        }
    }
}

async fn run_config(command: Option<AiConfigCommands>) -> Result<()> {
    match command {
        None => {
            let mut config = Config::load_global()?;
            config_tui::run_interactive_config(&mut config, &["chat", "image"]).await?;
            Ok(())
        }
        Some(AiConfigCommands::AddNode) => {
            let mut config = Config::load_global()?;
            if let Some(name) = config_tui::add_node_interactive(&mut config).await? {
                println!("{} Added node '{}'", "✓".green().bold(), name.bold());
            }
            Ok(())
        }
        Some(AiConfigCommands::EditNode { id }) => {
            let mut config = Config::load_global()?;
            config_tui::edit_node_interactive(&mut config, &id)
        }
        Some(AiConfigCommands::DeleteNode { id }) => run_delete_node(&id),
        Some(AiConfigCommands::SetDefault { node_name, task }) => {
            run_set_default(&task, &node_name)
        }
        Some(AiConfigCommands::ListNodes) => {
            let config = Config::load()?;
            config_tui::print_nodes_list(&config)
        }
        Some(AiConfigCommands::ShowNode { id }) => run_show_node(&id),
        Some(AiConfigCommands::Show) => super::config_cmd::run_show(),
        Some(AiConfigCommands::Set { key, value }) => super::config_cmd::run_set(&key, &value),
        Some(AiConfigCommands::Get { key }) => super::config_cmd::run_get(&key),
        Some(AiConfigCommands::Unset { key }) => super::config_cmd::run_unset(&key),
        Some(AiConfigCommands::Reset) => config_tui::reset_config(),
    }
}

fn run_delete_node(id_or_alias: &str) -> Result<()> {
    let mut config = Config::load_global()?;
    let canonical_id = config
        .resolve_node(id_or_alias)
        .map(|s| s.to_string())
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;

    config.remove_node(&canonical_id);
    config.save()?;
    println!(
        "{} Removed node '{}'",
        "✓".green().bold(),
        canonical_id.bold()
    );
    Ok(())
}

fn run_set_default(capability: &str, node_id: &str) -> Result<()> {
    let mut config = Config::load_global()?;
    let _: Capability = capability.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let canonical = config
        .resolve_node(node_id)
        .map(|s| s.to_string())
        .with_context(|| format!("Node '{}' not found", node_id))?;
    config.set_default(capability, &canonical);
    config.save()?;
    println!(
        "{} Default for '{}': {}",
        "✓".green().bold(),
        capability,
        canonical.bold()
    );
    Ok(())
}

fn run_show_node(id_or_alias: &str) -> Result<()> {
    let config = Config::load()?;
    let (canonical_id, node) = config
        .get_node(id_or_alias)
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;
    config_tui::print_node_info(canonical_id, node, &config);
    Ok(())
}

// ---------------------------------------------------------------------------
// Backward-compat: handle old `ailloy nodes` subcommand
// ---------------------------------------------------------------------------

pub async fn run_legacy_nodes(cmd: NodeCommands) -> Result<()> {
    eprintln!(
        "{}",
        "Note: 'ailloy nodes' is deprecated, use 'ailloy ai config' instead."
            .yellow()
            .dimmed()
    );

    match cmd {
        NodeCommands::List => {
            let config = Config::load()?;
            config_tui::print_nodes_list(&config)
        }
        NodeCommands::Add => {
            let mut config = Config::load_global()?;
            config_tui::add_node_interactive(&mut config).await?;
            Ok(())
        }
        NodeCommands::Edit { id } => {
            let mut config = Config::load_global()?;
            config_tui::edit_node_interactive(&mut config, &id)
        }
        NodeCommands::Remove { id } => run_delete_node(&id),
        NodeCommands::Default {
            capability,
            node_id,
        } => {
            if let Some(id) = node_id {
                run_set_default(&capability, &id)
            } else {
                let config = Config::load_global()?;
                match config.defaults.get(&capability) {
                    Some(id) => println!("{}: {}", capability, id.bold()),
                    None => println!("{}: {}", capability, "(not set)".dimmed()),
                }
                Ok(())
            }
        }
        NodeCommands::Show { id } => run_show_node(&id),
    }
}
