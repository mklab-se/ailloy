//! Programmatic configuration — how a dependent tool (rigg, cosq, mdeck,
//! pidge, ...) offers "set up AI" without shelling out to the ailloy CLI.
//!
//! Run: cargo run --example configure

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

    // Secrets belong in the OS keychain, never in config files:
    // ailloy::config::set_keychain_secret("openai/gpt-5.4-mini", "sk-...")?;
    Ok(())
}
