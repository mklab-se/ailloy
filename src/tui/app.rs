//! State model and pure reducer for the full-screen config TUI.
//!
//! [`App`] holds all UI state. [`App::handle_key`] is a pure reducer: it takes
//! a key event, mutates state, and returns an optional [`Effect`] for the event
//! loop in [`super::mod`] to execute (side effects like saving or quitting live
//! there, not here — keeping this unit-testable without a terminal).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::{Capability, Config, ProviderKind};
use crate::params::{self, ParamDef, ParamKind, params_for};
use crate::tui::actions;
use crate::tui::forms::{Editor, FieldKey, FieldKind, FormField, NodeForm};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Delete the node with this ID.
    DeleteNode { id: String },
}

/// The current interaction mode — a small state machine layered over
/// [`Focus`]. Browse is the resting state; the others are transient editors
/// and prompts.
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
    /// Choosing which capability of a node to make the default for.
    SetDefaultFor {
        node_id: String,
        caps: Vec<Capability>,
        selected: usize,
    },
    /// Entering an API key to store in the OS keychain for a node.
    Keychain { node_id: String, editor: Editor },
    /// Showing the result of a connectivity test.
    Test {
        node_id: String,
        result: Option<String>,
    },
}

/// A side effect the event loop must perform after a key is handled.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Persist the config to disk.
    Save,
    /// Run a connectivity test against the given node ID.
    RunTest(String),
    /// Store a secret in the OS keychain for a node and switch it to keychain
    /// auth (guarded by the `keychain` feature in the event loop).
    StoreKeychain { node_id: String, secret: String },
    /// Run Azure/Foundry discovery for the given provider, prefilling the form.
    Discover(ProviderKind),
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
    /// The ID of the node most recently saved via a form (for single-form
    /// sessions to report back what was added/edited).
    pub last_saved_node: Option<String>,
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
            last_saved_node: None,
        }
    }

    /// The node IDs in stable (sorted `BTreeMap`) order.
    pub fn node_ids(&self) -> Vec<String> {
        self.config.nodes.keys().cloned().collect()
    }

    /// The ID of the currently selected node, if any.
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
            Mode::AddNode(_) | Mode::EditNode { .. } => self.handle_form_key(key),
            Mode::Confirm { .. } => self.handle_confirm_key(key),
            Mode::SetDefaultFor { .. } => self.handle_set_default_key(key),
            Mode::Keychain { .. } => self.handle_keychain_key(key),
            Mode::Test { .. } => self.handle_test_key(key),
        }
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let node_count = self.config.nodes.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(Effect::Quit),
            KeyCode::Up => {
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
            // --- Node lifecycle keys ------------------------------------
            KeyCode::Char('a') => {
                self.mode = Mode::AddNode(NodeForm::new());
                None
            }
            KeyCode::Char('e') => {
                if let Some(id) = self.selected_node_id() {
                    if let Some(node) = self.config.nodes.get(&id) {
                        let form = NodeForm::from_node(&id, node);
                        self.mode = Mode::EditNode { id, form };
                    }
                }
                None
            }
            KeyCode::Char('x') => {
                if let Some(id) = self.selected_node_id() {
                    let message = format!("Delete node '{id}'? (y/n)");
                    self.mode = Mode::Confirm {
                        action: ConfirmAction::DeleteNode { id },
                        message,
                    };
                }
                None
            }
            KeyCode::Char('d') => {
                if let Some(id) = self.selected_node_id() {
                    let caps = self
                        .config
                        .nodes
                        .get(&id)
                        .map(|n| n.capabilities.clone())
                        .unwrap_or_default();
                    if caps.is_empty() {
                        self.status_line = Some(format!(
                            "node '{id}' has no capabilities to set a default for"
                        ));
                    } else {
                        self.mode = Mode::SetDefaultFor {
                            node_id: id,
                            caps,
                            selected: 0,
                        };
                    }
                }
                None
            }
            KeyCode::Char('k') => {
                if let Some(id) = self.selected_node_id() {
                    self.mode = Mode::Keychain {
                        node_id: id,
                        editor: Editor::text(""),
                    };
                }
                None
            }
            KeyCode::Char('t') => self.selected_node_id().map(Effect::RunTest),
            // All other keys are a no-op in Browse.
            _ => None,
        }
    }

    /// Set [`App::selected`] to point at `id` if it exists.
    fn select_node(&mut self, id: &str) {
        if let Some(pos) = self.node_ids().iter().position(|n| n == id) {
            self.selected = pos;
            self.detail_selected = 0;
        }
    }

    // --- Confirm ---------------------------------------------------------

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let action = match &self.mode {
                    Mode::Confirm { action, .. } => action.clone(),
                    _ => return None,
                };
                match action {
                    ConfirmAction::DeleteNode { id } => {
                        match actions::delete_node(&mut self.config, &id) {
                            Ok(()) => {
                                self.mode = Mode::Browse;
                                self.dirty = true;
                                self.clamp_selection();
                                self.status_line = Some(format!("deleted {id}"));
                                Some(Effect::Save)
                            }
                            Err(e) => {
                                self.mode = Mode::Browse;
                                self.status_line = Some(e.to_string());
                                None
                            }
                        }
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.mode = Mode::Browse;
                None
            }
            _ => None,
        }
    }

    /// Clamp [`App::selected`] into range after a deletion.
    fn clamp_selection(&mut self) {
        let count = self.config.nodes.len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
        self.detail_selected = 0;
    }

    // --- SetDefaultFor ---------------------------------------------------

    fn handle_set_default_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Mode::SetDefaultFor { selected, .. } = &mut self.mode {
                    *selected = selected.saturating_sub(1);
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Mode::SetDefaultFor { selected, caps, .. } = &mut self.mode {
                    if *selected + 1 < caps.len() {
                        *selected += 1;
                    }
                }
                None
            }
            KeyCode::Enter => {
                let (node_id, cap_key) = match &self.mode {
                    Mode::SetDefaultFor {
                        node_id,
                        caps,
                        selected,
                    } => (
                        node_id.clone(),
                        caps.get(*selected)?.config_key().to_string(),
                    ),
                    _ => return None,
                };
                match actions::set_capability_default(&mut self.config, &cap_key, &node_id) {
                    Ok(()) => {
                        self.mode = Mode::Browse;
                        self.dirty = true;
                        self.status_line = Some(format!("{node_id} is now the {cap_key} default"));
                        Some(Effect::Save)
                    }
                    Err(e) => {
                        self.mode = Mode::Browse;
                        self.status_line = Some(e.to_string());
                        None
                    }
                }
            }
            _ => None,
        }
    }

    // --- Keychain --------------------------------------------------------

    fn handle_keychain_key(&mut self, key: KeyEvent) -> Option<Effect> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                None
            }
            KeyCode::Enter => {
                let (node_id, secret) = match &self.mode {
                    Mode::Keychain { node_id, editor } => {
                        (node_id.clone(), editor.value.trim().to_string())
                    }
                    _ => return None,
                };
                if secret.is_empty() {
                    if let Mode::Keychain { editor, .. } = &mut self.mode {
                        editor.error = Some("enter a non-empty key".to_string());
                    }
                    return None;
                }
                self.mode = Mode::Browse;
                Some(Effect::StoreKeychain { node_id, secret })
            }
            _ => {
                if let Mode::Keychain { editor, .. } = &mut self.mode {
                    edit_text(editor, key.code);
                }
                None
            }
        }
    }

    // --- Test ------------------------------------------------------------

    fn handle_test_key(&mut self, _key: KeyEvent) -> Option<Effect> {
        // Any key dismisses the result popup.
        self.mode = Mode::Browse;
        None
    }

    // --- Node form (Add / Edit) -----------------------------------------

    /// A mutable borrow of the form driving [`Mode::AddNode`]/[`Mode::EditNode`].
    fn active_form_mut(&mut self) -> Option<&mut NodeForm> {
        match &mut self.mode {
            Mode::AddNode(form) => Some(form),
            Mode::EditNode { form, .. } => Some(form),
            _ => None,
        }
    }

    fn handle_form_key(&mut self, key: KeyEvent) -> Option<Effect> {
        // Ctrl+S commits from any field.
        if matches!(key.code, KeyCode::Char('s')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.commit_form();
        }
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                None
            }
            KeyCode::Up => {
                if let Some(form) = self.active_form_mut() {
                    form.active = form.active.saturating_sub(1);
                }
                None
            }
            KeyCode::Down | KeyCode::Tab => {
                if let Some(form) = self.active_form_mut() {
                    if form.active + 1 < form.fields.len() {
                        form.active += 1;
                    }
                }
                None
            }
            code => self.handle_form_field_key(code),
        }
    }

    /// Handle a key aimed at the active field of the form.
    fn handle_form_field_key(&mut self, code: KeyCode) -> Option<Effect> {
        // Category of the active field, read without holding a borrow.
        enum Cat {
            Text,
            Select,
            Toggles,
            Action,
        }
        let (active, field_key, cat) = {
            let form = self.active_form_mut()?;
            let active = form.active;
            let field_key = form.fields[active].key;
            let cat = match &form.fields[active].kind {
                FieldKind::Text { .. } => Cat::Text,
                FieldKind::Select { .. } => Cat::Select,
                FieldKind::Toggles { .. } => Cat::Toggles,
                FieldKind::Action => Cat::Action,
            };
            (active, field_key, cat)
        };

        match cat {
            Cat::Text => {
                if let Some(form) = self.active_form_mut() {
                    if let FieldKind::Text { value, cursor } = &mut form.fields[active].kind {
                        edit_text_parts(value, cursor, code);
                    }
                }
                None
            }
            Cat::Select => {
                let mut changed = false;
                if let Some(form) = self.active_form_mut() {
                    if let FieldKind::Select { options, selected } = &mut form.fields[active].kind {
                        let n = options.len();
                        if n > 0 {
                            let forward = matches!(
                                code,
                                KeyCode::Right
                                    | KeyCode::Char(' ')
                                    | KeyCode::Enter
                                    | KeyCode::Char('l')
                            );
                            let backward = matches!(code, KeyCode::Left | KeyCode::Char('h'));
                            if forward {
                                *selected = (*selected + 1) % n;
                                changed = true;
                            } else if backward {
                                *selected = (*selected + n - 1) % n;
                                changed = true;
                            }
                        }
                    }
                }
                if changed {
                    match field_key {
                        FieldKey::Provider => {
                            let sel = self
                                .active_form_mut()
                                .and_then(|f| match &f.fields[active].kind {
                                    FieldKind::Select { selected, .. } => Some(*selected),
                                    _ => None,
                                })
                                .unwrap_or(0);
                            if let Some(form) = self.active_form_mut() {
                                form.provider = crate::tui::forms::PROVIDER_ORDER[sel].clone();
                                form.rebuild();
                                form.active = 0;
                            }
                        }
                        FieldKey::Auth => self.sync_api_key_field(),
                        _ => {}
                    }
                }
                None
            }
            Cat::Toggles => {
                if let Some(form) = self.active_form_mut() {
                    if let FieldKind::Toggles {
                        checked, cursor, ..
                    } = &mut form.fields[active].kind
                    {
                        match code {
                            KeyCode::Left | KeyCode::Char('h') => {
                                *cursor = cursor.saturating_sub(1);
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                if *cursor + 1 < checked.len() {
                                    *cursor += 1;
                                }
                            }
                            KeyCode::Char(' ') | KeyCode::Enter => {
                                if let Some(flag) = checked.get_mut(*cursor) {
                                    *flag = !*flag;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                None
            }
            Cat::Action => match code {
                KeyCode::Enter | KeyCode::Char(' ') => match field_key {
                    FieldKey::Save => self.commit_form(),
                    FieldKey::Discover => self
                        .active_form_mut()
                        .map(|f| Effect::Discover(f.provider.clone())),
                    _ => None,
                },
                _ => None,
            },
        }
    }

    /// After the auth selector changes, insert or remove the inline API-key
    /// text field so it appears only for `api_key` auth.
    fn sync_api_key_field(&mut self) {
        let Some(form) = self.active_form_mut() else {
            return;
        };
        let is_api_key =
            form.field(FieldKey::Auth).and_then(FormField::select_value) == Some("api_key");
        let has_field = form.field(FieldKey::ApiKey).is_some();
        if is_api_key && !has_field {
            // Insert right after the Auth field.
            if let Some(auth_idx) = form.fields.iter().position(|f| f.key == FieldKey::Auth) {
                form.fields.insert(
                    auth_idx + 1,
                    FormField {
                        key: FieldKey::ApiKey,
                        label: "api key".to_string(),
                        kind: FieldKind::Text {
                            value: String::new(),
                            cursor: 0,
                        },
                    },
                );
            }
        } else if !is_api_key && has_field {
            form.fields.retain(|f| f.key != FieldKey::ApiKey);
            if form.active >= form.fields.len() {
                form.active = form.fields.len().saturating_sub(1);
            }
        }
    }

    /// Commit the active form: build the node, upsert it, and return to Browse
    /// with a save effect on success; on a validation error, keep the form open
    /// and record the message.
    fn commit_form(&mut self) -> Option<Effect> {
        let (result, old_id) = match &self.mode {
            Mode::AddNode(form) => (form.to_node(), None),
            Mode::EditNode { id, form } => (form.to_node(), Some(id.clone())),
            _ => return None,
        };
        match result {
            Ok((id, node)) => {
                // On an edit that renamed the node id, drop the old entry.
                if let Some(old) = &old_id {
                    if old != &id {
                        let _ = actions::delete_node(&mut self.config, old);
                    }
                }
                actions::upsert_node(&mut self.config, id.clone(), node);
                self.last_saved_node = Some(id.clone());
                self.dirty = true;
                self.mode = Mode::Browse;
                self.select_node(&id);
                self.status_line = Some(format!("saved {id}"));
                Some(Effect::Save)
            }
            Err(e) => {
                if let Some(form) = self.active_form_mut() {
                    form.error = Some(e.to_string());
                }
                None
            }
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

/// Apply a single editing keystroke to a `(value, cursor)` text buffer.
fn edit_text_parts(value: &mut String, cursor: &mut usize, code: KeyCode) {
    match code {
        KeyCode::Char(c) => {
            value.insert(*cursor, c);
            *cursor += c.len_utf8();
        }
        KeyCode::Backspace => {
            if *cursor > 0 {
                let prev = value[..*cursor]
                    .chars()
                    .next_back()
                    .map(char::len_utf8)
                    .unwrap_or(1);
                *cursor -= prev;
                value.remove(*cursor);
            }
        }
        KeyCode::Left if *cursor > 0 => {
            let prev = value[..*cursor]
                .chars()
                .next_back()
                .map(char::len_utf8)
                .unwrap_or(1);
            *cursor -= prev;
        }
        KeyCode::Right if *cursor < value.len() => {
            let next = value[*cursor..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
            *cursor += next;
        }
        _ => {}
    }
}

/// Apply an editing keystroke to an [`Editor`]'s text buffer.
fn edit_text(editor: &mut Editor, code: KeyCode) {
    edit_text_parts(&mut editor.value, &mut editor.cursor, code);
    editor.error = None;
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

    // --- Task 5.4: node lifecycle keys ----------------------------------

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn a_opens_add_node_form() {
        let mut app = App::new(config_with(1), "ailloy");
        assert!(app.handle_key(key(KeyCode::Char('a'))).is_none());
        assert!(matches!(app.mode, Mode::AddNode(_)));
    }

    #[test]
    fn e_opens_edit_form_for_selected_node() {
        let mut app = App::new(config_with(2), "ailloy");
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char('e')));
        match &app.mode {
            Mode::EditNode { id, form } => {
                assert_eq!(id, "openai/node-1");
                assert!(form.editing);
            }
            _ => panic!("expected EditNode mode"),
        }
    }

    #[test]
    fn x_opens_confirm_delete() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('x')));
        match &app.mode {
            Mode::Confirm { action, .. } => {
                assert_eq!(
                    *action,
                    ConfirmAction::DeleteNode {
                        id: "openai/node-0".into()
                    }
                );
            }
            _ => panic!("expected Confirm mode"),
        }
    }

    #[test]
    fn t_returns_run_test_effect() {
        let mut app = App::new(config_with(1), "ailloy");
        assert_eq!(
            app.handle_key(key(KeyCode::Char('t'))),
            Some(Effect::RunTest("openai/node-0".into()))
        );
    }

    #[test]
    fn k_opens_keychain_mode() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('k')));
        match &app.mode {
            Mode::Keychain { node_id, .. } => assert_eq!(node_id, "openai/node-0"),
            _ => panic!("expected Keychain mode"),
        }
    }

    #[test]
    fn d_opens_set_default_for_with_node_capabilities() {
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        app.handle_key(key(KeyCode::Char('d')));
        match &app.mode {
            Mode::SetDefaultFor { node_id, caps, .. } => {
                assert_eq!(node_id, "openai/node-0");
                assert_eq!(caps, &vec![Capability::Chat, Capability::Image]);
            }
            _ => panic!("expected SetDefaultFor mode"),
        }
    }

    #[test]
    fn confirm_delete_removes_node_and_saves() {
        let mut app = App::new(config_with(2), "ailloy");
        app.handle_key(key(KeyCode::Char('x')));
        let effect = app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(effect, Some(Effect::Save));
        assert!(matches!(app.mode, Mode::Browse));
        assert!(app.dirty);
        assert!(!app.config.nodes.contains_key("openai/node-0"));
        assert_eq!(app.config.nodes.len(), 1);
    }

    #[test]
    fn confirm_delete_declined_keeps_node() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('x')));
        let effect = app.handle_key(key(KeyCode::Char('n')));
        assert!(effect.is_none());
        assert!(matches!(app.mode, Mode::Browse));
        assert!(app.config.nodes.contains_key("openai/node-0"));
    }

    #[test]
    fn set_default_for_commit_sets_default_and_star() {
        use crate::tui::ui::capability_cell;
        let mut app = App::new(config_with_capable_nodes(), "ailloy");
        app.handle_key(key(KeyCode::Char('d')));
        // caps = [Chat, Image]; move to Image then commit.
        app.handle_key(key(KeyCode::Down));
        let effect = app.handle_key(key(KeyCode::Enter));
        assert_eq!(effect, Some(Effect::Save));
        assert_eq!(
            app.config.defaults.get("image").map(String::as_str),
            Some("openai/node-0")
        );
        let node = &app.config.nodes["openai/node-0"];
        assert_eq!(
            capability_cell(&app.config, "openai/node-0", node, &Capability::Image),
            "★"
        );
    }

    #[test]
    fn keychain_commit_returns_store_effect() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('k')));
        for c in ['s', 'k', '-', '1'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let effect = app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            effect,
            Some(Effect::StoreKeychain {
                node_id: "openai/node-0".into(),
                secret: "sk-1".into(),
            })
        );
        assert!(matches!(app.mode, Mode::Browse));
    }

    #[test]
    fn keychain_empty_secret_is_rejected() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('k')));
        let effect = app.handle_key(key(KeyCode::Enter));
        assert!(effect.is_none());
        assert!(matches!(app.mode, Mode::Keychain { .. }));
    }

    #[test]
    fn add_form_provider_select_cycles_provider() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('a')));
        // Active field is the provider select; Right cycles to Anthropic.
        app.handle_key(key(KeyCode::Right));
        match &app.mode {
            Mode::AddNode(form) => assert_eq!(form.provider, ProviderKind::Anthropic),
            _ => panic!("expected AddNode mode"),
        }
    }

    #[test]
    fn add_form_ctrl_s_with_empty_model_sets_error() {
        let mut app = App::new(config_with(1), "ailloy");
        app.handle_key(key(KeyCode::Char('a')));
        let effect = app.handle_key(ctrl(KeyCode::Char('s')));
        assert!(effect.is_none());
        match &app.mode {
            Mode::AddNode(form) => assert!(form.error.is_some()),
            _ => panic!("expected AddNode mode to persist with an error"),
        }
    }

    #[test]
    fn add_form_commit_upserts_node_and_saves() {
        let mut app = App::new(Config::default(), "ailloy");
        app.handle_key(key(KeyCode::Char('a')));
        // Fill in the model field directly, then commit with Ctrl+S.
        if let Mode::AddNode(form) = &mut app.mode {
            let f = form
                .fields
                .iter_mut()
                .find(|f| f.key == FieldKey::Model)
                .unwrap();
            f.kind = FieldKind::Text {
                value: "gpt-5.4-mini".into(),
                cursor: 0,
            };
        }
        let effect = app.handle_key(ctrl(KeyCode::Char('s')));
        assert_eq!(effect, Some(Effect::Save));
        assert!(app.config.nodes.contains_key("openai/gpt-5.4-mini"));
        assert_eq!(app.last_saved_node.as_deref(), Some("openai/gpt-5.4-mini"));
        // Auto-set as the chat default.
        assert_eq!(
            app.config.defaults.get("chat").map(String::as_str),
            Some("openai/gpt-5.4-mini")
        );
    }

    #[test]
    fn test_mode_any_key_dismisses() {
        let mut app = App::new(config_with(1), "ailloy");
        app.mode = Mode::Test {
            node_id: "openai/node-0".into(),
            result: Some("OK".into()),
        };
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::Browse));
    }
}
