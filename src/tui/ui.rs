//! Rendering for the config TUI.
//!
//! [`draw`] renders the read-only Browse view: a header bar, a two-pane split
//! (a node table on the left, a detail pane for the selected node on the
//! right), an optional status line, and a footer key legend. It reads only
//! from [`App`] — all mutation happens in the reducer.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};

use crate::config::{AiNode, Auth, Capability, Config};
use crate::params::{ParamDef, ParamKind, params_for};
use crate::retirement::retirement_warning;
use crate::tui::app::{App, Focus, Mode};
use crate::tui::forms::Editor;

/// The four capability columns shown in the node table, in display order.
const CAPABILITY_COLUMNS: &[Capability] = &[
    Capability::Chat,
    Capability::Image,
    Capability::Embedding,
    Capability::Video,
];

/// Marker glyphs for a capability cell.
const STAR: &str = "★";
const CHECK: &str = "✓";
const DOT: &str = "·";

/// Renders the marker for a single capability cell in the node table.
///
/// * `★` — the node has the capability **and** is the configured default for it
///   (`config.defaults[cap.config_key()] == node_id`).
/// * `✓` — the node has the capability but is not the default.
/// * `·` — the node does not have the capability.
pub(crate) fn capability_cell(
    config: &Config,
    node_id: &str,
    node: &AiNode,
    cap: &Capability,
) -> &'static str {
    if !node.has_capability(cap) {
        return DOT;
    }
    let is_default = config
        .defaults
        .get(cap.config_key())
        .is_some_and(|id| id == node_id);
    if is_default { STAR } else { CHECK }
}

/// A short label for a node's auth strategy, e.g. `env: OPENAI_API_KEY`.
fn auth_label(auth: &Option<Auth>) -> String {
    match auth {
        Some(Auth::Env(var)) => format!("env: {var}"),
        Some(Auth::ApiKey(_)) => "api-key (inline)".to_string(),
        Some(Auth::Keychain(_)) => "keychain".to_string(),
        Some(Auth::AzureCli(_)) => "azure-cli".to_string(),
        Some(Auth::GcloudCli(_)) => "gcloud-cli".to_string(),
        None => "none".to_string(),
    }
}

/// Draw a single frame of the TUI (Browse view).
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Vertical layout: header, main, [status], footer.
    let mut constraints = vec![Constraint::Length(1), Constraint::Min(0)];
    let has_status = app.status_line.is_some();
    if has_status {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let header_area = rows[0];
    let main_area = rows[1];
    let (status_area, footer_area) = if has_status {
        (Some(rows[2]), rows[3])
    } else {
        (None, rows[2])
    };

    draw_header(frame, app, header_area);
    draw_main(frame, app, main_area);
    if let Some(status_area) = status_area {
        draw_status(frame, app, status_area);
    }
    draw_footer(frame, footer_area);

    // Overlay the default editor popup on top of the Browse view.
    if let Mode::EditDefault { editor, .. } = &app.mode {
        draw_edit_popup(frame, app, editor, area);
    }
}

/// A rectangle centered in `area` with the given fixed width/height (clamped to
/// `area`).
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// A dimmed hint describing the accepted values for a parameter kind.
fn kind_hint(kind: &ParamKind) -> String {
    match kind {
        ParamKind::Enum(values) => format!("one of: {}", values.join(", ")),
        ParamKind::UInt { min, max } => format!("{min}..={max}"),
        ParamKind::Float { min, max } => format!("{min}..={max}"),
        ParamKind::Size => "WxH e.g. 1280x720".to_string(),
    }
}

