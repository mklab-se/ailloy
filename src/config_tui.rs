//! Shared interactive configuration TUI for ailloy and consumer projects.
//!
//! Gated behind the `config-tui` feature. Provides interactive wizards,
//! status display, enable/disable, test, and consent functions that any
//! project using ailloy can reuse.

use std::cell::Cell;
use std::future::Future;
use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use colored::Colorize;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{self, ClearType};
use crossterm::{cursor, execute, queue};
use inquire::Text;

use crate::azure_discover;
use crate::config::{
    ALL_CAPABILITIES, AiNode, Auth, Capability, Config, ProviderKind, consent_keys,
};

// ---------------------------------------------------------------------------
// TUI mode flag
// ---------------------------------------------------------------------------

thread_local! {
    static TUI_MODE: Cell<bool> = const { Cell::new(false) };
}

fn is_tui_mode() -> bool {
    TUI_MODE.with(|f| f.get())
}

fn set_tui_mode(active: bool) {
    TUI_MODE.with(|f| f.set(active));
}

// ---------------------------------------------------------------------------
// Crossterm TUI widgets (used when in TUI mode)
// ---------------------------------------------------------------------------

/// Select from a list of options using crossterm. Returns selected index or None on cancel.
fn tui_select(title: &str, options: &[String], default_idx: usize) -> Result<Option<usize>> {
    let mut stdout = std::io::stdout();
    let mut selected = default_idx.min(options.len().saturating_sub(1));

    loop {
        queue!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        queue!(
            stdout,
            crossterm::style::Print(title),
            cursor::MoveToNextLine(2)
        )?;

        for (i, opt) in options.iter().enumerate() {
            if i == selected {
                queue!(
                    stdout,
                    crossterm::style::Print("  > "),
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                    crossterm::style::Print(opt),
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                    cursor::MoveToNextLine(1)
                )?;
            } else {
                queue!(
                    stdout,
                    crossterm::style::Print("    "),
                    crossterm::style::Print(opt),
                    cursor::MoveToNextLine(1)
                )?;
            }
        }

        queue!(
            stdout,
            cursor::MoveToNextLine(1),
            crossterm::style::SetAttribute(crossterm::style::Attribute::Dim),
            crossterm::style::Print("\u{2191}\u{2193} Navigate  Enter Select  Esc Cancel"),
            crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
            cursor::MoveToNextLine(1)
        )?;
        stdout.flush()?;

        let ev = event::read()?;
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    selected = selected.saturating_sub(1);
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if !options.is_empty() && selected < options.len() - 1 {
                        selected += 1;
                    }
                }
                (KeyCode::Enter, _) => {
                    if options.is_empty() {
                        return Ok(None);
                    }
                    return Ok(Some(selected));
                }
                _ => {}
            }
        }
    }
}

/// Text input using crossterm. Returns entered text or None on cancel.
fn tui_text(label: &str, default: &str, help: Option<&str>) -> Result<Option<String>> {
    let mut stdout = std::io::stdout();
    let mut input = default.to_string();

    execute!(stdout, cursor::Show)?;

    loop {
        queue!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        queue!(
            stdout,
            crossterm::style::Print(label),
            cursor::MoveToNextLine(1)
        )?;

        if let Some(h) = help {
            queue!(
                stdout,
                crossterm::style::SetAttribute(crossterm::style::Attribute::Dim),
                crossterm::style::Print(h),
                crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                cursor::MoveToNextLine(1)
            )?;
        }

        queue!(
            stdout,
            cursor::MoveToNextLine(1),
            crossterm::style::Print(format!("> {}", input)),
            cursor::MoveToNextLine(2),
            crossterm::style::SetAttribute(crossterm::style::Attribute::Dim),
            crossterm::style::Print("Enter Confirm  Esc Cancel"),
            crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
            cursor::MoveToNextLine(1)
        )?;
        stdout.flush()?;

        let ev = event::read()?;
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    execute!(stdout, cursor::Hide)?;
                    return Ok(None);
                }
                (KeyCode::Enter, _) => {
                    execute!(stdout, cursor::Hide)?;
                    return Ok(Some(input));
                }
                (KeyCode::Backspace, _) => {
                    input.pop();
                }
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    input.push(c);
                }
                _ => {}
            }
        }
    }
}

/// Multi-select using crossterm. Returns selected indices or None on cancel.
fn tui_multi_select(
    title: &str,
    options: &[String],
    defaults: &[bool],
) -> Result<Option<Vec<usize>>> {
    let mut stdout = std::io::stdout();
    let mut selected: usize = 0;
    let mut checked: Vec<bool> = if defaults.len() == options.len() {
        defaults.to_vec()
    } else {
        vec![false; options.len()]
    };

    loop {
        queue!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        queue!(
            stdout,
            crossterm::style::Print(title),
            cursor::MoveToNextLine(2)
        )?;

        for (i, opt) in options.iter().enumerate() {
            let check = if checked[i] { "[\u{2713}]" } else { "[ ]" };
            if i == selected {
                queue!(
                    stdout,
                    crossterm::style::Print("  > "),
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                    crossterm::style::Print(format!("{} {}", check, opt)),
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                    cursor::MoveToNextLine(1)
                )?;
            } else {
                queue!(
                    stdout,
                    crossterm::style::Print(format!("    {} {}", check, opt)),
                    cursor::MoveToNextLine(1)
                )?;
            }
        }

        queue!(
            stdout,
            cursor::MoveToNextLine(1),
            crossterm::style::SetAttribute(crossterm::style::Attribute::Dim),
            crossterm::style::Print(
                "\u{2191}\u{2193} Navigate  Space Toggle  Enter Confirm  Esc Cancel"
            ),
            crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
            cursor::MoveToNextLine(1)
        )?;
        stdout.flush()?;

        let ev = event::read()?;
        if let Event::Key(key) = ev {
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    selected = selected.saturating_sub(1);
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if !options.is_empty() && selected < options.len() - 1 {
                        selected += 1;
                    }
                }
                (KeyCode::Char(' '), _) => {
                    if !options.is_empty() {
                        checked[selected] = !checked[selected];
                    }
                }
                (KeyCode::Enter, _) => {
                    let indices: Vec<usize> = checked
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| **c)
                        .map(|(i, _)| i)
                        .collect();
                    return Ok(Some(indices));
                }
                _ => {}
            }
        }
    }
}

/// Confirm dialog using crossterm. Returns bool or None on cancel.
fn tui_confirm(message: &str, default_yes: bool) -> Result<Option<bool>> {
    let options = vec!["Yes".to_string(), "No".to_string()];
    let default_idx = if default_yes { 0 } else { 1 };
    let result = tui_select(message, &options, default_idx)?;
    Ok(result.map(|idx| idx == 0))
}

/// Show a message on the TUI screen (e.g., loading states).
fn tui_show_message(message: &str) -> Result<()> {
    let mut stdout = std::io::stdout();
    queue!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        crossterm::style::Print(message),
        cursor::MoveToNextLine(1)
    )?;
    stdout.flush()?;
    Ok(())
}

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

/// Prompt the user for consent to use an external CLI tool.
pub fn prompt_consent(tool_name: &str, description: &str) -> Result<ConsentResult> {
    let message = format!("Allow ailloy to use {} to {}?", tool_name, description);

    let options: Vec<String> = vec![
        "Yes, and remember my choice".into(),
        "Yes, just this once".into(),
        "No, I'll configure manually".into(),
    ];

    let Some(idx) = prompt_select(&message, &options)? else {
        return Ok(ConsentResult::Denied);
    };

    Ok(match idx {
        0 => ConsentResult::AllowAndRemember,
        1 => ConsentResult::AllowOnce,
        _ => ConsentResult::Denied,
    })
}

/// Ensure consent for an external tool: check existing decision, prompt if needed.
///
/// Returns `true` if the tool may be used, `false` if denied.
/// If `AllowAndRemember`, inserts `true` into `config.consents` (persisted on next `config.save()`).
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
// Dual-mode prompt helpers (TUI widgets when in TUI mode, inquire otherwise)
// ---------------------------------------------------------------------------

