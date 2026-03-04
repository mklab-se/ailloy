//! Custom crossterm-based TUI for `ailloy config`.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{self, ClearType};
use crossterm::{cursor, execute, queue};

use ailloy::config::{ALL_TASKS, Config, ProviderKind};

/// An item in the TUI list — either a non-selectable header or a selectable row.
#[derive(Clone)]
enum Row {
    /// Section header like "Chat Completion Models:"
    Header(String),
    /// A provider entry within a section.
    Provider {
        name: String,
        kind: ProviderKind,
        detail: String,
        is_default: bool,
        task: String,
    },
    /// "<add new>" entry at the end of a section.
    AddNew { task: String },
    /// Empty line between sections.
    Blank,
}

impl Row {
    fn is_selectable(&self) -> bool {
        matches!(self, Row::Provider { .. } | Row::AddNew { .. })
    }
}

/// The action the user selected.
pub enum TuiAction {
    /// Edit an existing provider.
    EditProvider(String),
    /// Toggle default status for a provider on a task.
    ToggleDefault { name: String, task: String },
    /// Delete a provider.
    DeleteProvider(String),
    /// Add a new provider (optionally filtered to a task).
    AddProvider(Option<String>),
    /// Quit the TUI.
    Quit,
}

/// Build the list of rows from the current config.
fn build_rows(config: &Config) -> Vec<Row> {
    let mut rows = Vec::new();

    // Each provider appears exactly once.
    // - If it's the default for a task, it appears under that task.
    // - Otherwise, it appears under the first task it supports.
    let mut placed: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut task_providers: Vec<Vec<(String, ProviderKind, String, bool)>> =
        vec![Vec::new(); ALL_TASKS.len()];

    // First pass: place defaults under their task
    for (task_idx, &(task_key, _)) in ALL_TASKS.iter().enumerate() {
        if let Some(default_name) = config.defaults.get(task_key) {
            if let Some(pc) = config.providers.get(default_name) {
                if !placed.contains(default_name) {
                    let detail = pc
                        .deployment
                        .as_deref()
                        .or(pc.model.as_deref())
                        .or(pc.binary.as_deref())
                        .unwrap_or("?");
                    task_providers[task_idx].push((
                        default_name.clone(),
                        pc.kind.clone(),
                        detail.to_string(),
                        true,
                    ));
                    placed.insert(default_name.clone());
                }
            }
        }
    }

    // Second pass: place remaining providers under first matching task
    for (name, pc) in &config.providers {
        if placed.contains(name) {
            continue;
        }
        for (task_idx, &(task_key, _)) in ALL_TASKS.iter().enumerate() {
            if pc.kind.supports_task(task_key) {
                let detail = pc
                    .deployment
                    .as_deref()
                    .or(pc.model.as_deref())
                    .or(pc.binary.as_deref())
                    .unwrap_or("?");
                task_providers[task_idx].push((
                    name.clone(),
                    pc.kind.clone(),
                    detail.to_string(),
                    false,
                ));
                placed.insert(name.clone());
                break;
            }
        }
    }

    // Sort ALL providers alphabetically within each section
    for providers in &mut task_providers {
        providers.sort_by(|a, b| a.0.cmp(&b.0));
    }

    let task_labels = [
        "Chat Completion Models:",
        "Image Generation Models:",
        "Embedding Models:",
    ];

    for (task_idx, &(task_key, _)) in ALL_TASKS.iter().enumerate() {
        if task_idx > 0 {
            rows.push(Row::Blank);
        }
        rows.push(Row::Header(task_labels[task_idx].to_string()));
        rows.push(Row::Blank);

        for (name, kind, detail, is_default) in task_providers[task_idx].iter() {
            rows.push(Row::Provider {
                name: name.clone(),
                kind: kind.clone(),
                detail: detail.clone(),
                is_default: *is_default,
                task: task_key.to_string(),
            });
        }

        rows.push(Row::AddNew {
            task: task_key.to_string(),
        });
    }

    rows
}

/// Find the next selectable index from `from` in direction `delta` (+1 or -1).
fn next_selectable(rows: &[Row], from: usize, delta: isize) -> usize {
    let len = rows.len() as isize;
    let mut pos = from as isize + delta;
    loop {
        if pos < 0 {
            pos = len - 1;
        } else if pos >= len {
            pos = 0;
        }
        if rows[pos as usize].is_selectable() {
            return pos as usize;
        }
        pos += delta;
    }
}

/// Find the row index for a provider by name, or the first selectable row.
fn find_cursor_for(rows: &[Row], name: Option<&str>) -> usize {
    if let Some(name) = name {
        for (i, row) in rows.iter().enumerate() {
            if let Row::Provider { name: n, .. } = row {
                if n == name {
                    return i;
                }
            }
        }
    }
    // Fallback: first selectable row
    for (i, row) in rows.iter().enumerate() {
        if row.is_selectable() {
            return i;
        }
    }
    0
}

