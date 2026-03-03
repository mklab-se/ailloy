//! Local CLI agent integration (Claude, Codex, Copilot).
//!
//! Invokes AI tools installed on the system as subprocesses.

use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::Command;
use tracing::debug;

use crate::types::{ChatResponse, Message};

/// Client that delegates to a locally installed AI CLI tool.
pub struct LocalAgentClient {
    binary: String,
}

impl LocalAgentClient {
    /// Create a new local agent client.
    ///
    /// The `binary` should be the name of an executable in PATH (e.g. `claude`, `codex`, `copilot`).
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Send a chat message via the local agent's `--print` flag.
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let prompt = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        debug!(binary = %self.binary, "Sending prompt to local agent");

        let output = Command::new(&self.binary)
            .arg("--print")
            .arg(&prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .with_context(|| {
                format!(
                    "Failed to execute '{}'. Is it installed and in your PATH?",
                    self.binary
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("'{}' exited with error: {}", self.binary, stderr);
        }

        let content = String::from_utf8_lossy(&output.stdout).trim().to_string();

        Ok(ChatResponse {
            content,
            model: self.binary.clone(),
            usage: None,
        })
    }

    /// Get the binary name.
    pub fn binary(&self) -> &str {
        &self.binary
    }
}