/// Select from a list of options. Returns the index, or None if cancelled.
fn prompt_select(message: &str, options: &[String]) -> Result<Option<usize>> {
    if is_tui_mode() {
        return tui_select(message, options, 0);
    }
    match inquire::Select::new(message, options.to_vec()).prompt() {
        Ok(selected) => Ok(options.iter().position(|o| *o == selected)),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Select with a pre-selected default index.
fn prompt_select_with_default(
    message: &str,
    options: &[String],
    default: usize,
) -> Result<Option<usize>> {
    if is_tui_mode() {
        return tui_select(message, options, default);
    }
    match inquire::Select::new(message, options.to_vec())
        .with_starting_cursor(default)
        .prompt()
    {
        Ok(selected) => Ok(options.iter().position(|o| *o == selected)),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Confirm prompt. Returns false on cancel.
fn prompt_confirm(message: &str) -> Result<bool> {
    if is_tui_mode() {
        return Ok(tui_confirm(message, false)?.unwrap_or(false));
    }
    match inquire::Confirm::new(message).with_default(false).prompt() {
        Ok(val) => Ok(val),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Text input prompt. Returns the entered string or error on cancel.
fn prompt_text(label: &str, default: &str, help: Option<&str>) -> Result<String> {
    if is_tui_mode() {
        return tui_text(label, default, help)?.ok_or_else(|| anyhow::anyhow!("Cancelled"));
    }
    let mut prompt = Text::new(label);
    if !default.is_empty() {
        prompt = prompt.with_default(default);
    }
    if let Some(h) = help {
        prompt = prompt.with_help_message(h);
    }
    Ok(prompt.prompt()?)
}

/// Multi-select prompt. Returns selected option strings (empty vec on cancel).
fn prompt_multi_select(message: &str, options: &[String]) -> Result<Vec<String>> {
    if is_tui_mode() {
        let defaults = vec![false; options.len()];
        let result = tui_multi_select(message, options, &defaults)?;
        return Ok(result
            .unwrap_or_default()
            .into_iter()
            .map(|i| options[i].clone())
            .collect());
    }
    match inquire::MultiSelect::new(message, options.to_vec()).prompt() {
        Ok(selected) => Ok(selected),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------
// Enable / Disable
// ---------------------------------------------------------------------------

/// Returns the path to the disabled marker file for an app.
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
        println!("  {} — Local LLMs via LM Studio", "lm-studio".bold());
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

/// Delete the global ailloy config file.
pub fn reset_config() -> Result<()> {
    let path = Config::config_path()?;
    if path.exists() {
        if !prompt_confirm("Delete all AI configuration?")? {
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

// ---------------------------------------------------------------------------
// Interactive config wizard
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Table-based interactive config TUI (crossterm)
// ---------------------------------------------------------------------------

/// RAII guard for raw mode + alternate screen.
struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let mut stdout = std::io::stdout();
        let _ = execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Render the table of nodes to the terminal.
fn render_table(
    stdout: &mut impl Write,
    config: &Config,
    capability_columns: &[&str],
    selected_row: usize,
) -> Result<()> {
    queue!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    let (term_width, _) = terminal::size().unwrap_or((80, 24));
    let tw = term_width as usize;

    let node_ids: Vec<&str> = config.nodes.keys().map(|s| s.as_str()).collect();

    // Column widths: fixed capability columns, flexible name columns
    let cap_col_w = 10usize;
    let right_margin = 2usize;
    let total_cap_w = cap_col_w * capability_columns.len();
    let remaining = tw.saturating_sub(total_cap_w + right_margin + 4); // 4 for left padding + gaps
    let name_w = remaining * 40 / 100;
    let provider_w = remaining * 25 / 100;
    let model_w = remaining.saturating_sub(name_w + provider_w);

    // Header
    let mut header = format!(
        " {:<name_w$}  {:<provider_w$}  {:<model_w$}",
        "AI Nodes",
        "Provider",
        "Model Name",
        name_w = name_w,
        provider_w = provider_w,
        model_w = model_w,
    );
    for &cap_key in capability_columns {
        let label = ALL_CAPABILITIES
            .iter()
            .find(|(k, _)| *k == cap_key)
            .map(|(_, l)| *l)
            .unwrap_or(cap_key);
        // Truncate label to fit column
        let display: String = label.chars().take(cap_col_w - 1).collect();
        header.push_str(&format!("{:<w$}", display, w = cap_col_w));
    }
    // Truncate header to terminal width
    let header: String = header.chars().take(tw).collect();
    queue!(
        stdout,
        crossterm::style::Print(&header),
        cursor::MoveToNextLine(1)
    )?;

    // Separator
    let sep: String = "─".repeat(tw);
    queue!(
        stdout,
        crossterm::style::Print(&sep),
        cursor::MoveToNextLine(1)
    )?;

    if node_ids.is_empty() {
        queue!(
            stdout,
            cursor::MoveToNextLine(1),
            crossterm::style::Print(" No nodes configured. Press [A] to add one."),
            cursor::MoveToNextLine(1)
        )?;
    } else {
        for (i, id) in node_ids.iter().enumerate() {
            let node = &config.nodes[*id];
            let provider_str = node.provider.to_string();
            let model_str = node.detail();

            let mut row = format!(
                " {:<name_w$}  {:<provider_w$}  {:<model_w$}",
                id,
                provider_str,
                model_str,
                name_w = name_w,
                provider_w = provider_w,
                model_w = model_w,
            );

            for &cap_key in capability_columns {
                let cap: Capability = match cap_key.parse() {
                    Ok(c) => c,
                    Err(_) => {
                        row.push_str(&format!("{:<w$}", "", w = cap_col_w));
                        continue;
                    }
                };
                let is_default = config.defaults.get(cap_key).is_some_and(|d| d == *id);
                let has_cap = node.has_capability(&cap);
                let marker = if is_default {
                    "*"
                } else if has_cap {
                    "✓"
                } else {
                    ""
                };
                row.push_str(&format!("{:<w$}", marker, w = cap_col_w));
            }

            let row: String = row.chars().take(tw).collect();

            if i == selected_row {
                queue!(
                    stdout,
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                    crossterm::style::Print(&row),
                    crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                    cursor::MoveToNextLine(1)
                )?;
            } else {
                queue!(
                    stdout,
                    crossterm::style::Print(&row),
                    cursor::MoveToNextLine(1)
                )?;
            }
        }
    }

    // Footer
    queue!(
        stdout,
        cursor::MoveToNextLine(1),
        crossterm::style::Print(" [A]dd  [Enter] Edit  [D]elete  [Q]uit"),
        cursor::MoveToNextLine(1)
    )?;

    stdout.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Form-based node editor
// ---------------------------------------------------------------------------

/// Items in the edit form.
#[derive(Clone)]
enum FormItem {
    /// Editable text field: (label, current_value)
    TextField { label: &'static str, value: String },
    /// Auth field (special editing flow)
    AuthField { display: String },
    /// Section header (not selectable)
    Header(String),
    /// Capability toggle: (capability, currently_enabled)
    CapabilityToggle { cap: Capability, enabled: bool },
    /// Default toggle: (capability key, capability label, currently_default)
    DefaultToggle {
        cap_key: String,
        cap_label: String,
        is_default: bool,
    },
    /// Action button: (label, is_save)
    Action { label: String, is_save: bool },
}

impl FormItem {
    fn is_selectable(&self) -> bool {
        !matches!(self, FormItem::Header(_))
    }
}

/// Build form items from the current node state.
fn build_form_items(config: &Config, node_id: &str, node: &AiNode) -> Vec<FormItem> {
    let mut items = Vec::new();

    // Text/auth fields based on provider
    let fields = editable_fields(node);
    for (label, value) in &fields {
        if *label == "Auth" {
            items.push(FormItem::AuthField {
                display: value.clone().unwrap_or_else(|| "(not set)".to_string()),
            });
        } else if *label == "Capabilities" {
            // Skip — we handle capabilities as toggles below
        } else {
            items.push(FormItem::TextField {
                label,
                value: value.clone().unwrap_or_default(),
            });
        }
    }

    // Capability toggles
    let supported = node.provider.supported_capabilities();
    if supported.len() > 1 {
        items.push(FormItem::Header("Capabilities".to_string()));
        for cap in &supported {
            let enabled = node.capabilities.contains(cap);
            items.push(FormItem::CapabilityToggle {
                cap: cap.clone(),
                enabled,
            });
        }
    }

    // Default toggles (only for capabilities this node has)
    let node_caps: Vec<Capability> = node.capabilities.clone();
    if !node_caps.is_empty() {
        items.push(FormItem::Header("Defaults".to_string()));
        for cap in &node_caps {
            let cap_key = cap.config_key().to_string();
            let is_default = config.defaults.get(&cap_key).is_some_and(|d| d == node_id);
            items.push(FormItem::DefaultToggle {
                cap_key,
                cap_label: cap.label().to_string(),
                is_default,
            });
        }
    }

    // Actions
    items.push(FormItem::Header(String::new())); // blank separator
    items.push(FormItem::Action {
        label: "Save".to_string(),
        is_save: true,
    });
    items.push(FormItem::Action {
        label: "Cancel".to_string(),
        is_save: false,
    });

    items
}

/// Render the edit form.
fn render_form(
    stdout: &mut impl Write,
    node_id: &str,
    items: &[FormItem],
    selected: usize,
) -> Result<()> {
    queue!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // Title
    queue!(
        stdout,
        crossterm::style::Print(format!(" Edit: {}", node_id)),
        cursor::MoveToNextLine(1),
        crossterm::style::SetAttribute(crossterm::style::Attribute::Dim),
        crossterm::style::Print("\u{2500}".repeat(50)),
        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
        cursor::MoveToNextLine(1)
    )?;

    // Find selectable indices for mapping
    let selectable_indices: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.is_selectable())
        .map(|(i, _)| i)
        .collect();

    let selected_item_idx = selectable_indices.get(selected).copied().unwrap_or(0);

    for (i, item) in items.iter().enumerate() {
        let is_selected = i == selected_item_idx;

        match item {
            FormItem::TextField { label, value } => {
                let display_val = if value.is_empty() {
                    "(not set)".to_string()
                } else {
                    value.clone()
                };
                let line = format!("   {:<18} {}", format!("{}:", label), display_val);
                if is_selected {
                    queue!(
                        stdout,
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                        crossterm::style::Print(&line),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                        cursor::MoveToNextLine(1)
                    )?;
                } else {
                    queue!(
                        stdout,
                        crossterm::style::Print(&line),
                        cursor::MoveToNextLine(1)
                    )?;
                }
            }
            FormItem::AuthField { display } => {
                let line = format!("   {:<18} {}", "Auth:", display);
                if is_selected {
                    queue!(
                        stdout,
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                        crossterm::style::Print(&line),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                        cursor::MoveToNextLine(1)
                    )?;
                } else {
                    queue!(
                        stdout,
                        crossterm::style::Print(&line),
                        cursor::MoveToNextLine(1)
                    )?;
                }
            }
            FormItem::Header(text) => {
                if text.is_empty() {
                    queue!(stdout, cursor::MoveToNextLine(1))?;
                } else {
                    queue!(
                        stdout,
                        cursor::MoveToNextLine(1),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Bold),
                        crossterm::style::Print(format!("   {}", text)),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                        cursor::MoveToNextLine(1)
                    )?;
                }
            }
            FormItem::CapabilityToggle { cap, enabled } => {
                let check = if *enabled { "[\u{2713}]" } else { "[ ]" };
                let line = format!("   {} {}", check, cap.label());
                if is_selected {
                    queue!(
                        stdout,
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                        crossterm::style::Print(&line),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                        cursor::MoveToNextLine(1)
                    )?;
                } else {
                    queue!(
                        stdout,
                        crossterm::style::Print(&line),
                        cursor::MoveToNextLine(1)
                    )?;
                }
            }
            FormItem::DefaultToggle {
                cap_label,
                is_default,
                ..
            } => {
                let check = if *is_default { "[\u{2713}]" } else { "[ ]" };
                let line = format!("   {} Default for {}", check, cap_label);
                if is_selected {
                    queue!(
                        stdout,
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                        crossterm::style::Print(&line),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                        cursor::MoveToNextLine(1)
                    )?;
                } else {
                    queue!(
                        stdout,
                        crossterm::style::Print(&line),
                        cursor::MoveToNextLine(1)
                    )?;
                }
            }
            FormItem::Action { label, .. } => {
                let line = format!("   [{}]", label);
                if is_selected {
                    queue!(
                        stdout,
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reverse),
                        crossterm::style::Print(&line),
                        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
                        cursor::MoveToNextLine(1)
                    )?;
                } else {
                    queue!(
                        stdout,
                        crossterm::style::Print(&line),
                        cursor::MoveToNextLine(1)
                    )?;
                }
            }
        }
    }

    // Footer
    queue!(
        stdout,
        cursor::MoveToNextLine(1),
        crossterm::style::SetAttribute(crossterm::style::Attribute::Dim),
        crossterm::style::Print(" \u{2191}\u{2193} Navigate  Enter Edit  Space Toggle  Esc Cancel"),
        crossterm::style::SetAttribute(crossterm::style::Attribute::Reset),
        cursor::MoveToNextLine(1)
    )?;

    stdout.flush()?;
    Ok(())
}

/// Apply form edits back to config. Returns true if saved.
fn apply_form_to_config(config: &mut Config, node_id: &str, items: &[FormItem]) -> Result<()> {
    let node = config.get_node_mut(node_id).unwrap();

    for item in items {
        match item {
            FormItem::TextField { label, value } => {
                let opt = if value.is_empty() {
                    None
                } else {
                    Some(value.clone())
                };
                match *label {
                    "Model" => node.model = opt,
                    "Endpoint" => node.endpoint = opt,
                    "Deployment" => node.deployment = opt,
                    "API version" => node.api_version = opt,
                    "Binary" => node.binary = opt,
                    "Project" => node.project = opt,
                    "Location" => node.location = opt,
                    _ => {}
                }
            }
            FormItem::CapabilityToggle { cap, enabled } => {
                if *enabled && !node.capabilities.contains(cap) {
                    node.capabilities.push(cap.clone());
                } else if !enabled {
                    node.capabilities.retain(|c| c != cap);
                }
            }
            _ => {} // Auth handled inline, defaults handled separately
        }
    }

    // Apply default toggles
    for item in items {
        if let FormItem::DefaultToggle {
            cap_key,
            is_default,
            ..
        } = item
        {
            if *is_default {
                config.set_default(cap_key, node_id);
            } else if config.defaults.get(cap_key).is_some_and(|d| d == node_id) {
                config.defaults.remove(cap_key);
            }
        }
    }

    config.save()?;
    Ok(())
}

/// Form-based node editor. Returns `true` if config was modified.
fn tui_edit_node(config: &mut Config, node_id: &str) -> Result<bool> {
    let node = config
        .get_node(node_id)
        .map(|(_, n)| n.clone())
        .with_context(|| format!("Node '{}' not found", node_id))?;

    let mut items = build_form_items(config, node_id, &node);
    let selectable_count = items.iter().filter(|i| i.is_selectable()).count();
    let mut selected: usize = 0;
    let mut stdout = std::io::stdout();

    loop {
        render_form(&mut stdout, node_id, &items, selected)?;

        let ev = event::read()?;
        if let Event::Key(key) = ev {
            // Find the actual item index for the current selection
            let selectable_indices: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.is_selectable())
                .map(|(i, _)| i)
                .collect();
            let item_idx = selectable_indices[selected];

            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    return Ok(false);
                }
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    selected = selected.saturating_sub(1);
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if selected < selectable_count - 1 {
                        selected += 1;
                    }
                }
                (KeyCode::Char(' '), _) => match &mut items[item_idx] {
                    FormItem::CapabilityToggle { enabled, .. } => {
                        *enabled = !*enabled;
                    }
                    FormItem::DefaultToggle { is_default, .. } => {
                        *is_default = !*is_default;
                    }
                    _ => {}
                },
                (KeyCode::Enter, _) => {
                    match &items[item_idx] {
                        FormItem::TextField { label, value } => {
                            let new_val = tui_text(
                                &format!("{}:", label),
                                value,
                                Some("Enter new value, or leave empty to clear"),
                            )?;
                            if let Some(v) = new_val {
                                items[item_idx] = FormItem::TextField { label, value: v };
                            }
                        }
                        FormItem::AuthField { .. } => {
                            let auth_options: Vec<String> = vec![
                                "Environment variable".into(),
                                "API key (inline)".into(),
                                "Azure CLI".into(),
                                "gcloud CLI".into(),
                                "None (clear)".into(),
                            ];
                            if let Some(idx) =
                                tui_select("Authentication method", &auth_options, 0)?
                            {
                                let new_auth = match idx {
                                    0 => {
                                        let var = tui_text(
                                            "Environment variable:",
                                            "OPENAI_API_KEY",
                                            None,
                                        )?;
                                        var.map(Auth::Env)
                                    }
                                    1 => {
                                        let key = tui_text("API key:", "", None)?;
                                        key.and_then(|k| {
                                            if k.is_empty() {
                                                None
                                            } else {
                                                Some(Auth::ApiKey(k))
                                            }
                                        })
                                    }
                                    2 => Some(Auth::AzureCli(true)),
                                    3 => Some(Auth::GcloudCli(true)),
                                    _ => None,
                                };
                                // Update the node's auth in config immediately
                                let node_mut = config.get_node_mut(node_id).unwrap();
                                node_mut.auth = new_auth;
                                // Rebuild auth display
                                let display = match &node_mut.auth {
                                    Some(Auth::Env(v)) => format!("env: {}", v),
                                    Some(Auth::ApiKey(_)) => "api_key: ********".to_string(),
                                    Some(Auth::AzureCli(_)) => "azure_cli".to_string(),
                                    Some(Auth::GcloudCli(_)) => "gcloud_cli".to_string(),
                                    None => "(not set)".to_string(),
                                };
                                items[item_idx] = FormItem::AuthField { display };
                            }
                        }
                        FormItem::CapabilityToggle { .. } | FormItem::DefaultToggle { .. } => {
                            // Enter also toggles checkboxes
                            match &mut items[item_idx] {
                                FormItem::CapabilityToggle { enabled, .. } => {
                                    *enabled = !*enabled;
                                }
                                FormItem::DefaultToggle { is_default, .. } => {
                                    *is_default = !*is_default;
                                }
                                _ => unreachable!(),
                            }
                        }
                        FormItem::Action { is_save, .. } => {
                            if *is_save {
                                apply_form_to_config(config, node_id, &items)?;
                                return Ok(true);
                            } else {
                                return Ok(false);
                            }
                        }
                        FormItem::Header(_) => {}
                    }
                }
                _ => {}
            }
        }
    }
}

/// Confirm and delete a node. Returns `true` if deleted.
fn delete_with_confirm(config: &mut Config, node_id: &str) -> Result<bool> {
    if prompt_confirm(&format!("Delete node '{}'?", node_id))? {
        config.remove_node(node_id);
        config.save()?;
        if !is_tui_mode() {
            println!("{} Removed node '{}'", "✓".green().bold(), node_id.bold());
        }
        return Ok(true);
    }
    Ok(false)
}

/// Run the interactive table-based config wizard.
/// `capability_columns` specifies which capability columns to display
/// (e.g., `&["chat", "image"]` for mdeck, `&["chat"]` for hoist).
///
/// Returns `true` if config was modified.
pub async fn run_interactive_config(
    config: &mut Config,
    capability_columns: &[&str],
) -> Result<bool> {
    set_tui_mode(true);
    let mut modified = false;
    let mut selected: usize = 0;

    // Use the RAII guard for safety (ensures cleanup on panic/early return)
    let _guard = RawModeGuard::enter()?;
    let mut stdout = std::io::stdout();

    loop {
        let node_count = config.nodes.len();
        if node_count > 0 && selected >= node_count {
            selected = node_count - 1;
        }

        render_table(&mut stdout, config, capability_columns, selected)?;

        let ev = event::read()?;
        match ev {
            Event::Key(key) => match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => break,
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                    selected = selected.saturating_sub(1);
                }
                (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                    if node_count > 0 && selected < node_count - 1 {
                        selected += 1;
                    }
                }
                (KeyCode::Char('a'), _) => {
                    if let Some(_name) = add_node_full(config).await? {
                        modified = true;
                    }
                }
                (KeyCode::Enter, _) if node_count > 0 => {
                    let id = config.nodes.keys().nth(selected).unwrap().clone();
                    if tui_edit_node(config, &id)? {
                        modified = true;
                    }
                }
                (KeyCode::Char('d'), _) if node_count > 0 => {
                    let id = config.nodes.keys().nth(selected).unwrap().clone();
                    if delete_with_confirm(config, &id)? {
                        modified = true;
                    }
                }
                _ => {} // Other keys just re-render
            },
            Event::Resize(_, _) => {} // re-render on next iteration
            _ => {}
        }
    }

    set_tui_mode(false);
    Ok(modified)
}

/// Full add-node flow: select provider, prompt for details, ask for ID, save.
/// Returns the node ID if successful, None if cancelled.
async fn add_node_full(config: &mut Config) -> Result<Option<String>> {
    let Some((suggested_id, node)) = prompt_node_setup(config).await? else {
        return Ok(None);
    };

    let name = prompt_text("Node ID:", &suggested_id, None)?;

    if config.nodes.contains_key(&name)
        && !prompt_confirm(&format!("Node '{}' already exists. Overwrite?", name))?
    {
        return Ok(None);
    }

    let caps = node.capabilities.clone();
    config.add_node(name.clone(), node);

    // Auto-set as default for capabilities where no default exists
    for cap in &caps {
        let cap_key = cap.config_key();
        if !config.defaults.contains_key(cap_key) {
            config.set_default(cap_key, &name);
        }
    }

    config.save()?;
    Ok(Some(name))
}

/// Public add-node: runs the full add-node flow and returns the node ID.
pub async fn add_node_interactive(config: &mut Config) -> Result<Option<String>> {
    add_node_full(config).await
}

/// Public edit-node: interactive field editor for a node by ID or alias.
pub fn edit_node_interactive(config: &mut Config, id_or_alias: &str) -> Result<()> {
    let canonical_id = config
        .resolve_node(id_or_alias)
        .map(|s| s.to_string())
        .with_context(|| format!("Node '{}' not found", id_or_alias))?;

    edit_node_fields(config, &canonical_id)?;
    config.save()?;
    println!(
        "{} Updated node '{}'",
        "✓".green().bold(),
        canonical_id.bold()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Node setup (provider-specific prompting)
// ---------------------------------------------------------------------------

/// All provider kinds with their display labels.
const PROVIDER_KINDS: &[(&str, ProviderKind)] = &[
    ("OpenAI", ProviderKind::OpenAi),
    ("Anthropic", ProviderKind::Anthropic),
    ("Azure OpenAI", ProviderKind::AzureOpenAi),
    ("Microsoft Foundry", ProviderKind::MicrosoftFoundry),
    ("Google Vertex AI", ProviderKind::VertexAi),
    ("Ollama", ProviderKind::Ollama),
    ("LM Studio", ProviderKind::OpenAi),
    (
        "Local Agent (Claude, Codex, Copilot)",
        ProviderKind::LocalAgent,
    ),
];

/// Returns None if user cancelled.
async fn prompt_node_setup(config: &Config) -> Result<Option<(String, AiNode)>> {
    let kind_options: Vec<(&str, &ProviderKind)> = PROVIDER_KINDS
        .iter()
        .map(|(label, kind)| (*label, kind))
        .collect();

    let labels: Vec<String> = kind_options.iter().map(|(l, _)| l.to_string()).collect();

    let Some(idx) = prompt_select("Select AI provider", &labels)? else {
        return Ok(None);
    };

    let kind_label = kind_options[idx].0;

    // For Azure/Foundry, attempt auto-discovery first
    match kind_label {
        "Azure OpenAI" => {
            let mut config_mut = config.clone();
            let allowed = ensure_consent(
                &mut config_mut,
                consent_keys::AZURE_CLI,
                "the Azure CLI (az)",
                "automatically discover your subscriptions, resources, and deployments",
            )?;

            if allowed {
                match azure_discover_flow().await {
                    Ok(result) => return Ok(Some(result)),
                    Err(e) => {
                        eprintln!(
                            "{} Auto-discovery failed: {}",
                            "Warning:".yellow().bold(),
                            e
                        );
                        eprintln!("Falling back to manual configuration.\n");
                    }
                }
            }
        }
        "Microsoft Foundry" => {
            let mut config_mut = config.clone();
            let allowed = ensure_consent(
                &mut config_mut,
                consent_keys::AZURE_CLI,
                "the Azure CLI (az)",
                "automatically discover your subscriptions, resources, and deployments",
            )?;

            if allowed {
                match foundry_discover_flow().await {
                    Ok(result) => return Ok(Some(result)),
                    Err(e) => {
                        eprintln!(
                            "{} Auto-discovery failed: {}",
                            "Warning:".yellow().bold(),
                            e
                        );
                        eprintln!("Falling back to manual configuration.\n");
                    }
                }
            }
        }
        _ => {}
    }

    prompt_node_for_kind(config, kind_label).await
}

/// Describe an auth configuration for display in the reuse prompt.
fn auth_summary(node: &AiNode) -> String {
    let auth_part = match &node.auth {
        Some(Auth::Env(var)) => format!("env: {}", var),
        Some(Auth::ApiKey(_)) => "api_key: ********".to_string(),
        Some(Auth::AzureCli(_)) => "azure_cli".to_string(),
        Some(Auth::GcloudCli(_)) => "gcloud_cli".to_string(),
        None => "none".to_string(),
    };
    if let Some(ep) = &node.endpoint {
        format!("{}, endpoint: {}", auth_part, ep)
    } else {
        auth_part
    }
}

/// Holds reusable connection config from an existing node.
struct ReusedConnection {
    auth: Option<Auth>,
    endpoint: Option<String>,
}

/// Find existing nodes matching the given provider kind and optional ID prefix.
fn find_reusable_nodes<'a>(
    config: &'a Config,
    provider: &ProviderKind,
    id_prefix: Option<&str>,
) -> Vec<(&'a String, &'a AiNode)> {
    config
        .nodes
        .iter()
        .filter(|(id, n)| {
            &n.provider == provider && id_prefix.is_none_or(|prefix| id.starts_with(prefix))
        })
        .collect()
}

/// Check if there are existing nodes matching the given filter and offer to reuse
/// their connection config.
fn prompt_reuse_connection(
    config: &Config,
    provider: &ProviderKind,
    id_prefix: Option<&str>,
) -> Result<Option<ReusedConnection>> {
    let existing = find_reusable_nodes(config, provider, id_prefix);

    if existing.is_empty() {
        return Ok(None);
    }

    let mut options: Vec<String> = existing
        .iter()
        .map(|(id, node)| format!("{} ({})", id, auth_summary(node)))
        .collect();
    options.push("New configuration".to_string());

    let Some(idx) = prompt_select(
        "You have existing nodes for this provider. Reuse connection settings?",
        &options,
    )?
    else {
        return Ok(None);
    };

    if idx == existing.len() {
        return Ok(None);
    }

    let (_, node) = existing[idx];
    Ok(Some(ReusedConnection {
        auth: node.auth.clone(),
        endpoint: node.endpoint.clone(),
    }))
}

/// Fetch models from a provider API and let the user select one.
async fn select_model(
    models_future: impl Future<Output = Result<Vec<String>>>,
    default: &str,
) -> Result<String> {
    if is_tui_mode() {
        tui_show_message("Fetching available models...")?;
    }

    let result = tokio::time::timeout(Duration::from_secs(5), models_future).await;

    match result {
        Ok(Ok(mut models)) if !models.is_empty() => {
            models.sort();
            let manual_option = "[ Enter manually ]".to_string();
            models.push(manual_option.clone());

            let Some(idx) = prompt_select("Select model:", &models)? else {
                return Ok(default.to_string());
            };

            if models[idx] == manual_option {
                prompt_text("Model:", default, None)
            } else {
                Ok(models[idx].clone())
            }
        }
        Ok(Ok(_)) => {
            if !is_tui_mode() {
                println!(
                    "{}",
                    "No models found from the API. Enter model name manually.".dimmed()
                );
            }
            prompt_text("Model:", default, Some("No models found from the API"))
        }
        Ok(Err(e)) => {
            if !is_tui_mode() {
                println!(
                    "{} {}",
                    "Could not fetch models:".dimmed(),
                    format!("{:#}", e).dimmed()
                );
            }
            prompt_text(
                "Model:",
                default,
                Some("Could not fetch models from the API"),
            )
        }
        Err(_) => {
            if !is_tui_mode() {
                println!(
                    "{}",
                    "Timed out fetching models. Enter model name manually.".dimmed()
                );
            }
            prompt_text("Model:", default, Some("Timed out fetching models"))
        }
    }
}

/// Prompt user to select capabilities for a node.
fn prompt_capabilities(provider: &ProviderKind) -> Result<Vec<Capability>> {
    let supported = provider.supported_capabilities();

    if supported.len() <= 1 {
        return Ok(supported);
    }

    let labels: Vec<String> = supported.iter().map(|c| c.label().to_string()).collect();
    let selected = prompt_multi_select("What can this model do?", &labels)?;

    let caps = selected
        .iter()
        .filter_map(|s| {
            labels
                .iter()
                .position(|l| l == s)
                .map(|i| supported[i].clone())
        })
        .collect();
    Ok(caps)
}

/// Prompt for provider-specific fields given a pre-selected provider label.
async fn prompt_node_for_kind(
    config: &Config,
    kind_label: &str,
) -> Result<Option<(String, AiNode)>> {
    match kind_label {
        "OpenAI" => {
            let (auth, endpoint_opt) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::OpenAi, Some("openai/"))?
            {
                (reused.auth, reused.endpoint)
            } else {
                let api_key = prompt_text("API key:", "", Some("Or set OPENAI_API_KEY env var"))?;
                let endpoint = prompt_text("Endpoint (leave empty for default):", "", None)?;

                let auth = if api_key.is_empty() {
                    Some(Auth::Env("OPENAI_API_KEY".to_string()))
                } else {
                    Some(Auth::ApiKey(api_key))
                };
                let endpoint_opt = if endpoint.is_empty() {
                    None
                } else {
                    Some(endpoint)
                };
                (auth, endpoint_opt)
            };

            let effective_key = match &auth {
                Some(Auth::ApiKey(k)) => k.clone(),
                Some(Auth::Env(var)) => std::env::var(var).unwrap_or_default(),
                _ => String::new(),
            };

            let model = if !effective_key.is_empty() {
                let client =
                    crate::openai::OpenAiClient::new(&effective_key, "_", endpoint_opt.clone());
                select_model(client.list_models(), "gpt-4o").await?
            } else {
                prompt_text("Model:", "gpt-4o", None)?
            };

            let capabilities = prompt_capabilities(&ProviderKind::OpenAi)?;

            let node = AiNode {
                provider: ProviderKind::OpenAi,
                alias: None,
                capabilities,
                auth,
                model: Some(model.clone()),
                endpoint: endpoint_opt,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("openai/{}", model), node)))
        }
        "Anthropic" => {
            let auth = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::Anthropic, None)?
            {
                reused.auth
            } else {
                let api_key =
                    prompt_text("API key:", "", Some("Or set ANTHROPIC_API_KEY env var"))?;

                if api_key.is_empty() {
                    Some(Auth::Env("ANTHROPIC_API_KEY".to_string()))
                } else {
                    Some(Auth::ApiKey(api_key))
                }
            };

            let effective_key = match &auth {
                Some(Auth::ApiKey(k)) => k.clone(),
                Some(Auth::Env(var)) => std::env::var(var).unwrap_or_default(),
                _ => String::new(),
            };

            let model = if !effective_key.is_empty() {
                let client = crate::anthropic::AnthropicClient::new(&effective_key, "_");
                select_model(client.list_models(), "claude-sonnet-4-6").await?
            } else {
                prompt_text("Model:", "claude-sonnet-4-6", None)?
            };

            let capabilities = prompt_capabilities(&ProviderKind::Anthropic)?;

            let node = AiNode {
                provider: ProviderKind::Anthropic,
                alias: None,
                capabilities,
                auth,
                model: Some(model.clone()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("anthropic/{}", model), node)))
        }
        "Azure OpenAI" => {
            let (auth, endpoint, api_version) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::AzureOpenAi, None)?
            {
                let ep = reused
                    .endpoint
                    .unwrap_or_else(|| "https://my-instance.openai.azure.com".to_string());
                (reused.auth.unwrap_or(Auth::AzureCli(true)), ep, None)
            } else {
                let endpoint = prompt_text(
                    "Endpoint (e.g. https://my-instance.openai.azure.com):",
                    "",
                    None,
                )?;
                let api_version = prompt_text("API version:", "2025-04-01-preview", None)?;

                let auth_options: Vec<String> = vec![
                    "Azure CLI (az login) (Recommended)".into(),
                    "API Key".into(),
                ];
                let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
                    return Ok(None);
                };

                let auth = if auth_idx == 1 {
                    let key = prompt_text("API key:", "", None)?;
                    if key.is_empty() {
                        Auth::AzureCli(true)
                    } else {
                        Auth::ApiKey(key)
                    }
                } else {
                    Auth::AzureCli(true)
                };
                (auth, endpoint, Some(api_version))
            };

            let deployment = prompt_text("Deployment name:", "", None)?;
            let capabilities = prompt_capabilities(&ProviderKind::AzureOpenAi)?;

            let node = AiNode {
                provider: ProviderKind::AzureOpenAi,
                alias: None,
                capabilities,
                auth: Some(auth),
                model: None,
                endpoint: Some(endpoint),
                deployment: Some(deployment.clone()),
                api_version: api_version.or(Some("2025-04-01-preview".to_string())),
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("azure-openai/{}", deployment), node)))
        }
        "Microsoft Foundry" => {
            let (auth, endpoint, api_version) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::MicrosoftFoundry, None)?
            {
                let ep = reused
                    .endpoint
                    .unwrap_or_else(|| "https://my-instance.services.ai.azure.com".to_string());
                (reused.auth.unwrap_or(Auth::AzureCli(true)), ep, None)
            } else {
                let endpoint = prompt_text(
                    "Endpoint (e.g. https://my-instance.services.ai.azure.com):",
                    "",
                    None,
                )?;
                let api_version = prompt_text("API version:", "2024-05-01-preview", None)?;

                let auth_options: Vec<String> = vec![
                    "Azure CLI (az login) (Recommended)".into(),
                    "API Key".into(),
                ];
                let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
                    return Ok(None);
                };

                let auth = if auth_idx == 1 {
                    let key = prompt_text("API key:", "", None)?;
                    if key.is_empty() {
                        Auth::AzureCli(true)
                    } else {
                        Auth::ApiKey(key)
                    }
                } else {
                    Auth::AzureCli(true)
                };
                (auth, endpoint, Some(api_version))
            };

            let model = prompt_text("Model (e.g. gpt-4o):", "", None)?;
            let capabilities = prompt_capabilities(&ProviderKind::MicrosoftFoundry)?;

            let node = AiNode {
                provider: ProviderKind::MicrosoftFoundry,
                alias: None,
                capabilities,
                auth: Some(auth),
                model: Some(model.clone()),
                endpoint: Some(endpoint),
                deployment: None,
                api_version: api_version.or(Some("2024-05-01-preview".to_string())),
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("microsoft-foundry/{}", model), node)))
        }
        "Google Vertex AI" => {
            let existing: Vec<(&String, &AiNode)> = config
                .nodes
                .iter()
                .filter(|(_, n)| n.provider == ProviderKind::VertexAi)
                .collect();

            let (project, location) = if !existing.is_empty() {
                let mut options: Vec<String> = existing
                    .iter()
                    .map(|(id, node)| {
                        format!(
                            "{} (project: {}, location: {})",
                            id,
                            node.project.as_deref().unwrap_or("?"),
                            node.location.as_deref().unwrap_or("?"),
                        )
                    })
                    .collect();
                options.push("New configuration".to_string());

                let selected = prompt_select(
                    "You have existing Vertex AI nodes. Reuse project settings?",
                    &options,
                )?;

                if let Some(idx) = selected {
                    if idx < existing.len() {
                        let (_, node) = existing[idx];
                        (
                            node.project.clone().unwrap_or_default(),
                            node.location
                                .clone()
                                .unwrap_or_else(|| "us-central1".to_string()),
                        )
                    } else {
                        let project = prompt_text("GCP project:", "", None)?;
                        let location = prompt_text("Location:", "us-central1", None)?;
                        (project, location)
                    }
                } else {
                    let project = prompt_text("GCP project:", "", None)?;
                    let location = prompt_text("Location:", "us-central1", None)?;
                    (project, location)
                }
            } else {
                let project = prompt_text("GCP project:", "", None)?;
                let location = prompt_text("Location:", "us-central1", None)?;
                (project, location)
            };

            let model = prompt_text("Model:", "gemini-3.1-pro", None)?;
            let capabilities = prompt_capabilities(&ProviderKind::VertexAi)?;

            let node = AiNode {
                provider: ProviderKind::VertexAi,
                alias: None,
                capabilities,
                auth: Some(Auth::GcloudCli(true)),
                model: Some(model.clone()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: Some(project),
                location: Some(location),
                node_defaults: None,
            };
            Ok(Some((format!("vertex-ai/{}", model), node)))
        }
        "Ollama" => {
            let endpoint_opt = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::Ollama, None)?
            {
                reused.endpoint
            } else {
                let endpoint = prompt_text("Endpoint:", "http://localhost:11434", None)?;

                if endpoint == "http://localhost:11434" {
                    None
                } else {
                    Some(endpoint)
                }
            };

            let client = crate::ollama::OllamaClient::new("_", endpoint_opt.clone());
            let model = select_model(client.list_models(), "llama3.2").await?;
            let capabilities = prompt_capabilities(&ProviderKind::Ollama)?;

            let node = AiNode {
                provider: ProviderKind::Ollama,
                alias: None,
                capabilities,
                auth: None,
                model: Some(model.clone()),
                endpoint: endpoint_opt,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("ollama/{}", model), node)))
        }
        "LM Studio" => {
            let (auth, endpoint_opt) = if let Some(reused) =
                prompt_reuse_connection(config, &ProviderKind::OpenAi, Some("lm-studio/"))?
            {
                (reused.auth, reused.endpoint)
            } else {
                let endpoint = prompt_text("Endpoint:", "http://localhost:1234", None)?;
                let api_key = prompt_text("API key (leave empty if not required):", "", None)?;

                let auth = if api_key.is_empty() {
                    None
                } else {
                    Some(Auth::ApiKey(api_key))
                };
                (auth, Some(endpoint))
            };

            let effective_key = match &auth {
                Some(Auth::ApiKey(k)) => k.clone(),
                Some(Auth::Env(var)) => std::env::var(var).unwrap_or_default(),
                _ => String::new(),
            };

            let client = crate::openai::OpenAiClient::new(
                if effective_key.is_empty() {
                    "lm-studio"
                } else {
                    &effective_key
                },
                "_",
                endpoint_opt.clone(),
            );
            let model = select_model(client.list_models(), "default").await?;
            let capabilities = prompt_capabilities(&ProviderKind::OpenAi)?;

            let node = AiNode {
                provider: ProviderKind::OpenAi,
                alias: None,
                capabilities,
                auth,
                model: Some(model.clone()),
                endpoint: endpoint_opt,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("lm-studio/{}", model), node)))
        }
        _ => {
            // Local Agent
            let binary_options: Vec<String> =
                vec!["claude".into(), "codex".into(), "copilot".into()];
            let Some(idx) = prompt_select("Select agent", &binary_options)? else {
                return Ok(None);
            };
            let binary = &binary_options[idx];

            let capabilities = prompt_capabilities(&ProviderKind::LocalAgent)?;

            let node = AiNode {
                provider: ProviderKind::LocalAgent,
                alias: None,
                capabilities,
                auth: None,
                model: None,
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: Some(binary.to_string()),
                project: None,
                location: None,
                node_defaults: None,
            };
            Ok(Some((format!("local-agent/{}", binary), node)))
        }
    }
}

