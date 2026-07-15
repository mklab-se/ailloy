//! Config-mutation helpers invoked by the TUI reducer/event loop.
//!
//! These operate directly on a [`Config`] and are kept free of terminal
//! concerns so they can be unit-tested in isolation. The UI wiring that calls
//! them lands in Task 5.3.

use anyhow::{Result, anyhow};

use crate::config::Config;
use crate::params;

/// Set or clear a per-node default parameter.
///
/// - `Some(value)` validates `value` against the parameter definition for
///   `key` (via [`params::lookup`] + [`params::validate_value`]) and inserts it
///   into the node's `node_defaults` map (creating the map if absent).
/// - `None` removes `key` from the node's defaults, dropping the map entirely
///   when it becomes empty so the serialized YAML stays clean.
///
/// Errors with an actionable message when the node is unknown, the key is not a
/// recognized parameter, or the value fails validation.
// Wired to the parameter-editing UI in Task 5.3.
#[allow(dead_code)]
pub fn set_node_default(
    config: &mut Config,
    node_id: &str,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    let node = config
        .nodes
        .get_mut(node_id)
        .ok_or_else(|| anyhow!("no such node '{node_id}'"))?;

    match value {
        Some(value) => {
            let def = params::lookup(key)
                .ok_or_else(|| anyhow!("unknown parameter '{key}'; not a recognized default"))?;
            params::validate_value(def, value).map_err(|e| anyhow!(e))?;
            node.node_defaults
                .get_or_insert_with(Default::default)
                .insert(key.to_string(), value.to_string());
        }
        None => {
            if let Some(defaults) = node.node_defaults.as_mut() {
                defaults.remove(key);
                if defaults.is_empty() {
                    node.node_defaults = None;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiNode, ProviderKind};

    fn config_with_node() -> Config {
        let mut config = Config::default();
        config
            .nodes
            .insert("openai/img".to_string(), AiNode::new(ProviderKind::OpenAi));
        config
    }

    #[test]
    fn sets_valid_value_and_creates_map() {
        let mut config = config_with_node();
        set_node_default(&mut config, "openai/img", "image.quality", Some("high")).unwrap();
        let node = &config.nodes["openai/img"];
        assert_eq!(node.default_for("image.quality"), Some("high"));
    }

    #[test]
    fn rejects_invalid_value_actionably() {
        let mut config = config_with_node();
        let err = set_node_default(&mut config, "openai/img", "image.quality", Some("ultra"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("image.quality"), "message: {err}");
        assert!(err.contains("high"), "should list valid options: {err}");
        // Nothing was written.
        assert!(config.nodes["openai/img"].node_defaults.is_none());
    }

    #[test]
    fn rejects_unknown_key() {
        let mut config = config_with_node();
        let err = set_node_default(&mut config, "openai/img", "bogus.key", Some("x"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("bogus.key"), "message: {err}");
    }

    #[test]
    fn rejects_unknown_node() {
        let mut config = config_with_node();
        let err = set_node_default(&mut config, "nope", "image.quality", Some("high"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("nope"), "message: {err}");
    }

    #[test]
    fn none_removes_key_and_drops_empty_map() {
        let mut config = config_with_node();
        set_node_default(&mut config, "openai/img", "image.quality", Some("high")).unwrap();
        set_node_default(&mut config, "openai/img", "image.quality", None).unwrap();
        // Map dropped entirely once empty.
        assert!(config.nodes["openai/img"].node_defaults.is_none());
    }

    #[test]
    fn none_keeps_map_with_remaining_keys() {
        let mut config = config_with_node();
        set_node_default(&mut config, "openai/img", "image.quality", Some("high")).unwrap();
        set_node_default(&mut config, "openai/img", "image.variants", Some("2")).unwrap();
        set_node_default(&mut config, "openai/img", "image.quality", None).unwrap();
        let node = &config.nodes["openai/img"];
        assert_eq!(node.default_for("image.variants"), Some("2"));
        assert_eq!(node.default_for("image.quality"), None);
    }
}
