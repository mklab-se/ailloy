//! Input-widget state used by the config TUI's editing modes.
//!
//! These are deliberately minimal placeholders in this task — enough for the
//! [`super::app::Mode`] variants to compile and hold state. Tasks 5.3/5.4 flesh
//! out the rendering and input handling.

/// A single-value editor: either free-text with a cursor, or a selection from a
/// fixed list of choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    /// The current text value (also the resolved value for a selection).
    pub value: String,
    /// Cursor position within `value`, as a byte offset for text editing.
    pub cursor: usize,
    /// A validation error to surface next to the field, if any.
    pub error: Option<String>,
    /// When present, the editor is a picker: `(choices, selected_index)`.
    pub choices: Option<(Vec<String>, usize)>,
}

// Constructors are consumed by the edit flows in Tasks 5.3/5.4.
#[allow(dead_code)]
impl Editor {
    /// A free-text editor seeded with `initial`, cursor at the end.
    pub fn text(initial: impl Into<String>) -> Self {
        let value = initial.into();
        let cursor = value.len();
        Editor {
            value,
            cursor,
            error: None,
            choices: None,
        }
    }

    /// A selection editor over `choices`, starting at `current` (clamped).
    pub fn select(choices: Vec<String>, current: usize) -> Self {
        let current = current.min(choices.len().saturating_sub(1));
        let value = choices.get(current).cloned().unwrap_or_default();
        Editor {
            value,
            cursor: 0,
            error: None,
            choices: Some((choices, current)),
        }
    }
}

/// A single labeled field within a [`NodeForm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    /// The field's display label.
    pub label: String,
    /// The field's current value.
    pub value: String,
}

/// A multi-field form for adding or editing a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeForm {
    /// The fields, in display order.
    pub fields: Vec<FormField>,
    /// Index of the active field.
    pub active: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_editor_places_cursor_at_end() {
        let ed = Editor::text("hello");
        assert_eq!(ed.value, "hello");
        assert_eq!(ed.cursor, 5);
        assert!(ed.error.is_none());
        assert!(ed.choices.is_none());
    }

    #[test]
    fn select_editor_seeds_value_from_choice() {
        let ed = Editor::select(vec!["a".into(), "b".into(), "c".into()], 1);
        assert_eq!(ed.value, "b");
        assert_eq!(ed.choices.unwrap().1, 1);
    }

    #[test]
    fn select_editor_clamps_out_of_range_index() {
        let ed = Editor::select(vec!["a".into(), "b".into()], 9);
        assert_eq!(ed.value, "b");
        assert_eq!(ed.choices.unwrap().1, 1);
    }
}