// ---------------------------------------------------------------------------
// Field editing
// ---------------------------------------------------------------------------

fn edit_node_fields(config: &mut Config, id: &str) -> Result<()> {
    let node = config.get_node_mut(id).unwrap();

    loop {
        let fields = editable_fields(node);
        let mut options: Vec<String> = fields
            .iter()
            .map(|(label, val)| format!("{}: {}", label, val.as_deref().unwrap_or("(not set)")))
            .collect();
        options.push("Done editing".to_string());

        let selected = prompt_select(&format!("Edit: {}", id), &options)?;

        let Some(idx) = selected else {
            break;
        };

        if idx == options.len() - 1 {
            break;
        }

        let field_name = fields[idx].0;
        edit_field(node, field_name)?;
    }

    Ok(())
}

fn editable_fields(node: &AiNode) -> Vec<(&'static str, Option<String>)> {
    let auth_display = match &node.auth {
        Some(Auth::Env(v)) => Some(format!("env: {}", v)),
        Some(Auth::ApiKey(_)) => Some("api_key: ********".to_string()),
        Some(Auth::AzureCli(_)) => Some("azure_cli".to_string()),
        Some(Auth::GcloudCli(_)) => Some("gcloud_cli".to_string()),
        None => None,
    };

    let caps_display = if node.capabilities.is_empty() {
        None
    } else {
        Some(
            node.capabilities
                .iter()
                .map(|c| c.config_key())
                .collect::<Vec<_>>()
                .join(", "),
        )
    };

    let mut fields = match node.provider {
        ProviderKind::OpenAi => vec![
            ("Model", node.model.clone()),
            ("Endpoint", node.endpoint.clone()),
            ("Auth", auth_display),
        ],
        ProviderKind::Anthropic => vec![("Model", node.model.clone()), ("Auth", auth_display)],
        ProviderKind::AzureOpenAi => vec![
            ("Endpoint", node.endpoint.clone()),
            ("Deployment", node.deployment.clone()),
            ("API version", node.api_version.clone()),
            ("Auth", auth_display),
        ],
        ProviderKind::MicrosoftFoundry => vec![
            ("Endpoint", node.endpoint.clone()),
            ("Model", node.model.clone()),
            ("API version", node.api_version.clone()),
            ("Auth", auth_display),
        ],
        ProviderKind::VertexAi => vec![
            ("Project", node.project.clone()),
            ("Location", node.location.clone()),
            ("Model", node.model.clone()),
        ],
        ProviderKind::Ollama => vec![
            ("Model", node.model.clone()),
            ("Endpoint", node.endpoint.clone()),
        ],
        ProviderKind::LocalAgent => vec![("Binary", node.binary.clone())],
    };

    fields.push(("Capabilities", caps_display));
    fields
}