/// Render the centered popup used to edit a per-node default parameter.
///
/// Text editors show an input line with a visible cursor plus an error (red) or
/// hint (dimmed) line; select editors show the list of choices with the current
/// one highlighted. Both share a footer of key hints.
fn draw_edit_popup(frame: &mut Frame, app: &App, editor: &Editor, area: Rect) {
    // The parameter being edited (for the title and the value hint).
    let def: Option<&ParamDef> = editing_param_def(app);
    let title = def.map(|d| d.key).unwrap_or("edit default");

    let popup = centered_rect(60, 9, area);
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    // Split inner into: body (fills), footer (1 line).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let body_area = chunks[0];
    let footer_area = chunks[1];

    match &editor.choices {
        // --- Select editor: list of choices, current highlighted -----------
        Some((choices, selected)) => {
            let lines: Vec<Line> = choices
                .iter()
                .enumerate()
                .map(|(idx, choice)| {
                    if idx == *selected {
                        Line::styled(
                            format!("▸ {choice}"),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Line::from(format!("  {choice}"))
                    }
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), body_area);
        }
        // --- Text editor: input line + error/hint --------------------------
        None => {
            let value_line = Line::from(vec![
                Span::styled("› ", Style::default().fg(Color::DarkGray)),
                Span::raw(editor.value.clone()),
            ]);
            let second = match &editor.error {
                Some(err) => Line::styled(err.clone(), Style::default().fg(Color::Red)),
                None => {
                    let hint = def
                        .map(|d| kind_hint(&d.kind))
                        .unwrap_or_else(|| "enter a value".to_string());
                    Line::styled(hint, Style::default().fg(Color::DarkGray))
                }
            };
            frame.render_widget(
                Paragraph::new(vec![value_line, Line::from(""), second]),
                body_area,
            );

            // Place the terminal cursor after the "› " prefix + cursor offset.
            let prefix = 2u16;
            let cursor_col =
                body_area.x + prefix + editor.value[..editor.cursor].chars().count() as u16;
            frame.set_cursor_position((cursor_col, body_area.y));
        }
    }

    let footer = Paragraph::new(Line::styled(
        "Enter save · Esc cancel",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(footer, footer_area);
}

/// The [`ParamDef`] currently being edited, resolved from the selected node and
/// the mode's `param_idx`. Mirrors the reducer's own lookup.
fn editing_param_def(app: &App) -> Option<&'static ParamDef> {
    let param_idx = match &app.mode {
        Mode::EditDefault { param_idx, .. } => *param_idx,
        _ => return None,
    };
    let node_id = app.selected_node_id()?;
    let node = app.config.nodes.get(&node_id)?;
    params_for(&node.provider, &node.capabilities)
        .get(param_idx)
        .copied()
}

/// Header bar: tool name, config path, and an AI enabled/disabled indicator.
fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let path = Config::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| app.app_name.clone());

    let mut spans = vec![
        Span::styled(
            format!("{} ai config", app.app_name),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" — {path}"), Style::default().fg(Color::DarkGray)),
    ];

    if crate::config_tui::is_ai_active(&app.app_name) {
        spans.push(Span::styled(
            "  [AI enabled]",
            Style::default().fg(Color::Green),
        ));
    } else {
        spans.push(Span::styled(
            "  [AI disabled]",
            Style::default().fg(Color::Yellow),
        ));
    }

    if app.dirty {
        spans.push(Span::styled(
            "  (unsaved)",
            Style::default().fg(Color::Yellow),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Main horizontal split: node table (left) and detail pane (right).
fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    draw_node_table(frame, app, cols[0]);
    draw_detail(frame, app, cols[1]);
}

/// Returns a block with a border styled to reflect focus (thick, accented
/// border when focused; plain rounded border otherwise).
fn pane_block(title: &str, focused: bool) -> Block<'static> {
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL);
    if focused {
        block
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(Color::Cyan))
    } else {
        block.border_type(BorderType::Rounded)
    }
}

