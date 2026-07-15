//! Full-screen ratatui config dashboard (gated behind `config-tui`).
//!
//! [`run`] owns terminal setup/teardown and the event loop; all UI state lives
//! in [`app::App`] and all key handling in its pure reducer, which returns
//! [`Effect`]s that this module executes (saving, keychain writes, connectivity
//! tests, and Azure/Foundry discovery — the async and I/O side effects the
//! reducer must stay free of).

pub(crate) mod actions;
pub(crate) mod app;
pub(crate) mod forms;
pub(crate) mod ui;

use std::io::{Stdout, stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::azure_discover;
use crate::client::Client;
use crate::config::{AiNode, Auth, Config, ProviderKind, consent_keys};
use crate::types::Message;
use app::{App, Effect, Mode};
use forms::NodeForm;

/// The concrete terminal type used throughout this module.
type Term = Terminal<CrosstermBackend<Stdout>>;

/// Poll interval; also bounds how often the UI re-renders when idle.
const TICK: Duration = Duration::from_millis(100);

/// How long a connectivity test waits for a response before giving up.
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// RAII guard: enters raw mode + the alternate screen on construction and
/// restores the terminal on drop, so a panic in the loop still leaves the
/// terminal usable.
struct TerminalGuard {
    terminal: Term,
    _restore: RestoreGuard,
}

/// Inner restore guard, constructed the moment raw mode is enabled so that a
/// failure in any later setup step (alternate screen, backend init) still
/// restores the terminal on unwind. Tracks whether the alternate screen was
/// actually entered.
struct RestoreGuard {
    alt_screen: bool,
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        if self.alt_screen {
            let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        }
        let _ = disable_raw_mode();
    }
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        // From here on, `restore` guarantees raw mode is disabled on any
        // early return or panic.
        let mut restore = RestoreGuard { alt_screen: false };
        let mut out = stdout();
        execute!(out, EnterAlternateScreen).context("failed to enter alternate screen")?;
        restore.alt_screen = true;
        let terminal = Terminal::new(CrosstermBackend::new(out))
            .context("failed to initialize the terminal backend")?;
        Ok(TerminalGuard {
            terminal,
            _restore: restore,
        })
    }
}