fn edit_field(node: &mut AiNode, field: &str) -> Result<()> {
    if field == "Auth" {
        return edit_auth(node);
    }
    if field == "Capabilities" {
        node.capabilities = prompt_capabilities(&node.provider)?;
        return Ok(());
    }

    let current = match field {
        "Model" => node.model.as_deref().unwrap_or(""),
        "Endpoint" => node.endpoint.as_deref().unwrap_or(""),
        "Deployment" => node.deployment.as_deref().unwrap_or(""),
        "API version" => node.api_version.as_deref().unwrap_or(""),
        "Binary" => node.binary.as_deref().unwrap_or(""),
        "Project" => node.project.as_deref().unwrap_or(""),
        "Location" => node.location.as_deref().unwrap_or(""),
        _ => return Ok(()),
    };

    let label = format!("{}:", field);
    let value = prompt_text(
        &label,
        current,
        Some("Enter new value, or leave empty to clear"),
    )?;
    let opt = if value.is_empty() { None } else { Some(value) };

    match field {
        "Model" => node.model = opt,
        "Endpoint" => node.endpoint = opt,
        "Deployment" => node.deployment = opt,
        "API version" => node.api_version = opt,
        "Binary" => node.binary = opt,
        "Project" => node.project = opt,
        "Location" => node.location = opt,
        _ => {}
    }

    Ok(())
}

