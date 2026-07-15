//! Programmatic configuration — how a dependent tool (rigg, cosq, mdeck,
//! pidge, ...) offers "set up AI" without shelling out to the ailloy CLI.
//!
//! Run: cargo run --example configure

use std::collections::BTreeMap;

use ailloy::config::{AiNode, Auth, Capability, Config, ProviderKind};

fn main() -> anyhow::Result<()> {
    let mut config = Config::load_global().unwrap_or_default();

    // Describe the node your tool wants available.
    let mut node = AiNode::new(ProviderKind::OpenAi);
    node.model = Some("gpt-5.4-mini".into());
    node.auth = Some(Auth::Env("OPENAI_API_KEY".into())); // or Auth::Keychain(true)
    node.capabilities = vec![Capability::Chat];

    // ensure_node never overwrites what the user already configured.
    if config.ensure_node("openai/gpt-5.4-mini".into(), node) {
        // Only claim the default when the capability has none yet.
        if !config.defaults.contains_key("chat") {
            config.set_default_for("chat", "openai/gpt-5.4-mini")?;
        }
        config.save()?;
        println!("node added (stored in ~/.config/ailloy/config.yaml)");
    } else {
        println!("node already present — left untouched");
    }

    // Per-node default parameters (`node_defaults` in Rust, `defaults:` under
    // the node in YAML): a tool that always wants high-quality, longer-than-
    // default video clips from a given node can set them once here instead of
    // asking every caller to pass explicit options. Recognized keys and their
    // value shapes live in `ailloy::params`; resolution order at request time
    // is explicit options > node defaults > provider defaults.
    let mut media_node = AiNode::new(ProviderKind::AzureOpenAi);
    media_node.model = Some("gpt-image-2".into());
    media_node.endpoint = Some("https://my-resource.openai.azure.com".into());
    media_node.auth = Some(Auth::Keychain(true));
    media_node.capabilities = vec![Capability::Image, Capability::Video];
    media_node.node_defaults = Some(BTreeMap::from([
        ("image.quality".to_string(), "high".to_string()),
        ("video.seconds".to_string(), "8".to_string()),
    ]));

    if config.ensure_node("azure-openai/gpt-image-2".into(), media_node) {
        if !config.defaults.contains_key("image") {
            config.set_default_for("image", "azure-openai/gpt-image-2")?;
        }
        config.save()?;
        println!("media node added with node_defaults (image.quality=high, video.seconds=8)");
    } else {
        println!("media node already present — left untouched");
    }

    // Secrets belong in the OS keychain, never in config files:
    // ailloy::config::set_keychain_secret("openai/gpt-5.4-mini", "sk-...")?;
    Ok(())
}
