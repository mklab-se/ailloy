//! AI node discovery — detect available providers and models.
//!
//! These functions return discovered nodes as data only — no I/O prompting,
//! no config mutation. The CLI layer handles user interaction.

use anyhow::Result;

use crate::config::{AiNode, Auth, Capability, ProviderKind};

/// A discovered AI node with metadata.
#[derive(Debug, Clone)]
pub struct DiscoveredNode {
    /// Suggested node ID (e.g. `openai/gpt-4o`).
    pub suggested_id: String,
    /// The node configuration.
    pub node: AiNode,
    /// Human-readable description of the discovery source.
    pub description: String,
}

/// Check for environment variable-based API keys and return discovered nodes.
pub fn discover_env_keys() -> Vec<DiscoveredNode> {
    let mut results = Vec::new();

    if std::env::var("OPENAI_API_KEY").is_ok() {
        results.push(DiscoveredNode {
            suggested_id: "openai/gpt-4o".to_string(),
            node: AiNode {
                provider: ProviderKind::OpenAi,
                alias: None,
                capabilities: vec![Capability::Chat, Capability::Image],
                auth: Some(Auth::Env("OPENAI_API_KEY".to_string())),
                model: Some("gpt-4o".to_string()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            },
            description: "OPENAI_API_KEY is set".to_string(),
        });
    }

    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        results.push(DiscoveredNode {
            suggested_id: "anthropic/claude-sonnet-4-6".to_string(),
            node: AiNode {
                provider: ProviderKind::Anthropic,
                alias: None,
                capabilities: vec![Capability::Chat],
                auth: Some(Auth::Env("ANTHROPIC_API_KEY".to_string())),
                model: Some("claude-sonnet-4-6".to_string()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            },
            description: "ANTHROPIC_API_KEY is set".to_string(),
        });
    }

    results
}

/// Discover local CLI agents (claude, codex, copilot) by checking PATH.
pub async fn discover_local() -> Result<Vec<DiscoveredNode>> {
    let mut results = Vec::new();

    for binary in &["claude", "codex", "copilot"] {
        let check = tokio::process::Command::new("which")
            .arg(binary)
            .output()
            .await;
        if let Ok(output) = check {
            if output.status.success() {
                results.push(DiscoveredNode {
                    suggested_id: format!("local-agent/{}", binary),
                    node: AiNode {
                        provider: ProviderKind::LocalAgent,
                        alias: None,
                        capabilities: vec![Capability::Chat],
                        auth: None,
                        model: None,
                        endpoint: None,
                        deployment: None,
                        api_version: None,
                        binary: Some(binary.to_string()),
                        project: None,
                        location: None,
                        node_defaults: None,
                    },
                    description: format!("{} found in PATH", binary),
                });
            }
        }
    }

    Ok(results)
}

/// Discover Ollama models by querying the API.
pub async fn discover_ollama(endpoint: Option<&str>) -> Result<Vec<DiscoveredNode>> {
    let base = endpoint.unwrap_or("http://localhost:11434");
    let url = format!("{}/api/tags", base);

    let resp = tokio::time::timeout(std::time::Duration::from_secs(3), reqwest::get(&url))
        .await
        .map_err(|_| anyhow::anyhow!("Ollama connection timed out"))??;

    if !resp.status().is_success() {
        anyhow::bail!("Ollama returned status {}", resp.status());
    }

    let body: serde_json::Value = resp.json().await?;
    let mut results = Vec::new();

    if let Some(models) = body.get("models").and_then(|m| m.as_array()) {
        for model in models {
            if let Some(name) = model.get("name").and_then(|n| n.as_str()) {
                // Strip :latest suffix for cleaner IDs
                let clean_name = name.strip_suffix(":latest").unwrap_or(name);
                let custom_endpoint = if base != "http://localhost:11434" {
                    Some(base.to_string())
                } else {
                    None
                };
                results.push(DiscoveredNode {
                    suggested_id: format!("ollama/{}", clean_name),
                    node: AiNode {
                        provider: ProviderKind::Ollama,
                        alias: None,
                        capabilities: vec![Capability::Chat],
                        auth: None,
                        model: Some(clean_name.to_string()),
                        endpoint: custom_endpoint,
                        deployment: None,
                        api_version: None,
                        binary: None,
                        project: None,
                        location: None,
                        node_defaults: None,
                    },
                    description: format!("Ollama model: {}", name),
                });
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_env_keys_no_keys() {
        // Remove env vars to ensure they're not set
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
        let results = discover_env_keys();
        assert!(results.is_empty());
    }

    #[test]
    fn test_discovered_node_structure() {
        let node = DiscoveredNode {
            suggested_id: "openai/gpt-4o".to_string(),
            node: AiNode {
                provider: ProviderKind::OpenAi,
                alias: None,
                capabilities: vec![Capability::Chat],
                auth: Some(Auth::Env("OPENAI_API_KEY".to_string())),
                model: Some("gpt-4o".to_string()),
                endpoint: None,
                deployment: None,
                api_version: None,
                binary: None,
                project: None,
                location: None,
                node_defaults: None,
            },
            description: "test".to_string(),
        };
        assert_eq!(node.suggested_id, "openai/gpt-4o");
        assert_eq!(node.node.provider, ProviderKind::OpenAi);
    }
}