fn edit_auth(node: &mut AiNode) -> Result<()> {
    let options: Vec<String> = vec![
        "Environment variable".into(),
        "API key (inline)".into(),
        "Azure CLI".into(),
        "gcloud CLI".into(),
        "None (clear)".into(),
    ];

    let Some(idx) = prompt_select("Authentication method", &options)? else {
        return Ok(());
    };

    node.auth = match idx {
        0 => {
            let var = prompt_text("Environment variable:", "OPENAI_API_KEY", None)?;
            Some(Auth::Env(var))
        }
        1 => {
            let key = prompt_text("API key:", "", None)?;
            if key.is_empty() {
                None
            } else {
                Some(Auth::ApiKey(key))
            }
        }
        2 => Some(Auth::AzureCli(true)),
        3 => Some(Auth::GcloudCli(true)),
        _ => None,
    };

    Ok(())
}

// ---------------------------------------------------------------------------
// Azure / Foundry discovery flows
// ---------------------------------------------------------------------------

async fn azure_discover_flow() -> Result<(String, AiNode)> {
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("No enabled Azure subscriptions found");
    }

    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
    let Some(idx) = prompt_select_with_default("Select Azure subscription", &labels, default_idx)?
    else {
        anyhow::bail!("No subscription selected");
    };
    let sub = &subs[idx];

    azure_discover::set_subscription(&sub.id).await?;

    let resources = azure_discover::list_openai_resources().await?;
    if resources.is_empty() {
        anyhow::bail!(
            "No Azure OpenAI resources found in subscription '{}'",
            sub.name
        );
    }

    let labels: Vec<String> = resources.iter().map(|r| r.to_string()).collect();
    let Some(idx) = prompt_select("Select Azure OpenAI resource", &labels)? else {
        anyhow::bail!("No resource selected");
    };
    let resource = &resources[idx];
    let endpoint = resource
        .endpoint()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no endpoint", resource.name))?
        .trim_end_matches('/')
        .to_string();

    let rg = resource
        .resource_group
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no resource group", resource.name))?;

    let deployments = azure_discover::list_deployments(rg, &resource.name).await?;
    if deployments.is_empty() {
        anyhow::bail!("No deployments found on resource '{}'", resource.name);
    }

    let labels: Vec<String> = deployments.iter().map(|d| d.to_string()).collect();
    let Some(idx) = prompt_select("Select deployment", &labels)? else {
        anyhow::bail!("No deployment selected");
    };
    let deployment = &deployments[idx];

    let api_version = prompt_text("API version:", "2025-04-01-preview", None)?;

    let auth_options: Vec<String> = vec![
        "Azure CLI (az login) (Recommended)".into(),
        "API Key".into(),
    ];
    let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
        anyhow::bail!("No auth method selected");
    };

    let auth = if auth_idx == 1 {
        let key = prompt_text("API key:", "", None)?;
        if key.is_empty() {
            Auth::AzureCli(true)
        } else {
            Auth::ApiKey(key)
        }
    } else {
        Auth::AzureCli(true)
    };

    let node = AiNode {
        provider: ProviderKind::AzureOpenAi,
        alias: None,
        capabilities: vec![Capability::Chat, Capability::Image],
        auth: Some(auth),
        model: None,
        endpoint: Some(endpoint),
        deployment: Some(deployment.name.clone()),
        api_version: Some(api_version),
        binary: None,
        project: None,
        location: None,
        node_defaults: None,
    };
    Ok((format!("azure-openai/{}", deployment.name), node))
}

