//! Config-mutation helpers invoked by the TUI reducer/event loop.
//!
//! These operate directly on a [`Config`] and are kept free of terminal
//! concerns so they can be unit-tested in isolation. The UI wiring that calls
//! them lands in Task 5.3.

use anyhow::{Result, anyhow};

use crate::config::{AiNode, Capability, Config};
use crate::params;

/// Insert or replace the node with `id`, auto-assigning it as the default for
/// any of its capabilities that have no default yet.
///
/// Returns the list of capability keys the node became the default for, so the
/// caller can surface it in a status line.
pub fn upsert_node(config: &mut Config, id: String, node: AiNode) -> Vec<String> {
    let caps = node.capabilities.clone();
    config.upsert_node(id.clone(), node);
    let mut became_default = Vec::new();
    for cap in &caps {
        let key = cap.config_key();
        if !config.defaults.contains_key(key) {
            config.defaults.insert(key.to_string(), id.clone());
            became_default.push(key.to_string());
        }
    }
    became_default
}

/// Delete the node with `id` and drop any capability defaults pointing at it.
///
/// Errors actionably when the node does not exist.
pub fn delete_node(config: &mut Config, id: &str) -> Result<()> {
    if !config.nodes.contains_key(id) {
        return Err(anyhow!("no such node '{id}'"));
    }
    config.nodes.remove(id);
    config.defaults.retain(|_, v| v != id);
    Ok(())
}

/// Set the default node for a capability key, validating that the node has it.
///
/// Errors actionably when the node is unknown, the capability key is not
/// recognized, or the node lacks that capability.
pub fn set_capability_default(config: &mut Config, cap_key: &str, node_id: &str) -> Result<()> {
    let cap: Capability = cap_key.parse().map_err(|e: String| anyhow!("{e}"))?;
    let node = config
        .nodes
        .get(node_id)
        .ok_or_else(|| anyhow!("no such node '{node_id}'"))?;
    if !node.has_capability(&cap) {
        return Err(anyhow!(
            "node '{node_id}' does not have the '{cap_key}' capability"
        ));
    }
    config
        .defaults
        .insert(cap_key.to_string(), node_id.to_string());
    Ok(())
}

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

    // --- Task 5.4: node add/delete + capability defaults ----------------

    use crate::config::Capability;

    fn chat_node() -> AiNode {
        let mut node = AiNode::new(ProviderKind::OpenAi);
        node.model = Some("gpt-5.4".into());
        node.capabilities = vec![Capability::Chat];
        node
    }

    #[test]
    fn upsert_node_inserts_and_auto_defaults() {
        let mut config = Config::default();
        let became = upsert_node(&mut config, "openai/gpt-5.4".into(), chat_node());
        assert!(config.nodes.contains_key("openai/gpt-5.4"));
        assert_eq!(became, vec!["chat".to_string()]);
        assert_eq!(
            config.defaults.get("chat").map(String::as_str),
            Some("openai/gpt-5.4")
        );
    }

    #[test]
    fn upsert_node_does_not_override_existing_default() {
        let mut config = Config::default();
        config.defaults.insert("chat".into(), "other/node".into());
        let became = upsert_node(&mut config, "openai/gpt-5.4".into(), chat_node());
        assert!(became.is_empty(), "existing default preserved");
        assert_eq!(
            config.defaults.get("chat").map(String::as_str),
            Some("other/node")
        );
    }

    #[test]
    fn delete_node_clears_defaults_pointing_at_it() {
        let mut config = Config::default();
        upsert_node(&mut config, "openai/gpt-5.4".into(), chat_node());
        assert!(config.defaults.contains_key("chat"));
        delete_node(&mut config, "openai/gpt-5.4").unwrap();
        assert!(!config.nodes.contains_key("openai/gpt-5.4"));
        assert!(!config.defaults.contains_key("chat"), "default cleaned up");
    }

    #[test]
    fn delete_node_unknown_errors() {
        let mut config = Config::default();
        let err = delete_node(&mut config, "nope").unwrap_err().to_string();
        assert!(err.contains("nope"), "message: {err}");
    }

    #[test]
    fn set_capability_default_validates_capability_presence() {
        let mut config = Config::default();
        config.nodes.insert("openai/gpt-5.4".into(), chat_node());
        // Node has chat but not image.
        set_capability_default(&mut config, "chat", "openai/gpt-5.4").unwrap();
        assert_eq!(
            config.defaults.get("chat").map(String::as_str),
            Some("openai/gpt-5.4")
        );

        let err = set_capability_default(&mut config, "image", "openai/gpt-5.4")
            .unwrap_err()
            .to_string();
        assert!(err.contains("image"), "message: {err}");
    }

    #[test]
    fn set_capability_default_unknown_node_errors() {
        let mut config = Config::default();
        let err = set_capability_default(&mut config, "chat", "nope")
            .unwrap_err()
            .to_string();
        assert!(err.contains("nope"), "message: {err}");
    }
}
