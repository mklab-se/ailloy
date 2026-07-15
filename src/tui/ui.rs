//! Rendering for the config TUI.
//!
//! This is a placeholder in this task — Task 5.2 replaces [`draw`] with the
//! real node table, detail pane, and modal overlays. Keeping the signature
//! stable now lets the event loop compile against it.

use ratatui::Frame;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::app::App;

/// Draw a single frame of the TUI.
pub fn draw(frame: &mut Frame, app: &App) {
    let node_count = app.config.nodes.len();
    let dirty = if app.dirty { " (unsaved)" } else { "" };
    let status = app.status_line.as_deref().unwrap_or("press q to quit");
    let text = format!("ailloy config — {node_count} node(s){dirty}\n\n{status}",);
    let widget = Paragraph::new(text).block(
        Block::default()
            .title(format!(" {} ", app.app_name))
            .borders(Borders::ALL),
    );
    frame.render_widget(widget, frame.area());
}