async fn foundry_discover_flow() -> Result<(String, AiNode)> {
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("No enabled Azure subscriptions found");
    }

    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let default_idx = subs.iter().position(|s| s.is_default).unwrap_or(0);
    let Some(idx) = prompt_select_with_default("Select Azure subscription", &labels, default_idx)?
    else {
        anyhow::bail!("No subscription selected");
    };
    let sub = &subs[idx];

    azure_discover::set_subscription(&sub.id).await?;

    let resources = azure_discover::list_foundry_resources().await?;
    if resources.is_empty() {
        anyhow::bail!(
            "No Microsoft Foundry resources found in subscription '{}'",
            sub.name
        );
    }

    let labels: Vec<String> = resources.iter().map(|r| r.to_string()).collect();
    let Some(idx) = prompt_select("Select Microsoft Foundry resource", &labels)? else {
        anyhow::bail!("No resource selected");
    };
    let resource = &resources[idx];
    let raw_endpoint = resource
        .endpoint()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no endpoint", resource.name))?
        .trim_end_matches('/')
        .to_string();

    let endpoint = raw_endpoint.replace(".cognitiveservices.azure.com", ".services.ai.azure.com");

    let rg = resource
        .resource_group
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Resource '{}' has no resource group", resource.name))?;

    let deployments = azure_discover::list_deployments(rg, &resource.name).await?;
    if deployments.is_empty() {
        anyhow::bail!("No deployments found on resource '{}'", resource.name);
    }

    let labels: Vec<String> = deployments.iter().map(|d| d.to_string()).collect();
    let Some(idx) = prompt_select("Select deployment", &labels)? else {
        anyhow::bail!("No deployment selected");
    };
    let deployment = &deployments[idx];
    let model = deployment
        .properties
        .as_ref()
        .and_then(|p| p.model.as_ref())
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| deployment.name.clone());

    let api_version = prompt_text("API version:", "2024-05-01-preview", None)?;

    let auth_options: Vec<String> = vec![
        "Azure CLI (az login) (Recommended)".into(),
        "API Key".into(),
    ];
    let Some(auth_idx) = prompt_select("Authentication method", &auth_options)? else {
        anyhow::bail!("No auth method selected");
    };

    let auth = if auth_idx == 1 {
        let key = prompt_text("API key:", "", None)?;
        if key.is_empty() {
            Auth::AzureCli(true)
        } else {
            Auth::ApiKey(key)
        }
    } else {
        Auth::AzureCli(true)
    };

    let node = AiNode {
        provider: ProviderKind::MicrosoftFoundry,
        alias: None,
        capabilities: vec![Capability::Chat],
        auth: Some(auth),
        model: Some(model.clone()),
        endpoint: Some(endpoint),
        deployment: None,
        api_version: Some(api_version),
        binary: None,
        project: None,
        location: None,
        node_defaults: None,
    };
    Ok((format!("microsoft-foundry/{}", model), node))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(provider: ProviderKind, auth: Option<Auth>, endpoint: Option<&str>) -> AiNode {
        AiNode {
            provider,
            alias: None,
            capabilities: vec![],
            auth,
            model: Some("test-model".to_string()),
            endpoint: endpoint.map(|s| s.to_string()),
            deployment: None,
            api_version: None,
            binary: None,
            project: None,
            location: None,
            node_defaults: None,
        }
    }

    fn config_with_nodes(nodes: Vec<(&str, AiNode)>) -> Config {
        let mut config = Config::default();
        for (id, node) in nodes {
            config.add_node(id.to_string(), node);
        }
        config
    }

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
    fn test_find_reusable_nodes_empty_config() {
        let config = Config::default();
        let nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert!(nodes.is_empty());
    }

    #[test]
    fn test_find_reusable_nodes_matches_provider() {
        let config = config_with_nodes(vec![
            (
                "openai/gpt-4o",
                make_node(
                    ProviderKind::OpenAi,
                    Some(Auth::ApiKey("sk-123".into())),
                    None,
                ),
            ),
            (
                "anthropic/claude",
                make_node(
                    ProviderKind::Anthropic,
                    Some(Auth::ApiKey("ak-456".into())),
                    None,
                ),
            ),
        ]);

        let nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].0, "openai/gpt-4o");
    }

    #[test]
    fn test_find_reusable_nodes_openai_prefix_excludes_lm_studio() {
        let config = config_with_nodes(vec![
            (
                "openai/gpt-4o",
                make_node(
                    ProviderKind::OpenAi,
                    Some(Auth::ApiKey("sk-123".into())),
                    None,
                ),
            ),
            (
                "lm-studio/llama",
                make_node(ProviderKind::OpenAi, None, Some("http://localhost:1234")),
            ),
        ]);

        let openai_nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, Some("openai/"));
        assert_eq!(openai_nodes.len(), 1);
        assert_eq!(openai_nodes[0].0, "openai/gpt-4o");

        let lm_nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, Some("lm-studio/"));
        assert_eq!(lm_nodes.len(), 1);
        assert_eq!(lm_nodes[0].0, "lm-studio/llama");
    }

    #[test]
    fn test_find_reusable_nodes_no_prefix_finds_all() {
        let config = config_with_nodes(vec![
            (
                "openai/gpt-4o",
                make_node(
                    ProviderKind::OpenAi,
                    Some(Auth::ApiKey("sk-123".into())),
                    None,
                ),
            ),
            (
                "lm-studio/llama",
                make_node(ProviderKind::OpenAi, None, Some("http://localhost:1234")),
            ),
        ]);

        let all_nodes = find_reusable_nodes(&config, &ProviderKind::OpenAi, None);
        assert_eq!(all_nodes.len(), 2);
    }

    #[test]
    fn test_auth_summary_api_key() {
        let node = make_node(
            ProviderKind::OpenAi,
            Some(Auth::ApiKey("sk-123".into())),
            None,
        );
        assert_eq!(auth_summary(&node), "api_key: ********");
    }

    #[test]
    fn test_auth_summary_env() {
        let node = make_node(
            ProviderKind::OpenAi,
            Some(Auth::Env("OPENAI_API_KEY".into())),
            None,
        );
        assert_eq!(auth_summary(&node), "env: OPENAI_API_KEY");
    }

    #[test]
    fn test_auth_summary_with_endpoint() {
        let node = make_node(
            ProviderKind::OpenAi,
            Some(Auth::ApiKey("sk-123".into())),
            Some("http://localhost:1234"),
        );
        assert_eq!(
            auth_summary(&node),
            "api_key: ********, endpoint: http://localhost:1234"
        );
    }

    #[test]
    fn test_auth_summary_none() {
        let node = make_node(ProviderKind::Ollama, None, None);
        assert_eq!(auth_summary(&node), "none");
    }
}
