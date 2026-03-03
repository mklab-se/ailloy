use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tracing::debug;

const CACHE_DURATION: Duration = Duration::from_secs(24 * 60 * 60);
const CRATE_NAME: &str = "ailloy";

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("ailloy").join("latest_version"))
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
