//! State model and pure reducer for the full-screen config TUI.
//!
//! [`App`] holds all UI state. [`App::handle_key`] is a pure reducer: it takes
//! a key event, mutates state, and returns an optional [`Effect`] for the event
//! loop in [`super::mod`] to execute (side effects like saving or quitting live
//! there, not here — keeping this unit-testable without a terminal).

use crossterm::event::{KeyCode, KeyEvent};

use crate::config::Config;
use crate::tui::forms::{Editor, NodeForm};

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
            // All other keys are extended in later tasks.
            _ => None,
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
}
