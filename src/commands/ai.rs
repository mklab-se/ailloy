use anyhow::{Context, Result};
use colored::Colorize;

use ailloy::config::{Capability, Config};
use ailloy::config_tui;

use crate::cli::{AiCommands, AiConfigCommands, NodeCommands};

pub async fn run(command: Option<AiCommands>) -> Result<()> {
    match command {
        None => config_tui::print_ai_status("ailloy", &["chat", "image", "video"]),
        Some(AiCommands::Config { command }) => run_config(command).await,
        Some(AiCommands::Test { message, all }) => {
            if all {
                run_test_all().await
            } else {
                config_tui::run_test_chat("ailloy", message).await
            }
        }
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
            config_tui::run_interactive_config(
                &mut config,
                &["chat", "image", "video", "embedding"],
            )
            .await?;
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
        Some(AiConfigCommands::SetKey { id }) => run_set_key(&id),
        Some(AiConfigCommands::InitLocal { extends_global }) => run_init_local(extends_global),
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

/// Store a node's API key in the OS keychain and switch its auth to keychain.
fn run_set_key(id_or_alias: &str) -> Result<()> {
    let mut config = Config::load_global()?;
    let (node_id, _) = config.get_node(id_or_alias).with_context(|| {
        format!("unknown node '{id_or_alias}' — see `ailloy ai config list-nodes`")
    })?;
    let node_id = node_id.to_string();

    let secret = inquire::Password::new(&format!("API key for {node_id}:"))
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()
        .context("cancelled")?;
    if secret.trim().is_empty() {
        anyhow::bail!("empty key — nothing stored");
    }

    ailloy::config::set_keychain_secret(&node_id, secret.trim())?;
    if let Some(node) = config.nodes.get_mut(&node_id) {
        node.auth = Some(ailloy::config::Auth::Keychain(true));
    }
    config.save()?;
    println!(
        "{} key stored in the OS keychain (service 'ailloy', account '{}'); node auth set to keychain",
        "✓".green().bold(),
        node_id.bold()
    );
    Ok(())
}

/// Ping every configured node: 1-token chat for chat-capable nodes, a tiny
/// embed for embedding-capable ones. Exit code 1 when any node fails.
async fn run_test_all() -> Result<()> {
    use std::time::Instant;
    let config = Config::load()?;
    if config.nodes.is_empty() {
        println!("No nodes configured. Run `ailloy ai config` to add one.");
        return Ok(());
    }
    let mut failures = 0usize;
    for (id, node) in &config.nodes {
        let started = Instant::now();
        let result: Result<String> = if node.capabilities.contains(&Capability::Chat) {
            match ailloy::Client::with_node(id) {
                Ok(client) => {
                    let options = ailloy::ChatOptions::builder().max_tokens(8).build();
                    client
                        .chat_with(
                            &[ailloy::Message::user("Reply with the single word: ok")],
                            &options,
                        )
                        .await
                        .map(|_| "chat".to_string())
                }
                Err(e) => Err(e),
            }
        } else if node.capabilities.contains(&Capability::Embedding) {
            match ailloy::Client::with_node(id) {
                Ok(client) => client
                    .embed_one("ping")
                    .await
                    .map(|_| "embedding".to_string()),
                Err(e) => Err(e),
            }
        } else {
            println!("  {} {} (no testable capability)", "-".dimmed(), id);
            continue;
        };
        match result {
            Ok(kind) => println!(
                "  {} {} ({kind}, {} ms)",
                "✓".green().bold(),
                id.bold(),
                started.elapsed().as_millis()
            ),
            Err(e) => {
                failures += 1;
                println!("  {} {} — {e:#}", "✗".red().bold(), id.bold());
            }
        }
    }
    if failures > 0 {
        anyhow::bail!("{failures} node(s) failed");
    }
    Ok(())
}

/// Write a starter `.ailloy.yaml` for folder-local configuration.
fn run_init_local(extends_global: bool) -> Result<()> {
    let path = std::path::Path::new(".ailloy.yaml");
    if path.exists() {
        anyhow::bail!(".ailloy.yaml already exists in this directory");
    }
    let content = if extends_global {
        "# Folder-local ailloy configuration for this repository.\n\
         # `extends: global` merges these entries OVER the machine-wide config;\n\
         # remove it to make this file the complete configuration here.\n\
         extends: global\n\
         defaults:\n\
         #  chat: openai/gpt-5.4-mini\n\
         nodes: {}\n"
    } else {
        "# Folder-local ailloy configuration for this repository.\n\
         # The closest .ailloy.yaml wins entirely: only the nodes and defaults\n\
         # in this file exist here (the machine-wide config is ignored, except\n\
         # tool consents). Add `extends: global` to merge instead.\n\
         nodes:\n\
         #  openai/gpt-5.4-mini:\n\
         #    provider: openai\n\
         #    model: gpt-5.4-mini\n\
         #    auth:\n\
         #      env: OPENAI_API_KEY\n\
         #    capabilities: [chat]\n\
         defaults: {}\n"
    };
    std::fs::write(path, content)?;
    println!(
        "{} wrote .ailloy.yaml ({}) — commit it to share the setup, or add it to .gitignore to keep it personal",
        "✓".green().bold(),
        if extends_global {
            "extends global"
        } else {
            "replaces global here"
        }
    );
    Ok(())
}