/// Launch the interactive config dashboard for `app_name`, editing the global
/// config. Returns once the user quits; config changes are persisted via the
/// [`Effect::Save`] effect.
pub(crate) async fn run(app_name: &str) -> Result<()> {
    let config = Config::load_global()?;
    let mut app = App::new(config, app_name);

    let mut guard = TerminalGuard::enter()?;

    loop {
        guard.terminal.draw(|frame| ui::draw(frame, &app))?;

        if !event::poll(TICK)? {
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if let Some(effect) = app.handle_key(key) {
                if handle_effect(&mut app, effect, &mut guard.terminal).await? {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Run a single add- or edit-node form to completion on the ratatui stack,
/// returning the saved node id (if any). Used by the library's
/// [`crate::config_tui::add_node_interactive`]/`edit_node_interactive` API.
///
/// `edit_id = None` starts a fresh add-node form; `Some(id)` edits that node.
pub(crate) async fn run_single_form(
    app_name: &str,
    config: &mut Config,
    edit_id: Option<String>,
) -> Result<Option<String>> {
    let mut app = App::new(std::mem::take(config), app_name);

    match &edit_id {
        Some(id) => match app.config.nodes.get(id) {
            Some(node) => {
                let form = NodeForm::from_node(id, node);
                app.mode = Mode::EditNode {
                    id: id.clone(),
                    form,
                };
            }
            None => {
                *config = std::mem::take(&mut app.config);
                anyhow::bail!("node '{id}' not found");
            }
        },
        None => app.mode = Mode::AddNode(NodeForm::new()),
    }

    {
        let mut guard = TerminalGuard::enter()?;
        loop {
            guard.terminal.draw(|frame| ui::draw(frame, &app))?;

            if !event::poll(TICK)? {
                continue;
            }

            if let Event::Key(key) = event::read()? {
                if let Some(effect) = app.handle_key(key) {
                    if handle_effect(&mut app, effect, &mut guard.terminal).await? {
                        break;
                    }
                }
            }

            // A single-form session ends the moment we fall back to Browse
            // (after a save or a cancel).
            if matches!(app.mode, Mode::Browse) {
                break;
            }
        }
    }

    let saved = app.last_saved_node.clone();
    *config = app.config;
    Ok(saved)
}

/// Execute an [`Effect`]. Returns `Ok(true)` when the app should quit.
async fn handle_effect(app: &mut App, effect: Effect, terminal: &mut Term) -> Result<bool> {
    match effect {
        Effect::Quit => return Ok(true),
        Effect::Save => {
            app.config.save()?;
            app.dirty = false;
            app.status_line = Some("saved".to_string());
        }
        Effect::RunTest(node_id) => run_test(app, &node_id, terminal).await?,
        Effect::StoreKeychain { node_id, secret } => store_keychain(app, &node_id, &secret)?,
        Effect::Discover(provider) => run_discovery(app, provider, terminal).await?,
    }
    Ok(false)
}

/// Ping a node with a one-line chat, showing the outcome in a popup.
async fn run_test(app: &mut App, node_id: &str, terminal: &mut Term) -> Result<()> {
    // Draw a "testing…" status once before we block on the request.
    app.status_line = Some(format!("testing {node_id}…"));
    terminal.draw(|frame| ui::draw(frame, app))?;

    let node = app.config.nodes.get(node_id).cloned();
    let result: std::result::Result<String, String> = match node {
        None => Err(format!("node '{node_id}' not found")),
        Some(node) => match Client::from_node(&node) {
            Err(e) => Err(format!("{e:#}")),
            Ok(client) => {
                let messages = [Message::user("Say hello in one sentence.")];
                let fut = client.chat(&messages);
                match tokio::time::timeout(TEST_TIMEOUT, fut).await {
                    Ok(Ok(resp)) => Ok(resp.content),
                    Ok(Err(e)) => Err(format!("{e:#}")),
                    Err(_) => Err(format!("timed out after {}s", TEST_TIMEOUT.as_secs())),
                }
            }
        },
    };

    let text = match result {
        Ok(reply) => format!("OK: {reply}"),
        Err(e) => format!("ERROR: {e}"),
    };
    app.status_line = None;
    app.mode = Mode::Test {
        node_id: node_id.to_string(),
        result: Some(text),
    };
    Ok(())
}

/// Store a secret in the OS keychain and switch the node to keychain auth.
fn store_keychain(app: &mut App, node_id: &str, secret: &str) -> Result<()> {
    #[cfg(feature = "keychain")]
    {
        crate::config::set_keychain_secret(node_id, secret)?;
        if let Some(node) = app.config.nodes.get_mut(node_id) {
            node.auth = Some(Auth::Keychain(true));
        }
        app.config.save()?;
        app.dirty = false;
        app.status_line = Some(format!("stored key for {node_id} in the OS keychain"));
    }
    #[cfg(not(feature = "keychain"))]
    {
        let _ = (node_id, secret);
        app.status_line =
            Some("keychain feature not enabled in this build — cannot store the key".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Azure / Foundry discovery (inline, blocking the UI briefly like RunTest)
// ---------------------------------------------------------------------------

/// Run az-CLI discovery for the given provider, prefilling the add-node form
/// with the chosen deployment. Consent is gated by a modal on first use.
async fn run_discovery(app: &mut App, provider: ProviderKind, terminal: &mut Term) -> Result<()> {
    // Consent gate (persisted in the global config's consents map).
    let key = consent_keys::AZURE_CLI;
    let allowed = match crate::config_tui::check_consent(&app.config, key) {
        Some(allowed) => allowed,
        None => {
            let ok = confirm_modal(
                terminal,
                "Allow ailloy to run the Azure CLI (az) to discover your \
                 subscriptions, resources, and deployments?",
            )?;
            if ok {
                app.config.consents.insert(key.to_string(), true);
                app.config.save()?;
            }
            ok
        }
    };
    if !allowed {
        app.status_line = Some("azure-cli discovery declined".to_string());
        return Ok(());
    }

    let outcome = match provider {
        ProviderKind::AzureOpenAi => discover_azure(terminal).await,
        ProviderKind::MicrosoftFoundry => discover_foundry(terminal).await,
        _ => return Ok(()),
    };

    match outcome {
        Ok(Some((id, node))) => {
            let form = NodeForm::from_node(&id, &node);
            app.mode = Mode::AddNode(form);
            app.status_line = Some(format!("discovered {id} — review and Save"));
        }
        Ok(None) => app.status_line = Some("discovery cancelled".to_string()),
        Err(e) => app.status_line = Some(format!("discovery failed: {e:#}")),
    }
    Ok(())
}

/// Azure OpenAI discovery: subscription → resource → deployment pick lists.
async fn discover_azure(terminal: &mut Term) -> Result<Option<(String, AiNode)>> {
    status_popup(terminal, "Listing Azure subscriptions…")?;
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("no enabled Azure subscriptions found — run 'az login' first");
    }
    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let Some(i) = pick_list(terminal, "Select Azure subscription", &labels)? else {
        return Ok(None);
    };
    azure_discover::set_subscription(&subs[i].id).await?;

    status_popup(terminal, "Listing Azure OpenAI resources…")?;
    let resources = azure_discover::list_openai_resources().await?;
    if resources.is_empty() {
        anyhow::bail!(
            "no Azure OpenAI resources found in subscription '{}'",
            subs[i].name
        );
    }
    let labels: Vec<String> = resources.iter().map(|r| r.to_string()).collect();
    let Some(j) = pick_list(terminal, "Select Azure OpenAI resource", &labels)? else {
        return Ok(None);
    };
    let resource = &resources[j];
    let endpoint = resource
        .endpoint()
        .ok_or_else(|| anyhow::anyhow!("resource '{}' has no endpoint", resource.name))?
        .trim_end_matches('/')
        .to_string();
    let rg = resource
        .resource_group
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("resource '{}' has no resource group", resource.name))?;

    status_popup(terminal, "Listing deployments…")?;
    let deployments = azure_discover::list_deployments(rg, &resource.name).await?;
    if deployments.is_empty() {
        anyhow::bail!("no deployments found on resource '{}'", resource.name);
    }
    let labels: Vec<String> = deployments.iter().map(|d| d.to_string()).collect();
    let Some(k) = pick_list(terminal, "Select deployment", &labels)? else {
        return Ok(None);
    };
    let deployment = &deployments[k];
    let model_name = deployment
        .properties
        .as_ref()
        .and_then(|p| p.model.as_ref())
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| deployment.name.clone());

    let mut node = AiNode::new(ProviderKind::AzureOpenAi);
    node.endpoint = Some(endpoint);
    node.deployment = Some(deployment.name.clone());
    node.auth = Some(Auth::AzureCli(true));
    node.capabilities = azure_discover::capabilities_for_deployment(&model_name);
    Ok(Some((format!("azure-openai/{}", deployment.name), node)))
}

/// Microsoft Foundry discovery: subscription → resource → deployment.
async fn discover_foundry(terminal: &mut Term) -> Result<Option<(String, AiNode)>> {
    status_popup(terminal, "Listing Azure subscriptions…")?;
    let subs = azure_discover::list_subscriptions().await?;
    if subs.is_empty() {
        anyhow::bail!("no enabled Azure subscriptions found — run 'az login' first");
    }
    let labels: Vec<String> = subs.iter().map(|s| s.to_string()).collect();
    let Some(i) = pick_list(terminal, "Select Azure subscription", &labels)? else {
        return Ok(None);
    };
    azure_discover::set_subscription(&subs[i].id).await?;

    status_popup(terminal, "Listing Microsoft Foundry resources…")?;
    let resources = azure_discover::list_foundry_resources().await?;
    if resources.is_empty() {
        anyhow::bail!(
            "no Microsoft Foundry resources found in subscription '{}'",
            subs[i].name
        );
    }
    let labels: Vec<String> = resources.iter().map(|r| r.to_string()).collect();
    let Some(j) = pick_list(terminal, "Select Microsoft Foundry resource", &labels)? else {
        return Ok(None);
    };
    let resource = &resources[j];
    let raw_endpoint = resource
        .endpoint()
        .ok_or_else(|| anyhow::anyhow!("resource '{}' has no endpoint", resource.name))?
        .trim_end_matches('/')
        .to_string();
    let endpoint = raw_endpoint.replace(".cognitiveservices.azure.com", ".services.ai.azure.com");
    let rg = resource
        .resource_group
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("resource '{}' has no resource group", resource.name))?;

    status_popup(terminal, "Listing deployments…")?;
    let deployments = azure_discover::list_deployments(rg, &resource.name).await?;
    if deployments.is_empty() {
        anyhow::bail!("no deployments found on resource '{}'", resource.name);
    }
    let labels: Vec<String> = deployments.iter().map(|d| d.to_string()).collect();
    let Some(k) = pick_list(terminal, "Select deployment", &labels)? else {
        return Ok(None);
    };
    let deployment = &deployments[k];
    let model = deployment
        .properties
        .as_ref()
        .and_then(|p| p.model.as_ref())
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| deployment.name.clone());

    let mut node = AiNode::new(ProviderKind::MicrosoftFoundry);
    node.endpoint = Some(endpoint);
    node.model = Some(model.clone());
    node.auth = Some(Auth::AzureCli(true));
    node.capabilities = azure_discover::capabilities_for_deployment(&model);
    Ok(Some((format!("microsoft-foundry/{model}"), node)))
}

/// Draw a transient status popup (does not wait for input).
fn status_popup(terminal: &mut Term, message: &str) -> Result<()> {
    terminal.draw(|frame| ui::draw_message_popup(frame, "Discovering", message))?;
    Ok(())
}

/// Draw a yes/no modal and block until the user answers.
fn confirm_modal(terminal: &mut Term, message: &str) -> Result<bool> {
    loop {
        terminal.draw(|frame| ui::draw_message_popup(frame, "Confirm (y/n)", message))?;
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => return Ok(true),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return Ok(false),
                _ => {}
            }
        }
    }
}

/// Draw a pick list and block until the user selects (Some) or cancels (None).
fn pick_list(terminal: &mut Term, title: &str, items: &[String]) -> Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }
    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| ui::draw_picker_popup(frame, title, items, selected))?;
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < items.len() {
                        selected += 1;
                    }
                }
                KeyCode::Enter => return Ok(Some(selected)),
                KeyCode::Esc => return Ok(None),
                _ => {}
            }
        }
    }
}
