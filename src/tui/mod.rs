//! Full-screen ratatui config dashboard (gated behind `config-tui`).
//!
//! [`run`] owns terminal setup/teardown and the event loop; all state lives in
//! [`app::App`] and all decisions in its pure reducer, so this module stays a
//! thin shell around draw + poll + dispatch-effect.

pub(crate) mod actions;
pub(crate) mod app;
pub(crate) mod forms;
pub(crate) mod ui;

use std::io::{Stdout, stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::Config;
use app::{App, Effect};

/// Poll interval; also bounds how often the UI re-renders when idle.
const TICK: Duration = Duration::from_millis(100);

/// RAII guard: enters raw mode + the alternate screen on construction and
/// restores the terminal on drop, so a panic in the loop still leaves the
/// terminal usable.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let terminal = Terminal::new(CrosstermBackend::new(out))
            .context("failed to initialize the terminal backend")?;
        Ok(TerminalGuard { terminal })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

/// Launch the interactive config TUI for `app_name`, editing the global config.
///
/// Loads the config, runs the event loop, and returns once the user quits.
/// Config changes are persisted via the [`Effect::Save`] effect.
pub(crate) async fn run(app_name: &str) -> Result<()> {
    let config = Config::load_global()?;
    let mut app = App::new(config, app_name);

    let mut guard = TerminalGuard::enter()?;

    loop {
        guard.terminal.draw(|frame| ui::draw(frame, &app))?;

        // Poll for input with a tick timeout so the UI stays responsive to
        // resizes and future async status updates.
        if !event::poll(TICK)? {
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if let Some(effect) = app.handle_key(key) {
                match effect {
                    Effect::Quit => break,
                    Effect::Save => {
                        app.config.save()?;
                        app.dirty = false;
                        app.status_line = Some("saved".to_string());
                    }
                    // Connectivity testing is wired up in Task 5.4.
                    Effect::RunTest(_node_id) => {
                        app.status_line = Some("test not yet wired".to_string());
                    }
                }
            }
        }
    }

    Ok(())
}
