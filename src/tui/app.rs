//! State model and pure reducer for the full-screen config TUI.
//!
//! [`App`] holds all UI state. [`App::handle_key`] is a pure reducer: it takes
//! a key event, mutates state, and returns an optional [`Effect`] for the event
//! loop in [`super::mod`] to execute (side effects like saving or quitting live
//! there, not here — keeping this unit-testable without a terminal).

use crossterm::event::{KeyCode, KeyEvent};

use crate::config::Config;
use crate::params::{self, ParamDef, ParamKind, params_for};
use crate::tui::actions;
use crate::tui::forms::{Editor, NodeForm};

/// The synthetic leading choice in an Enum default editor that clears the key.
pub(crate) const UNSET_CHOICE: &str = "(unset)";

/// Which pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The list of node IDs on the left.
    NodeList,
    /// The detail/parameter view for the selected node.
    Detail,
}

/// What the confirmation prompt will do if accepted.
// Variants constructed by the delete flow in Task 5.3.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Delete the node with this ID.
    DeleteNode { id: String },
}

/// The current interaction mode — a small state machine layered over
/// [`Focus`]. Browse is the resting state; the others are transient editors
/// and prompts (mostly wired up in later tasks).
// Non-Browse variants are constructed by the edit flows in Tasks 5.3/5.4.
#[allow(dead_code)]
pub enum Mode {
    /// Normal navigation.
    Browse,
    /// Editing a per-node default parameter value.
    EditDefault { param_idx: usize, editor: Editor },
    /// A yes/no confirmation prompt.
    Confirm {
        action: ConfirmAction,
        message: String,
    },
    /// Adding a brand-new node via a form.
    AddNode(NodeForm),
    /// Editing an existing node via a form.
    EditNode { id: String, form: NodeForm },
    /// Choosing which node becomes the default for a capability.
    SetDefaultFor { cap_idx: usize },
    /// Showing the result of a connectivity test.
    Test {
        node_id: String,
        result: Option<String>,
    },
}

/// A side effect the event loop must perform after a key is handled.
// Save/RunTest are returned by the edit and test flows in Tasks 5.3/5.4.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Persist the config to disk.
    Save,
    /// Run a connectivity test against the given node ID.
    RunTest(String),
    /// Exit the TUI.
    Quit,
}

/// All state for the config TUI.
pub struct App {
    /// The config being edited (loaded and saved by the event loop).
    pub config: Config,
    /// The consuming application's name, for display and status paths.
    pub app_name: String,
    /// Index of the selected node in [`App::node_ids`] order.
    pub selected: usize,
    /// Index of the selected row within the detail pane's parameter list.
    /// Only meaningful while [`Focus::Detail`] has focus. Reset to `0` whenever
    /// the node selection changes; clamped to the selected node's parameter
    /// count.
    pub detail_selected: usize,
    /// Which pane has focus.
    pub focus: Focus,
    /// Current interaction mode.
    pub mode: Mode,
    /// Whether the config has unsaved changes.
    pub dirty: bool,
    /// A transient status/help line shown at the bottom.
    pub status_line: Option<String>,
}

impl App {
    /// Build a fresh app in [`Mode::Browse`] with the first node selected.
    pub fn new(config: Config, app_name: &str) -> Self {
        App {
            config,
            app_name: app_name.to_string(),
            selected: 0,
            detail_selected: 0,
            focus: Focus::NodeList,
            mode: Mode::Browse,
            dirty: false,
            status_line: None,
        }
    }

    /// The node IDs in stable (sorted `BTreeMap`) order.
    // Consumed by the table/detail rendering in Tasks 5.2/5.3.
    #[allow(dead_code)]
    pub fn node_ids(&self) -> Vec<String> {
        self.config.nodes.keys().cloned().collect()
    }

    /// The ID of the currently selected node, if any.
    // Consumed by the edit/delete/test flows in Tasks 5.3/5.4.
    #[allow(dead_code)]
    pub fn selected_node_id(&self) -> Option<String> {
        self.node_ids().get(self.selected).cloned()
    }

    /// The number of tunable parameters shown in the detail pane for the
    /// currently selected node (via [`crate::params::params_for`]). Zero when
    /// no node is selected.
    pub fn selected_params_len(&self) -> usize {
        match self.selected_node_id() {
            Some(id) => match self.config.nodes.get(&id) {
                Some(node) => crate::params::params_for(&node.provider, &node.capabilities).len(),
                None => 0,
            },
            None => 0,
        }
    }

