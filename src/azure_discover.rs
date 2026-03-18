//! Azure CLI wrappers for discovering subscriptions, resources, and deployments.
//!
//! Gated behind the `config-tui` feature. Wraps the `az` CLI to discover
//! Azure OpenAI and Microsoft Foundry resources.

use anyhow::{Context, Result};
use serde::Deserialize;

/// An Azure subscription from `az account list`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AzSubscription {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

impl std::fmt::Display for AzSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let marker = if self.is_default { " (default)" } else { "" };
        write!(f, "{}{}", self.name, marker)
    }
}

/// An Azure Cognitive Services account from `az cognitiveservices account list`.
#[derive(Debug, Clone, Deserialize)]
pub struct AzCognitiveAccount {
    pub name: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    #[allow(dead_code)] // Deserialized from JSON; used in tests.
    pub kind: String,
    pub properties: Option<AzCognitiveProperties>,
    /// Resource group extracted from the resource ID.
    #[serde(rename = "resourceGroup")]
    pub resource_group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzCognitiveProperties {
    pub endpoint: Option<String>,
}

impl std::fmt::Display for AzCognitiveAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.location)
    }
}

impl AzCognitiveAccount {
    pub fn endpoint(&self) -> Option<&str> {
        self.properties.as_ref()?.endpoint.as_deref()
    }
}

/// A deployment on an Azure Cognitive Services account.
#[derive(Debug, Clone, Deserialize)]
pub struct AzDeployment {
    pub name: String,
    pub properties: Option<AzDeploymentProperties>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzDeploymentProperties {
    pub model: Option<AzDeploymentModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AzDeploymentModel {
    pub name: Option<String>,
    #[allow(dead_code)] // Deserialized from JSON; used in tests.
    pub version: Option<String>,
}

impl std::fmt::Display for AzDeployment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let model_info = self
            .properties
            .as_ref()
            .and_then(|p| p.model.as_ref())
            .and_then(|m| m.name.as_deref())
            .unwrap_or("unknown model");
        write!(f, "{} [{}]", self.name, model_info)
    }
}

/// List enabled Azure subscriptions.
pub async fn list_subscriptions() -> Result<Vec<AzSubscription>> {
    let output = tokio::process::Command::new("az")
        .args([
            "account",
            "list",
            "--query",
            "[?state=='Enabled']",
            "-o",
            "json",
        ])
        .output()
        .await
        .context("Failed to run 'az account list'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("az account list failed: {}", stderr.trim());
    }

    let subs: Vec<AzSubscription> =
        serde_json::from_slice(&output.stdout).context("Failed to parse subscription list")?;
    Ok(subs)
}

/// Set the active Azure subscription.
pub async fn set_subscription(subscription_id: &str) -> Result<()> {
    let output = tokio::process::Command::new("az")
        .args(["account", "set", "-s", subscription_id])
        .output()
        .await
        .context("Failed to run 'az account set'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("az account set failed: {}", stderr.trim());
    }
    Ok(())
}

/// List Azure OpenAI (Cognitive Services) resources in the current subscription.
pub async fn list_openai_resources() -> Result<Vec<AzCognitiveAccount>> {
    let output = tokio::process::Command::new("az")
        .args([
            "cognitiveservices",
            "account",
            "list",
            "--query",
            "[?kind=='OpenAI']",
            "-o",
            "json",
        ])
        .output()
        .await
        .context("Failed to run 'az cognitiveservices account list'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "az cognitiveservices account list failed: {}",
            stderr.trim()
        );
    }

    let resources: Vec<AzCognitiveAccount> =
        serde_json::from_slice(&output.stdout).context("Failed to parse resource list")?;
    Ok(resources)
}

/// List Microsoft Foundry (AI Services) resources in the current subscription.
pub async fn list_foundry_resources() -> Result<Vec<AzCognitiveAccount>> {
    let output = tokio::process::Command::new("az")
        .args([
            "cognitiveservices",
            "account",
            "list",
            "--query",
            "[?kind=='AIServices']",
            "-o",
            "json",
        ])
        .output()
        .await
        .context("Failed to run 'az cognitiveservices account list'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "az cognitiveservices account list failed: {}",
            stderr.trim()
        );
    }

    let resources: Vec<AzCognitiveAccount> =
        serde_json::from_slice(&output.stdout).context("Failed to parse resource list")?;
    Ok(resources)
}

