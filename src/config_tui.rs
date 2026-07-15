//! Shared configuration entry points for ailloy and consumer projects.
//!
//! Gated behind the `config-tui` feature. The interactive configuration UI is
//! the full-screen ratatui dashboard in [`crate::tui`]; this module keeps the
//! stable public surface consumer tools depend on: enable/disable, status and
//! node printing, connectivity testing, consent helpers, and thin wrappers that
//! launch the dashboard or a single add/edit-node form.

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::{ALL_CAPABILITIES, AiNode, Auth, Capability, Config};

// ---------------------------------------------------------------------------
// Consent
// ---------------------------------------------------------------------------

/// Result of a consent prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsentResult {
    /// User agreed and wants the choice remembered.
    AllowAndRemember,
    /// User agreed for this session only.
    AllowOnce,
    /// User declined.
    Denied,
}

/// Check whether the user has already consented (or declined) for the given key.
pub fn check_consent(config: &Config, key: &str) -> Option<bool> {
    config.consents.get(key).copied()
}

/// Prompt the user for consent to use an external CLI tool (non-TUI, stdin).
///
/// The interactive dashboard gates discovery consent with its own modal; this
/// stdin-based prompt is the fallback for consumer tools calling
/// [`ensure_consent`] outside a full-screen UI.
pub fn prompt_consent(tool_name: &str, description: &str) -> Result<ConsentResult> {
    use std::io::{self, Write};

    println!("Allow ailloy to use {tool_name} to {description}?");
    print!("  [y] yes, and remember   [o] yes, once   [N] no: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read consent response from stdin")?;

    Ok(match input.trim().to_lowercase().as_str() {
        "y" | "yes" => ConsentResult::AllowAndRemember,
        "o" | "once" => ConsentResult::AllowOnce,
        _ => ConsentResult::Denied,
    })
}

