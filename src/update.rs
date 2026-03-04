use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tracing::debug;

const CACHE_DURATION: Duration = Duration::from_secs(24 * 60 * 60);
const CRATE_NAME: &str = "ailloy";

/// Returns true if the binary appears to be running from a Cargo build directory.
pub fn is_running_from_source() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.contains("/target/")))
        .unwrap_or(false)
}

/// Returns the appropriate upgrade command based on how ailloy was installed.
pub fn upgrade_hint() -> String {
    if let Ok(exe) = std::env::current_exe() {
        let path = exe.to_string_lossy();
        if path.contains("/Cellar/") || path.contains("/homebrew/") {
            return "brew upgrade ailloy".to_string();
        }
    }
    "cargo install ailloy".to_string()
}

fn cache_path() -> Option<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else {
        dirs::home_dir()?.join(".cache")
    };
    Some(base.join("ailloy").join("latest_version"))
}

pub async fn check_for_update() -> Option<String> {
    if std::env::var("AILLOY_NO_UPDATE_CHECK").is_ok() {
        return None;
    }

    // Check cache first
    if let Some(path) = cache_path() {
        if let Ok(metadata) = std::fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                if SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or(CACHE_DURATION)
                    < CACHE_DURATION
                {
                    if let Ok(version) = std::fs::read_to_string(&path) {
                        return Some(version.trim().to_string());
                    }
                }
            }
        }
    }

    // Query crates.io
    let url = format!("https://crates.io/api/v1/crates/{}", CRATE_NAME);
    let client = reqwest::Client::builder()
        .user_agent(format!("{}/{}", CRATE_NAME, env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;

    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        debug!("Update check failed: HTTP {}", response.status());
        return None;
    }

    #[derive(serde::Deserialize)]
    struct CrateInfo {
        #[serde(rename = "crate")]
        krate: CrateData,
    }

    #[derive(serde::Deserialize)]
    struct CrateData {
        max_version: String,
    }

    let info: CrateInfo = response.json().await.ok()?;
    let latest = info.krate.max_version;

    // Update cache
    if let Some(path) = cache_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &latest);
    }

    Some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_running_from_source() {
        // When running tests, the binary is in target/debug/deps/ — so this should be true
        assert!(is_running_from_source());
    }

    #[test]
    fn test_upgrade_hint_defaults_to_cargo() {
        // In a test/dev environment, exe is in target/ — not Cellar or homebrew
        let hint = upgrade_hint();
        assert_eq!(hint, "cargo install ailloy");
    }
}