/// Render the main TUI screen.
fn render(stdout: &mut io::Stdout, rows: &[Row], cursor_idx: usize) -> io::Result<()> {
    queue!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All)
    )?;

    write!(stdout, "-- Ailloy Configuration --")?;

    let mut line: u16 = 2;

    let max_name_len = rows
        .iter()
        .filter_map(|r| match r {
            Row::Provider { name, .. } => Some(name.len()),
            _ => None,
        })
        .max()
        .unwrap_or(12)
        .max(9);

    for (idx, row) in rows.iter().enumerate() {
        queue!(stdout, cursor::MoveTo(0, line))?;
        match row {
            Row::Header(label) => {
                write!(stdout, "{label}")?;
            }
            Row::Provider {
                name,
                kind,
                detail,
                is_default,
                ..
            } => {
                let prefix = if *is_default { "★ " } else { "  " };
                let pointer = if idx == cursor_idx { "> " } else { "  " };
                write!(
                    stdout,
                    "{pointer}{prefix}{name:<width$} ({kind}, {detail})",
                    width = max_name_len
                )?;
            }
            Row::AddNew { .. } => {
                let pointer = if idx == cursor_idx { "> " } else { "  " };
                write!(stdout, "{pointer}  <add new>")?;
            }
            Row::Blank => {}
        }
        line += 1;
    }

    line += 1;
    queue!(stdout, cursor::MoveTo(0, line))?;
    write!(
        stdout,
        "Press <Space> to set as default, <Enter> to edit, <Delete>/<Backspace> to delete, <A> to add new, <Q>/<ESC> to quit."
    )?;

    stdout.flush()?;
    Ok(())
}

/// Run the main config TUI. Returns the action and the name of the currently selected provider
/// (used to restore cursor position on re-entry).
pub fn run_tui(config: &Config, restore_cursor: Option<&str>) -> io::Result<(TuiAction, String)> {
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        cursor::Hide,
        terminal::Clear(ClearType::All)
    )?;

    let rows = build_rows(config);
    let mut cursor_idx = find_cursor_for(&rows, restore_cursor);

    let result = loop {
        render(&mut stdout, &rows, cursor_idx)?;

        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    cursor_idx = next_selectable(&rows, cursor_idx, -1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    cursor_idx = next_selectable(&rows, cursor_idx, 1);
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    break TuiAction::Quit;
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break TuiAction::Quit;
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    let task = match &rows[cursor_idx] {
                        Row::Provider { task, .. } | Row::AddNew { task } => Some(task.clone()),
                        _ => None,
                    };
                    break TuiAction::AddProvider(task);
                }
                KeyCode::Char(' ') => {
                    if let Row::Provider { name, task, .. } = &rows[cursor_idx] {
                        break TuiAction::ToggleDefault {
                            name: name.clone(),
                            task: task.clone(),
                        };
                    }
                }
                KeyCode::Enter => match &rows[cursor_idx] {
                    Row::Provider { name, .. } => {
                        break TuiAction::EditProvider(name.clone());
                    }
                    Row::AddNew { task } => {
                        break TuiAction::AddProvider(Some(task.clone()));
                    }
                    _ => {}
                },
                KeyCode::Delete | KeyCode::Backspace => {
                    if let Row::Provider { name, .. } = &rows[cursor_idx] {
                        break TuiAction::DeleteProvider(name.clone());
                    }
                }
                _ => {}
            }
        }
    };

    // Determine which provider the cursor is on for restore
    let cursor_name = match &rows[cursor_idx] {
        Row::Provider { name, .. } => name.clone(),
        _ => String::new(),
    };

    execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show)?;
    terminal::disable_raw_mode()?;

    Ok((result, cursor_name))
}

// ---------------------------------------------------------------------------
// Reusable crossterm-based select and confirm — replaces inquire Select/Confirm
// ---------------------------------------------------------------------------

/// A crossterm-based selection menu matching the TUI style.
/// Returns `Some(index)` on Enter, `None` on ESC/q.
pub fn tui_select(title: &str, options: &[String]) -> io::Result<Option<usize>> {
    tui_select_with_default(title, options, 0)
}

/// A crossterm-based selection menu with a pre-selected default index.
pub fn tui_select_with_default(
    title: &str,
    options: &[String],
    default: usize,
) -> io::Result<Option<usize>> {
    if options.is_empty() {
        return Ok(None);
    }

    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, cursor::Hide)?;

    let mut cursor_idx = default.min(options.len() - 1);

    let result = loop {
        execute!(
            stdout,
            cursor::MoveTo(0, 0),
            terminal::Clear(ClearType::All)
        )?;

        write!(stdout, "-- {title} --\r\n\r\n")?;
        for (i, option) in options.iter().enumerate() {
            let pointer = if i == cursor_idx { "> " } else { "  " };
            write!(stdout, "{pointer}{option}\r\n")?;
        }
        write!(stdout, "\r\nPress <Enter> to select, <ESC> to cancel.")?;
        stdout.flush()?;

        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    cursor_idx = cursor_idx.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    cursor_idx = (cursor_idx + 1).min(options.len() - 1);
                }
                KeyCode::Enter => {
                    break Some(cursor_idx);
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    break None;
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break None;
                }
                _ => {}
            }
        }
    };

    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Show
    )?;
    terminal::disable_raw_mode()?;

    Ok(result)
}

/// A crossterm-based yes/no confirmation prompt.
/// Returns `true` for y, `false` for n/ESC/Enter (default=No).
pub fn tui_confirm(prompt: &str) -> io::Result<bool> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;

    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    write!(stdout, "{prompt} (y/N) ")?;
    stdout.flush()?;

    let result = loop {
        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => break true,
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                    break false;
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    break false;
                }
                _ => {}
            }
        }
    };

    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    terminal::disable_raw_mode()?;

    Ok(result)
}