/// Ensure consent for an external tool: check existing decision, prompt if needed.
///
/// Returns `true` if the tool may be used, `false` if denied. If
/// `AllowAndRemember`, inserts `true` into `config.consents` (persisted on the
/// next `config.save()`).
pub fn ensure_consent(
    config: &mut Config,
    key: &str,
    tool_name: &str,
    description: &str,
) -> Result<bool> {
    if let Some(allowed) = check_consent(config, key) {
        return Ok(allowed);
    }

    match prompt_consent(tool_name, description)? {
        ConsentResult::AllowAndRemember => {
            config.consents.insert(key.to_string(), true);
            Ok(true)
        }
        ConsentResult::AllowOnce => Ok(true),
        ConsentResult::Denied => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// Enable / disable
// ---------------------------------------------------------------------------

/// Path to the marker file that disables AI for an app.
pub fn disabled_marker_path(app_name: &str) -> Result<std::path::PathBuf> {
    Ok(Config::config_dir()?.join(format!("{}.disabled", app_name)))
}

/// Enable AI features for an app (remove disabled marker).
pub fn enable_ai(app_name: &str) -> Result<()> {
    let path = disabled_marker_path(app_name)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove disabled marker at {}", path.display()))?;
    }
    println!("{} AI enabled for {}", "✓".green().bold(), app_name);
    Ok(())
}

/// Disable AI features for an app (create disabled marker).
pub fn disable_ai(app_name: &str) -> Result<()> {
    let path = disabled_marker_path(app_name)?;
    let dir = Config::config_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config directory {}", dir.display()))?;
    std::fs::write(&path, "")
        .with_context(|| format!("Failed to create disabled marker at {}", path.display()))?;
    println!("{} AI disabled for {}", "✓".green().bold(), app_name);
    Ok(())
}

/// Check whether AI is disabled for an app.
pub fn is_ai_disabled(app_name: &str) -> bool {
    disabled_marker_path(app_name).is_ok_and(|p| p.exists())
}

/// Check whether AI is active (configured and not disabled) for an app.
pub fn is_ai_active(app_name: &str) -> bool {
    if is_ai_disabled(app_name) {
        return false;
    }
    Config::load().is_ok_and(|c| !c.nodes.is_empty())
}

// ---------------------------------------------------------------------------
// Status display
// ---------------------------------------------------------------------------

/// Print AI status for an app, showing each capability's configured node.
pub fn print_ai_status(app_name: &str, capabilities: &[&str]) -> Result<()> {
    let config = Config::load()?;

    println!("{}", "AI Status".bold());
    println!();
    if config.source != crate::config::ConfigSource::Global {
        println!("  {} {}", "Config:".dimmed(), config.source);
        println!(
            "  {}",
            "note: `ailloy ai config` edits the GLOBAL config; edit the local file directly"
                .dimmed()
        );
        println!();
    }

    for &cap_key in capabilities {
        let label = ALL_CAPABILITIES
            .iter()
            .find(|(k, _)| *k == cap_key)
            .map(|(_, l)| *l)
            .unwrap_or(cap_key);

        match config.defaults.get(cap_key) {
            Some(node_id) => {
                if let Some((_, node)) = config.get_node(node_id) {
                    println!(
                        "  {} {}: {} ({}, {})",
                        "✓".green().bold(),
                        label,
                        node_id.bold(),
                        node.provider.to_string().dimmed(),
                        node.detail(),
                    );
                    if let Some(model) = node.model.as_deref().or(node.deployment.as_deref()) {
                        if let Some(warning) = crate::retirement::retirement_warning(model) {
                            println!("    {} {}", "⚠".yellow().bold(), warning.yellow());
                        }
                    }
                } else {
                    println!(
                        "  {} {}: {} {}",
                        "✗".red(),
                        label,
                        node_id,
                        "(node not found)".dimmed(),
                    );
                }
            }
            None => {
                println!("  {} {}: {}", "✗".red(), label, "not configured".dimmed(),);
            }
        }
    }

    println!();
    if is_ai_disabled(app_name) {
        println!(
            "  AI is {}. Run '{} ai enable' to re-enable.",
            "disabled".red().bold(),
            app_name,
        );
    } else if config.nodes.is_empty() {
        println!(
            "  No nodes configured. Run '{} ai config' to set up.",
            app_name,
        );
    } else {
        println!("  AI is {}.", "enabled".green().bold());
    }

    Ok(())
}

/// Print detailed information about a single node.
pub fn print_node_info(id: &str, node: &AiNode, config: &Config) {
    println!("{}", id.bold());
    println!("  {} {}", "Provider:".dimmed(), node.provider);
    if let Some(alias) = &node.alias {
        println!("  {} {}", "Alias:".dimmed(), alias);
    }
    if !node.capabilities.is_empty() {
        let caps: Vec<_> = node.capabilities.iter().map(|c| c.to_string()).collect();
        println!("  {} {}", "Capabilities:".dimmed(), caps.join(", "));
    }
    if let Some(model) = &node.model {
        println!("  {} {}", "Model:".dimmed(), model);
    }
    if let Some(endpoint) = &node.endpoint {
        println!("  {} {}", "Endpoint:".dimmed(), endpoint);
    }
    if let Some(deployment) = &node.deployment {
        println!("  {} {}", "Deployment:".dimmed(), deployment);
    }
    if let Some(api_version) = &node.api_version {
        println!("  {} {}", "API version:".dimmed(), api_version);
    }
    if let Some(binary) = &node.binary {
        println!("  {} {}", "Binary:".dimmed(), binary);
    }
    if let Some(project) = &node.project {
        println!("  {} {}", "Project:".dimmed(), project);
    }
    if let Some(location) = &node.location {
        println!("  {} {}", "Location:".dimmed(), location);
    }
    match &node.auth {
        Some(Auth::Env(var)) => println!("  {} env: {}", "Auth:".dimmed(), var),
        Some(Auth::ApiKey(_)) => println!("  {} api_key: ********", "Auth:".dimmed()),
        Some(Auth::Keychain(_)) => println!("  {} OS keychain", "Auth:".dimmed()),
        Some(Auth::AzureCli(_)) => println!("  {} azure_cli", "Auth:".dimmed()),
        Some(Auth::GcloudCli(_)) => println!("  {} gcloud_cli", "Auth:".dimmed()),
        None => {}
    }

    // Show if this node is a default for any capability
    for (cap, default_id) in &config.defaults {
        if default_id == id {
            println!("  {} default for '{}'", "★".green().bold(), cap);
        }
    }
}

/// List all configured nodes, grouped by capability.
pub fn print_nodes_list(config: &Config) -> Result<()> {
    if config.nodes.is_empty() {
        println!("{}", "No nodes configured.".dimmed());
        println!("Run {} to add one.", "'ai config'".bold());
        println!();
        println!("{}", "Available Provider Types".bold());
        println!();
        println!("  {} — OpenAI API (GPT-4o, etc.)", "openai".bold());
        println!(
            "  {} — Anthropic API (Claude Sonnet, etc.)",
            "anthropic".bold()
        );
        println!("  {} — Azure OpenAI Service", "azure-openai".bold());
        println!(
            "  {} — Microsoft Foundry (GPT, Llama, Mistral, etc.)",
            "microsoft-foundry".bold()
        );
        println!("  {} — Google Vertex AI (Gemini, etc.)", "vertex-ai".bold());
        println!("  {} — Local LLMs via Ollama", "ollama".bold());
        println!(
            "  {} — CLI agents (Claude, Codex, Copilot)",
            "local-agent".bold()
        );
        return Ok(());
    }

    println!("{}", "Configured Nodes".bold());
    println!();

    for &(cap_key, cap_label) in ALL_CAPABILITIES {
        let default_id = config.defaults.get(cap_key);
        let cap: Capability = cap_key.parse().unwrap();
        let nodes: Vec<_> = config.nodes_for_capability(&cap);

        if nodes.is_empty() {
            continue;
        }

        println!("  {} {}", cap_label.bold(), "Nodes:".dimmed());

        for (id, node) in &nodes {
            let is_default = default_id.is_some_and(|d| d == *id);
            let marker = if is_default {
                " (default)".green().to_string()
            } else {
                String::new()
            };
            let alias = node
                .alias
                .as_ref()
                .map(|a| format!(" [{}]", a))
                .unwrap_or_default();

            println!(
                "    {} ({}, {}){}{}",
                id.bold(),
                node.provider.to_string().dimmed(),
                node.detail(),
                alias.dimmed(),
                marker,
            );
        }
        println!();
    }

    // Show nodes that don't match any capability
    let uncategorized: Vec<_> = config
        .nodes
        .iter()
        .filter(|(_, n)| n.capabilities.is_empty())
        .collect();
    if !uncategorized.is_empty() {
        println!("  {} {}", "Uncategorized".bold(), "Nodes:".dimmed());
        for (id, node) in &uncategorized {
            println!(
                "    {} ({}, {})",
                id.bold(),
                node.provider.to_string().dimmed(),
                node.detail(),
            );
        }
        println!();
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Test chat
// ---------------------------------------------------------------------------

/// Run a test chat message against the default chat node.
pub async fn run_test_chat(app_name: &str, message: Option<String>) -> Result<()> {
    let msg = message.unwrap_or_else(|| "Say hello in one sentence.".to_string());

    if is_ai_disabled(app_name) {
        println!(
            "{} AI is disabled for {}. Run '{} ai enable' to re-enable.",
            "✗".red().bold(),
            app_name,
            app_name
        );
        return Ok(());
    }

    let config = Config::load()?;
    let (node_id, node) = config.default_chat_node()?;

    println!("{}", format!("Testing chat with {}...", node_id).dimmed());

    let client = crate::client::Client::from_node(node)?;
    let response = client.chat(&[crate::types::Message::user(&msg)]).await?;

    println!("{} {}", "✓".green().bold(), response.content);
    Ok(())
}

// ---------------------------------------------------------------------------
// Reset
// ---------------------------------------------------------------------------

/// Delete the global ailloy config file (asks for confirmation on stdin).
pub fn reset_config() -> Result<()> {
    let path = Config::config_path()?;
    if path.exists() {
        if !stdin_confirm("Delete all AI configuration?")? {
            return Ok(());
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove config at {}", path.display()))?;
        println!("{} Configuration reset", "✓".green().bold());
    } else {
        println!("{}", "No configuration file found.".dimmed());
    }
    Ok(())
}

/// A minimal stdin yes/no confirmation (default: no).
fn stdin_confirm(message: &str) -> Result<bool> {
    use std::io::{self, Write};
    print!("{message} [y/N]: ");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read confirmation from stdin")?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

// ---------------------------------------------------------------------------
// Interactive configuration (ratatui dashboard)
// ---------------------------------------------------------------------------

/// Launch the interactive configuration dashboard.
///
/// On a TTY this opens the full-screen ratatui dashboard (which loads and saves
/// the global config itself). Without a TTY it prints the current status and
/// exits. Returns `false` — the dashboard persists its own changes, so the
/// caller has nothing further to save.
pub async fn run_interactive_config(
    config: &mut Config,
    capability_columns: &[&str],
) -> Result<bool> {
    use std::io::IsTerminal;

    // The dashboard operates on the global config directly; the passed-in
    // config is not used (kept for backward-compatible signature).
    let _ = config;

    if std::io::stdout().is_terminal() {
        crate::tui::run("ailloy").await?;
    } else {
        print_ai_status("ailloy", capability_columns)?;
    }
    Ok(false)
}

/// Add a node interactively via a single ratatui add-node form.
///
/// Returns the saved node id, or `None` if the user cancelled.
pub async fn add_node_interactive(config: &mut Config) -> Result<Option<String>> {
    crate::tui::run_single_form("ailloy", config, None).await
}

/// Edit an existing node interactively via a single ratatui edit-node form.
pub async fn edit_node_interactive(config: &mut Config, id_or_alias: &str) -> Result<()> {
    let canonical_id = config
        .resolve_node(id_or_alias)
        .map(|s| s.to_string())
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;

    crate::tui::run_single_form("ailloy", config, Some(canonical_id)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_consent_none() {
        let config = Config::default();
        assert_eq!(check_consent(&config, "azure-cli"), None);
    }

    #[test]
    fn test_check_consent_allowed() {
        let mut config = Config::default();
        config.consents.insert("azure-cli".to_string(), true);
        assert_eq!(check_consent(&config, "azure-cli"), Some(true));
    }

    #[test]
    fn test_check_consent_denied() {
        let mut config = Config::default();
        config.consents.insert("azure-cli".to_string(), false);
        assert_eq!(check_consent(&config, "azure-cli"), Some(false));
    }

    #[test]
    fn ensure_consent_returns_stored_decision_without_prompting() {
        let mut config = Config::default();
        config.consents.insert("azure-cli".to_string(), true);
        assert!(ensure_consent(&mut config, "azure-cli", "az", "discover").unwrap());
        config.consents.insert("azure-cli".to_string(), false);
        assert!(!ensure_consent(&mut config, "azure-cli", "az", "discover").unwrap());
    }
}