    /// Clamp [`App::detail_selected`] into the valid range for the selected
    /// node's parameter list, so it never points past the last row (or stays
    /// non-zero for a node with no parameters).
    fn clamp_detail_selected(&mut self) {
        let len = self.selected_params_len();
        let max = len.saturating_sub(1);
        if len == 0 {
            self.detail_selected = 0;
        } else if self.detail_selected > max {
            self.detail_selected = max;
        }
    }

    /// Pure reducer: apply a key event and return an [`Effect`] for the event
    /// loop to execute, if any.
    ///
    /// Only [`Mode::Browse`] keys are handled in this task; the other modes and
    /// their keys are added in later tasks. Unhandled keys are a no-op.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match self.mode {
            Mode::Browse => self.handle_browse_key(key),
            Mode::EditDefault { .. } => self.handle_edit_default_key(key),
            // Other modes are wired up in later tasks; ignore input for now.
            _ => None,
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let node_count = self.config.nodes.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(Effect::Quit),
            KeyCode::Up | KeyCode::Char('k') => {
                match self.focus {
                    // In the detail pane, Up/Down scroll the parameter list.
                    Focus::Detail => {
                        self.detail_selected = self.detail_selected.saturating_sub(1);
                    }
                    // In the node list, Up/Down change the selected node and
                    // reset the detail cursor to the top.
                    Focus::NodeList => {
                        let before = self.selected;
                        self.selected = self.selected.saturating_sub(1);
                        if self.selected != before {
                            self.detail_selected = 0;
                        }
                    }
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match self.focus {
                    Focus::Detail => {
                        let max = self.selected_params_len().saturating_sub(1);
                        if self.detail_selected < max {
                            self.detail_selected += 1;
                        }
                    }
                    Focus::NodeList => {
                        if node_count > 0 && self.selected < node_count - 1 {
                            self.selected += 1;
                            self.detail_selected = 0;
                        }
                    }
                }
                None
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::NodeList => Focus::Detail,
                    Focus::Detail => Focus::NodeList,
                };
                if self.focus == Focus::Detail {
                    self.clamp_detail_selected();
                }
                None
            }
            // Enter on an editable default row (detail pane) opens the editor.
            KeyCode::Enter => {
                if self.focus == Focus::Detail {
                    self.begin_edit_default();
                }
                None
            }
            // All other keys are extended in later tasks.
            _ => None,
        }
    }

    /// The [`ParamDef`] the detail cursor currently points at, if the selected
    /// node has an editable parameter there.
    fn param_def_at_cursor(&self) -> Option<&'static ParamDef> {
        let node_id = self.selected_node_id()?;
        let node = self.config.nodes.get(&node_id)?;
        params_for(&node.provider, &node.capabilities)
            .get(self.detail_selected)
            .copied()
    }

    /// The [`ParamDef`] currently being edited (by the mode's `param_idx`).
    fn editing_param_def(&self) -> Option<&'static ParamDef> {
        let param_idx = match &self.mode {
            Mode::EditDefault { param_idx, .. } => *param_idx,
            _ => return None,
        };
        let node_id = self.selected_node_id()?;
        let node = self.config.nodes.get(&node_id)?;
        params_for(&node.provider, &node.capabilities)
            .get(param_idx)
            .copied()
    }

    /// Enter [`Mode::EditDefault`] for the parameter under the detail cursor,
    /// building the appropriate editor for its [`ParamKind`].
    fn begin_edit_default(&mut self) {
        let Some(def) = self.param_def_at_cursor() else {
            return;
        };
        let idx = self.detail_selected;
        let node_id = match self.selected_node_id() {
            Some(id) => id,
            None => return,
        };
        // Current stored value (if any) for this key on the selected node.
        let current = self
            .config
            .nodes
            .get(&node_id)
            .and_then(|n| n.default_for(def.key))
            .map(str::to_string);

        let editor = match &def.kind {
            ParamKind::Enum(choices) => {
                let mut options = vec![UNSET_CHOICE.to_string()];
                options.extend(choices.iter().map(|c| c.to_string()));
                // Index 0 is "(unset)"; a stored value maps to its slot + 1.
                let selected = current
                    .as_deref()
                    .and_then(|v| choices.iter().position(|c| *c == v))
                    .map(|p| p + 1)
                    .unwrap_or(0);
                Editor::select(options, selected)
            }
            // UInt / Float / Size are free-text with live validation.
            _ => Editor::text(current.unwrap_or_default()),
        };

        self.mode = Mode::EditDefault {
            param_idx: idx,
            editor,
        };
    }

    /// Key handling while in [`Mode::EditDefault`].
    fn handle_edit_default_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                None
            }
            KeyCode::Enter => self.commit_edit_default(),
            _ => {
                // Look up the def first so the immutable borrow ends before we
                // take a mutable borrow of the editor.
                let def = self.editing_param_def();
                if let Mode::EditDefault { editor, .. } = &mut self.mode {
                    if let Some((choices, selected)) = editor.choices.as_mut() {
                        // Select editor: Up/Down move the highlight.
                        match key.code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                *selected = selected.saturating_sub(1);
                            }
                            KeyCode::Down | KeyCode::Char('j') if *selected + 1 < choices.len() => {
                                *selected += 1;
                            }
                            _ => {}
                        }
                        editor.value = choices.get(*selected).cloned().unwrap_or_default();
                    } else {
                        // Text editor: edit the value at the cursor.
                        match key.code {
                            KeyCode::Char(c) => {
                                editor.value.insert(editor.cursor, c);
                                editor.cursor += c.len_utf8();
                            }
                            KeyCode::Backspace => {
                                if editor.cursor > 0 {
                                    let prev = editor.value[..editor.cursor]
                                        .chars()
                                        .next_back()
                                        .map(char::len_utf8)
                                        .unwrap_or(1);
                                    editor.cursor -= prev;
                                    editor.value.remove(editor.cursor);
                                }
                            }
                            KeyCode::Left if editor.cursor > 0 => {
                                let prev = editor.value[..editor.cursor]
                                    .chars()
                                    .next_back()
                                    .map(char::len_utf8)
                                    .unwrap_or(1);
                                editor.cursor -= prev;
                            }
                            KeyCode::Right if editor.cursor < editor.value.len() => {
                                let next = editor.value[editor.cursor..]
                                    .chars()
                                    .next()
                                    .map(char::len_utf8)
                                    .unwrap_or(1);
                                editor.cursor += next;
                            }
                            _ => {}
                        }
                        // Live validation: empty clears the key (valid); any
                        // other value must pass the param's validator.
                        if let Some(def) = def {
                            let trimmed = editor.value.trim();
                            editor.error = if trimmed.is_empty() {
                                None
                            } else {
                                params::validate_value(def, trimmed).err()
                            };
                        }
                    }
                }
                None
            }
        }
    }

    /// Commit the current [`Mode::EditDefault`] value: write it through
    /// [`actions::set_node_default`], returning to Browse with a save effect on
    /// success. A live validation error present makes Enter a no-op.
    fn commit_edit_default(&mut self) -> Option<Effect> {
        // Resolve the value the editor commits to (None clears the key).
        let value: Option<String> = match &self.mode {
            Mode::EditDefault { editor, .. } => {
                if editor.error.is_some() {
                    // Standing validation error: refuse to commit.
                    return None;
                }
                match &editor.choices {
                    Some((choices, selected)) => match choices.get(*selected) {
                        Some(choice) if choice == UNSET_CHOICE => None,
                        Some(choice) => Some(choice.clone()),
                        None => None,
                    },
                    None => {
                        let trimmed = editor.value.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        }
                    }
                }
            }
            _ => return None,
        };

        let node_id = self.selected_node_id()?;
        let key = self.editing_param_def()?.key.to_string();

        match actions::set_node_default(&mut self.config, &node_id, &key, value.as_deref()) {
            Ok(()) => {
                self.mode = Mode::Browse;
                self.dirty = true;
                self.status_line = Some(format!("saved {key}"));
                Some(Effect::Save)
            }
            Err(e) => {
                // Should not happen given live validation, but surface it
                // rather than silently discarding the input.
                if let Mode::EditDefault { editor, .. } = &mut self.mode {
                    editor.error = Some(e.to_string());
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiNode, Capability, Config, ProviderKind};
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn config_with(n: usize) -> Config {
        let mut config = Config::default();
        for i in 0..n {
            config.nodes.insert(
                format!("openai/node-{i}"),
                AiNode::new(ProviderKind::OpenAi),
            );
        }
        config
    }

    #[test]
    fn selection_moves_down_and_clamps_to_last() {
        let mut app = App::new(config_with(2), "ailloy");
        assert_eq!(app.selected, 0);
        assert!(app.handle_key(key(KeyCode::Down)).is_none());
        assert_eq!(app.selected, 1);
        // Already at the last node: further Down stays clamped.
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn selection_up_clamps_at_zero() {
        let mut app = App::new(config_with(2), "ailloy");
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn q_in_browse_returns_quit_effect() {
        let mut app = App::new(config_with(1), "ailloy");
        assert_eq!(app.handle_key(key(KeyCode::Char('q'))), Some(Effect::Quit));
    }

    #[test]
    fn esc_in_browse_returns_quit_effect() {
        let mut app = App::new(config_with(1), "ailloy");
        assert_eq!(app.handle_key(key(KeyCode::Esc)), Some(Effect::Quit));
    }

    #[test]
    fn tab_toggles_focus() {
        let mut app = App::new(config_with(1), "ailloy");
        assert_eq!(app.focus, Focus::NodeList);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Detail);
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::NodeList);
    }

    #[test]
    fn browse_keys_do_not_set_dirty() {
        let mut app = App::new(config_with(2), "ailloy");
        assert!(!app.dirty);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Tab));
        assert!(!app.dirty, "navigation must not mark the config dirty");
    }

    #[test]
    fn selected_node_id_tracks_selection() {
        let mut app = App::new(config_with(2), "ailloy");
        assert_eq!(app.selected_node_id().as_deref(), Some("openai/node-0"));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected_node_id().as_deref(), Some("openai/node-1"));
    }

    /// A node with chat + image capabilities so it exposes several params.
    fn config_with_capable_nodes() -> Config {
        let mut config = Config::default();
        for i in 0..2 {
            let mut node = AiNode::new(ProviderKind::OpenAi);
            node.capabilities = vec![Capability::Chat, Capability::Image];
            config.nodes.insert(format!("openai/node-{i}"), node);
        }
        config
    }

    #[test]
    fn detail_focus_up_down_scrolls_param_list() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        let param_count = app.selected_params_len();
        assert!(param_count > 1, "test needs a node with multiple params");
        app.focus = Focus::Detail;
        assert_eq!(app.detail_selected, 0);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.detail_selected, 1);
        // Up clamps at zero.
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.detail_selected, 0);
    }

    #[test]
    fn detail_selected_clamps_to_last_param() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        let param_count = app.selected_params_len();
        app.focus = Focus::Detail;
        // Press Down far more than there are params.
        for _ in 0..(param_count + 5) {
            app.handle_key(key(KeyCode::Down));
        }
        assert_eq!(app.detail_selected, param_count - 1);
    }

    #[test]
    fn detail_selected_resets_when_node_selection_changes() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        app.focus = Focus::Detail;
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.detail_selected, 1);
        // Move back to the node list and change the selected node.
        app.focus = Focus::NodeList;
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 1);
        assert_eq!(app.detail_selected, 0, "changing node resets detail cursor");
    }

    #[test]
    fn detail_selected_resets_when_node_selection_changes_up() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        // Select the second node, then scroll the detail cursor down.
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected, 1);
        app.focus = Focus::Detail;
        app.handle_key(key(KeyCode::Down));
        assert!(app.detail_selected > 0);
        // Back in the node list, Up changes the node and resets the cursor.
        app.focus = Focus::NodeList;
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected, 0);
        assert_eq!(app.detail_selected, 0, "changing node (Up) resets cursor");
    }

    #[test]
    fn tab_into_detail_clamps_stale_cursor() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        // Simulate a stale cursor beyond the param range.
        app.detail_selected = 999;
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Detail);
        assert_eq!(app.detail_selected, app.selected_params_len() - 1);
    }

    #[test]
    fn selected_params_len_zero_for_empty_config() {
        let app = App::new(Config::default(), "ailloy");
        assert_eq!(app.selected_params_len(), 0);
    }

    // --- Task 5.3: per-node default editing -----------------------------

    /// Index of `key` in the selected node's parameter list.
    fn param_index(app: &App, key: &str) -> usize {
        let id = app.selected_node_id().unwrap();
        let node = &app.config.nodes[&id];
        params_for(&node.provider, &node.capabilities)
            .iter()
            .position(|d| d.key == key)
            .unwrap_or_else(|| panic!("param '{key}' not found for selected node"))
    }

    /// Enter Browse→edit for the given param key on the selected node.
    fn open_editor(app: &mut App, key: &str) {
        app.focus = Focus::Detail;
        app.detail_selected = param_index(app, key);
        app.handle_key(key_ev(KeyCode::Enter));
    }

    fn key_ev(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_opens_select_editor_for_enum_param() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.quality");
        match &app.mode {
            Mode::EditDefault { editor, .. } => {
                let (choices, _) = editor.choices.as_ref().expect("enum → select editor");
                assert_eq!(choices[0], UNSET_CHOICE, "leading (unset) choice");
                assert!(choices.iter().any(|c| c == "high"), "enum values present");
            }
            _ => panic!("expected EditDefault mode"),
        }
    }

    #[test]
    fn enter_opens_text_editor_for_uint_param() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.compression");
        match &app.mode {
            Mode::EditDefault { editor, .. } => {
                assert!(editor.choices.is_none(), "uint → free-text editor");
                assert_eq!(editor.value, "", "no stored value → empty");
                assert_eq!(editor.cursor, 0);
            }
            _ => panic!("expected EditDefault mode"),
        }
    }

    #[test]
    fn typing_and_backspace_update_text_editor() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.compression");
        app.handle_key(key_ev(KeyCode::Char('5')));
        app.handle_key(key_ev(KeyCode::Char('0')));
        match &app.mode {
            Mode::EditDefault { editor, .. } => {
                assert_eq!(editor.value, "50");
                assert_eq!(editor.cursor, 2);
                assert!(editor.error.is_none(), "50 is a valid compression");
            }
            _ => panic!("expected EditDefault mode"),
        }
        app.handle_key(key_ev(KeyCode::Backspace));
        match &app.mode {
            Mode::EditDefault { editor, .. } => {
                assert_eq!(editor.value, "5");
                assert_eq!(editor.cursor, 1);
            }
            _ => panic!("expected EditDefault mode"),
        }
    }

    #[test]
    fn invalid_text_sets_error_and_enter_is_noop() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.compression");
        // 999 is out of the 0..=100 range.
        for c in ['9', '9', '9'] {
            app.handle_key(key_ev(KeyCode::Char(c)));
        }
        match &app.mode {
            Mode::EditDefault { editor, .. } => {
                assert!(editor.error.is_some(), "out-of-range value sets an error");
            }
            _ => panic!("expected EditDefault mode"),
        }
        // Enter with an error present must not commit.
        let effect = app.handle_key(key_ev(KeyCode::Enter));
        assert!(effect.is_none(), "Enter is a no-op while invalid");
        assert!(
            matches!(app.mode, Mode::EditDefault { .. }),
            "stays in edit mode"
        );
        assert!(!app.dirty);
        let id = app.selected_node_id().unwrap();
        assert!(app.config.nodes[&id].node_defaults.is_none());
    }

    #[test]
    fn valid_text_enter_commits_and_saves() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.compression");
        app.handle_key(key_ev(KeyCode::Char('5')));
        app.handle_key(key_ev(KeyCode::Char('0')));
        let effect = app.handle_key(key_ev(KeyCode::Enter));
        assert_eq!(effect, Some(Effect::Save));
        assert!(matches!(app.mode, Mode::Browse), "back to Browse");
        assert!(app.dirty);
        assert_eq!(app.status_line.as_deref(), Some("saved image.compression"));
        let id = app.selected_node_id().unwrap();
        assert_eq!(
            app.config.nodes[&id].default_for("image.compression"),
            Some("50")
        );
    }

    #[test]
    fn select_enter_commits_chosen_value() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.quality");
        // choices: ["(unset)", "low", "medium", "high", "auto"] → Down → "low".
        app.handle_key(key_ev(KeyCode::Down));
        let effect = app.handle_key(key_ev(KeyCode::Enter));
        assert_eq!(effect, Some(Effect::Save));
        let id = app.selected_node_id().unwrap();
        assert_eq!(
            app.config.nodes[&id].default_for("image.quality"),
            Some("low")
        );
    }

    #[test]
    fn select_unset_removes_existing_key() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        // Pre-set a stored default so the editor opens on it.
        let id = app.selected_node_id().unwrap();
        actions::set_node_default(&mut app.config, &id, "image.quality", Some("high")).unwrap();

        open_editor(&mut app, "image.quality");
        // Editor should start on the stored value ("high"); move to (unset).
        for _ in 0..5 {
            app.handle_key(key_ev(KeyCode::Up));
        }
        let effect = app.handle_key(key_ev(KeyCode::Enter));
        assert_eq!(effect, Some(Effect::Save));
        assert_eq!(app.config.nodes[&id].default_for("image.quality"), None);
        // Map dropped once empty.
        assert!(app.config.nodes[&id].node_defaults.is_none());
    }

    #[test]
    fn esc_cancels_without_touching_config() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        open_editor(&mut app, "image.compression");
        app.handle_key(key_ev(KeyCode::Char('7')));
        let effect = app.handle_key(key_ev(KeyCode::Esc));
        assert!(effect.is_none());
        assert!(matches!(app.mode, Mode::Browse));
        assert!(!app.dirty);
        let id = app.selected_node_id().unwrap();
        assert!(app.config.nodes[&id].node_defaults.is_none());
    }
}
