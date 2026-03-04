use anyhow::Result;

use ailloy::config::Config;

/// Result of a consent prompt.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsentResult {
    /// User agreed and wants the choice remembered.
    AllowAndRemember,
    /// User agreed for this session only.
    AllowOnce,
    /// User declined.
    Denied,
}

/// Check whether the user has already consented (or declined) for the given key.
pub fn check_consent(config: &Config, key: &str) -> Option<bool> {
    config.consents.get(key).copied()
}

/// Prompt the user for consent to use an external CLI tool.
pub fn prompt_consent(tool_name: &str, description: &str) -> Result<ConsentResult> {
    let message = format!("Allow ailloy to use {} to {}?", tool_name, description,);

    let options = vec![
        "Yes, and remember my choice",
        "Yes, just this once",
        "No, I'll configure manually",
    ];

    let selected = match inquire::Select::new(&message, options).prompt() {
        Ok(s) => s,
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => return Ok(ConsentResult::Denied),
        Err(e) => return Err(e.into()),
    };

    Ok(match selected {
        "Yes, and remember my choice" => ConsentResult::AllowAndRemember,
        "Yes, just this once" => ConsentResult::AllowOnce,
        _ => ConsentResult::Denied,
    })
}

/// Ensure consent for an external tool: check existing decision, prompt if needed.
///
/// Returns `true` if the tool may be used, `false` if denied.
/// If `AllowAndRemember`, inserts `true` into `config.consents` (persisted on next `config.save()`).
pub fn ensure_consent(
    config: &mut Config,
    key: &str,
    tool_name: &str,
    description: &str,
) -> Result<bool> {
    if let Some(allowed) = check_consent(config, key) {
        return Ok(allowed);
    }

    match prompt_consent(tool_name, description)? {
        ConsentResult::AllowAndRemember => {
            config.consents.insert(key.to_string(), true);
            Ok(true)
        }
        ConsentResult::AllowOnce => Ok(true),
        ConsentResult::Denied => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_consent_none() {
        let config = Config::default();
        assert_eq!(check_consent(&config, "azure-cli"), None);
    }

    #[test]
    fn test_check_consent_allowed() {
        let mut config = Config::default();
        config.consents.insert("azure-cli".to_string(), true);
        assert_eq!(check_consent(&config, "azure-cli"), Some(true));
    }

    #[test]
    fn test_check_consent_denied() {
        let mut config = Config::default();
        config.consents.insert("azure-cli".to_string(), false);
        assert_eq!(check_consent(&config, "azure-cli"), Some(false));
    }
}