/// The left pane: a table of nodes with per-capability cells.
fn draw_node_table(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::NodeList;
    let block = pane_block("Nodes", focused);

    let node_ids = app.node_ids();
    let header = Row::new(vec![
        Cell::from("ID"),
        Cell::from("Provider"),
        Cell::from("C"),
        Cell::from("I"),
        Cell::from("E"),
        Cell::from("V"),
    ])
    .style(
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = node_ids
        .iter()
        .enumerate()
        .map(|(idx, id)| {
            let node = &app.config.nodes[id];
            let label = match &node.alias {
                Some(alias) => format!("{id} ({alias})"),
                None => id.clone(),
            };
            let mut cells = vec![Cell::from(label), Cell::from(node.provider.to_string())];
            for cap in CAPABILITY_COLUMNS {
                cells.push(Cell::from(capability_cell(&app.config, id, node, cap)));
            }
            let row = Row::new(cells);
            if idx == app.selected {
                let mut sel = Style::default().add_modifier(Modifier::REVERSED);
                if focused {
                    sel = sel.fg(Color::Cyan);
                }
                row.style(sel)
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Min(10),
        Constraint::Length(16),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ];

    if node_ids.is_empty() {
        let empty = Paragraph::new("No nodes configured. Press 'a' to add one.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(block);
    frame.render_widget(table, area);
}

/// The right pane: connection, capabilities, and defaults for the selected
/// node, plus any retirement warning.
fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Detail;
    let block = pane_block("Detail", focused);

    let node_id = app.selected_node_id();
    let node = node_id.as_ref().and_then(|id| app.config.nodes.get(id));

    let (node_id, node) = match (node_id, node) {
        (Some(id), Some(node)) => (id, node),
        _ => {
            let empty = Paragraph::new("Select a node to see its details.")
                .style(Style::default().fg(Color::DarkGray))
                .block(block);
            frame.render_widget(empty, area);
            return;
        }
    };

    let dim = Style::default().fg(Color::DarkGray);
    let heading = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();

    // --- Connection ------------------------------------------------------
    lines.push(Line::styled("Connection", heading));
    lines.push(kv("provider", &node.provider.to_string()));
    lines.push(kv("model", node.detail()));
    if let Some(endpoint) = &node.endpoint {
        lines.push(kv("endpoint", endpoint));
    }
    // api_version: None means the unified v1 surface for Azure/Foundry.
    match &node.api_version {
        Some(v) => lines.push(kv("api_version", v)),
        None => {
            if matches!(
                node.provider,
                crate::config::ProviderKind::AzureOpenAi
                    | crate::config::ProviderKind::MicrosoftFoundry
            ) {
                lines.push(kv("api_version", "v1 (unified)"));
            }
        }
    }
    if let Some(project) = &node.project {
        lines.push(kv("project", project));
    }
    if let Some(location) = &node.location {
        lines.push(kv("location", location));
    }
    lines.push(kv("auth", &auth_label(&node.auth)));
    lines.push(Line::from(""));

    // --- Capabilities ----------------------------------------------------
    lines.push(Line::styled("Capabilities", heading));
    for cap in CAPABILITY_COLUMNS {
        let marker = capability_cell(&app.config, &node_id, node, cap);
        let style = match marker {
            STAR => Style::default().fg(Color::Yellow),
            CHECK => Style::default().fg(Color::Green),
            _ => dim,
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {marker} "), style),
            Span::styled(cap.label().to_string(), style),
        ]));
    }
    lines.push(Line::from(""));

    // --- Defaults --------------------------------------------------------
    lines.push(Line::styled("Defaults", heading));
    let params = params_for(&node.provider, &node.capabilities);
    if params.is_empty() {
        lines.push(Line::styled("  (no tunable parameters)", dim));
    }
    for (idx, def) in params.iter().enumerate() {
        let selected = focused && idx == app.detail_selected;
        let bullet = if selected { "▸ " } else { "  " };
        let line = match node.default_for(def.key) {
            Some(value) => Line::from(vec![
                Span::raw(bullet),
                Span::styled(
                    format!("{} = {value}", def.key),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            None => {
                let hint = match def.default {
                    Some(d) => format!("{}   (provider default: {d})", def.key),
                    None => format!("{}   (unset)", def.key),
                };
                Line::from(vec![Span::raw(bullet), Span::styled(hint, dim)])
            }
        };
        if selected {
            lines.push(line.style(Style::default().add_modifier(Modifier::REVERSED)));
        } else {
            lines.push(line);
        }
    }
    // Registry params for this node's capabilities that params_for excluded
    // (provider-filtered, e.g. image.format on Vertex). Shown as dimmed,
    // non-selectable hints — detail_selected only ranges over `params`.
    for def in crate::params::PARAMS {
        if node.capabilities.contains(&def.capability) && !params.iter().any(|p| p.key == def.key) {
            lines.push(Line::styled(
                format!("  {}   (not available for this provider)", def.key),
                dim,
            ));
        }
    }
    lines.push(Line::styled(
        "  Enter: edit · params shown are valid for this node",
        dim,
    ));

    // --- Retirement warning ---------------------------------------------
    if let Some(warning) = retirement_warning(node.detail()) {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("⚠ {warning}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

/// A dimmed `key: value` detail line. Owns its content, so the returned line
/// does not borrow the inputs.
fn kv(key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key}: "), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

/// The optional status line above the footer.
fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(status) = &app.status_line {
        let widget = Paragraph::new(Line::styled(
            status.clone(),
            Style::default().fg(Color::Cyan),
        ));
        frame.render_widget(widget, area);
    }
}

/// The footer key legend.
fn draw_footer(frame: &mut Frame, area: Rect) {
    let legend = "↑/↓ select · Tab focus · Enter edit · a add · e edit · x delete · d default · k key · t test · q quit";
    let widget = Paragraph::new(Line::styled(legend, Style::default().fg(Color::DarkGray)));
    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiNode, Config, ProviderKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn node_with_caps(provider: ProviderKind, caps: Vec<Capability>) -> AiNode {
        let mut node = AiNode::new(provider);
        node.capabilities = caps;
        node
    }

    // --- capability_cell matrix ----------------------------------------

    #[test]
    fn capability_cell_dot_when_missing() {
        let config = Config::default();
        let node = node_with_caps(ProviderKind::OpenAi, vec![Capability::Chat]);
        assert_eq!(
            capability_cell(&config, "openai/x", &node, &Capability::Image),
            DOT
        );
    }

    #[test]
    fn capability_cell_check_when_present_not_default() {
        let config = Config::default();
        let node = node_with_caps(ProviderKind::OpenAi, vec![Capability::Chat]);
        assert_eq!(
            capability_cell(&config, "openai/x", &node, &Capability::Chat),
            CHECK
        );
    }

    #[test]
    fn capability_cell_star_when_present_and_default() {
        let mut config = Config::default();
        config
            .defaults
            .insert("chat".to_string(), "openai/x".to_string());
        let node = node_with_caps(ProviderKind::OpenAi, vec![Capability::Chat]);
        assert_eq!(
            capability_cell(&config, "openai/x", &node, &Capability::Chat),
            STAR
        );
    }

    #[test]
    fn capability_cell_check_when_default_points_elsewhere() {
        let mut config = Config::default();
        config
            .defaults
            .insert("chat".to_string(), "openai/other".to_string());
        let node = node_with_caps(ProviderKind::OpenAi, vec![Capability::Chat]);
        assert_eq!(
            capability_cell(&config, "openai/x", &node, &Capability::Chat),
            CHECK
        );
    }

    #[test]
    fn capability_cell_dot_when_missing_even_if_default_key_matches() {
        // Node is the recorded image default but lacks the image capability.
        let mut config = Config::default();
        config
            .defaults
            .insert("image".to_string(), "openai/x".to_string());
        let node = node_with_caps(ProviderKind::OpenAi, vec![Capability::Chat]);
        assert_eq!(
            capability_cell(&config, "openai/x", &node, &Capability::Image),
            DOT
        );
    }

    #[test]
    fn auth_label_renders_each_variant() {
        assert_eq!(
            auth_label(&Some(Auth::Env("OPENAI_API_KEY".into()))),
            "env: OPENAI_API_KEY"
        );
        assert_eq!(auth_label(&Some(Auth::Keychain(true))), "keychain");
        assert_eq!(auth_label(&Some(Auth::AzureCli(true))), "azure-cli");
        assert_eq!(auth_label(&None), "none");
    }

    // --- render smoke test ---------------------------------------------

    fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn two_node_config() -> Config {
        let mut config = Config::default();
        let mut chat = node_with_caps(ProviderKind::OpenAi, vec![Capability::Chat]);
        chat.model = Some("gpt-5.4".to_string());
        config.nodes.insert("openai/gpt-5.4".to_string(), chat);

        let img = node_with_caps(
            ProviderKind::OpenAi,
            vec![Capability::Chat, Capability::Image],
        );
        config.nodes.insert("openai/gpt-image-2".to_string(), img);

        // Make the image node the image default so a star renders.
        config
            .defaults
            .insert("image".to_string(), "openai/gpt-image-2".to_string());
        config
    }

    #[test]
    fn draw_renders_nodes_defaults_and_footer() {
        let app = App::new(two_node_config(), "ailloy");
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        // Node ids appear in the table.
        assert!(text.contains("openai/gpt-5.4"), "table shows first node id");
        assert!(
            text.contains("openai/gpt-image-2"),
            "table shows second node id"
        );
        // The image default renders a star somewhere.
        assert!(text.contains(STAR), "a default capability shows a star");
        // A param key for the selected node's detail pane.
        assert!(
            text.contains("chat.temperature"),
            "detail pane lists a param key"
        );
        // Footer legend.
        assert!(text.contains("Tab focus"), "footer legend is rendered");
    }

    #[test]
    fn draw_detail_lists_image_param_for_image_node() {
        let mut app = App::new(two_node_config(), "ailloy");
        // Select the image-capable node (BTreeMap order: gpt-image-2 < gpt-5.4).
        app.selected = app
            .node_ids()
            .iter()
            .position(|id| id == "openai/gpt-image-2")
            .unwrap();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(
            text.contains("image.quality"),
            "detail pane lists an image param for an image node"
        );
    }

    #[test]
    fn draw_detail_shows_provider_unavailable_params_dimmed() {
        // Vertex supports image, but the OpenAI-family-only image params
        // (image.format/compression/background) are provider-filtered out.
        let mut config = Config::default();
        config.nodes.insert(
            "vertex-ai/imagen".to_string(),
            node_with_caps(
                ProviderKind::VertexAi,
                vec![Capability::Chat, Capability::Image],
            ),
        );
        let app = App::new(config, "ailloy");
        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(
            text.contains("not available for this provider"),
            "provider-excluded params render the unavailable hint"
        );
        assert!(
            text.contains("image.format"),
            "the excluded param key itself is listed"
        );
        // Still-configurable params are listed normally too.
        assert!(text.contains("image.quality"));
    }

    #[test]
    fn draw_renders_edit_popup_with_title_and_hint() {
        let mut config = Config::default();
        config.nodes.insert(
            "openai/img".to_string(),
            node_with_caps(
                ProviderKind::OpenAi,
                vec![Capability::Chat, Capability::Image],
            ),
        );
        let mut app = App::new(config, "ailloy");
        let id = app.selected_node_id().unwrap();
        let node = &app.config.nodes[&id];
        let idx = params_for(&node.provider, &node.capabilities)
            .iter()
            .position(|d| d.key == "image.compression")
            .unwrap();
        app.focus = Focus::Detail;
        app.detail_selected = idx;
        app.mode = Mode::EditDefault {
            param_idx: idx,
            editor: Editor::text(""),
        };

        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let text = buffer_to_string(&terminal);
        assert!(
            text.contains("image.compression"),
            "popup title shows the param key"
        );
        assert!(text.contains("0..=100"), "popup shows the range hint");
        assert!(text.contains("Enter save"), "popup footer key hints render");
    }

    #[test]
    fn draw_handles_empty_config() {
        let app = App::new(Config::default(), "ailloy");
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        // Must not panic on an empty node set.
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let text = buffer_to_string(&terminal);
        assert!(text.contains("No nodes configured"));
    }
}