/// List deployments on an Azure Cognitive Services (OpenAI) resource.
pub async fn list_deployments(
    resource_group: &str,
    account_name: &str,
) -> Result<Vec<AzDeployment>> {
    let output = tokio::process::Command::new("az")
        .args([
            "cognitiveservices",
            "account",
            "deployment",
            "list",
            "-g",
            resource_group,
            "-n",
            account_name,
            "-o",
            "json",
        ])
        .output()
        .await
        .context("Failed to run 'az cognitiveservices account deployment list'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("az deployment list failed: {}", stderr.trim());
    }

    let deployments: Vec<AzDeployment> =
        serde_json::from_slice(&output.stdout).context("Failed to parse deployment list")?;
    Ok(deployments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_subscriptions() {
        let json = r#"[
            {
                "id": "00000000-0000-0000-0000-000000000001",
                "name": "My Subscription",
                "isDefault": true,
                "state": "Enabled"
            },
            {
                "id": "00000000-0000-0000-0000-000000000002",
                "name": "Dev Subscription",
                "isDefault": false,
                "state": "Enabled"
            }
        ]"#;

        let subs: Vec<AzSubscription> = serde_json::from_str(json).unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].name, "My Subscription");
        assert!(subs[0].is_default);
        assert_eq!(subs[1].id, "00000000-0000-0000-0000-000000000002");
        assert!(!subs[1].is_default);
    }

    #[test]
    fn test_parse_cognitive_accounts() {
        let json = r#"[
            {
                "name": "my-openai",
                "location": "eastus",
                "kind": "OpenAI",
                "resourceGroup": "my-rg",
                "properties": {
                    "endpoint": "https://my-openai.openai.azure.com/"
                }
            }
        ]"#;

        let accounts: Vec<AzCognitiveAccount> = serde_json::from_str(json).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "my-openai");
        assert_eq!(accounts[0].resource_group.as_deref(), Some("my-rg"));
        assert_eq!(
            accounts[0].endpoint(),
            Some("https://my-openai.openai.azure.com/")
        );
    }

    #[test]
    fn test_parse_deployments() {
        let json = r#"[
            {
                "name": "gpt-4o",
                "properties": {
                    "model": {
                        "name": "gpt-4o",
                        "version": "2024-11-20"
                    }
                }
            },
            {
                "name": "text-embedding",
                "properties": {
                    "model": {
                        "name": "text-embedding-ada-002",
                        "version": "2"
                    }
                }
            }
        ]"#;

        let deployments: Vec<AzDeployment> = serde_json::from_str(json).unwrap();
        assert_eq!(deployments.len(), 2);
        assert_eq!(deployments[0].name, "gpt-4o");
        assert_eq!(
            deployments[0]
                .properties
                .as_ref()
                .unwrap()
                .model
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("gpt-4o")
        );
    }

    #[test]
    fn test_subscription_display() {
        let sub = AzSubscription {
            id: "abc".to_string(),
            name: "My Sub".to_string(),
            is_default: true,
        };
        assert_eq!(format!("{sub}"), "My Sub (default)");

        let sub2 = AzSubscription {
            id: "def".to_string(),
            name: "Other".to_string(),
            is_default: false,
        };
        assert_eq!(format!("{sub2}"), "Other");
    }

    #[test]
    fn test_deployment_display() {
        let dep = AzDeployment {
            name: "gpt-4o".to_string(),
            properties: Some(AzDeploymentProperties {
                model: Some(AzDeploymentModel {
                    name: Some("gpt-4o".to_string()),
                    version: Some("2024-11-20".to_string()),
                }),
            }),
        };
        assert_eq!(format!("{dep}"), "gpt-4o [gpt-4o]");
    }

    #[test]
    fn test_deployment_display_no_model() {
        let dep = AzDeployment {
            name: "my-deploy".to_string(),
            properties: None,
        };
        assert_eq!(format!("{dep}"), "my-deploy [unknown model]");
    }

    #[test]
    fn test_parse_foundry_accounts() {
        let json = r#"[
            {
                "name": "my-foundry",
                "location": "swedencentral",
                "kind": "AIServices",
                "resourceGroup": "my-rg",
                "properties": {
                    "endpoint": "https://my-foundry.services.ai.azure.com/"
                }
            }
        ]"#;

        let accounts: Vec<AzCognitiveAccount> = serde_json::from_str(json).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "my-foundry");
        assert_eq!(accounts[0].kind, "AIServices");
        assert_eq!(accounts[0].resource_group.as_deref(), Some("my-rg"));
        assert_eq!(
            accounts[0].endpoint(),
            Some("https://my-foundry.services.ai.azure.com/")
        );
    }

    #[test]
    fn test_cognitive_account_no_endpoint() {
        let acc = AzCognitiveAccount {
            name: "test".to_string(),
            location: "westus".to_string(),
            kind: "OpenAI".to_string(),
            properties: None,
            resource_group: None,
        };
        assert_eq!(acc.endpoint(), None);
    }
}
