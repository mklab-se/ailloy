//! Local CLI agent integration (Claude, Codex, Copilot).
//!
//! Invokes AI tools installed on the system as subprocesses.

use std::process::Stdio;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tracing::debug;

use crate::client::Provider;
use crate::types::{ChatOptions, ChatResponse, ChatStream, Message, StreamEvent};

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

    /// Send a chat message via the local agent's `--print` flag (legacy).
    pub async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        <Self as Provider>::chat(self, messages, None).await
    }

    /// Get the binary name.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    fn build_prompt(messages: &[Message]) -> String {
        messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
impl Provider for LocalAgentClient {
    fn name(&self) -> &str {
        &self.binary
    }

    async fn chat(
        &self,
        messages: &[Message],
        _options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        let prompt = Self::build_prompt(messages);

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

    async fn chat_stream(
        &self,
        messages: &[Message],
        _options: Option<&ChatOptions>,
    ) -> Result<ChatStream> {
        let prompt = Self::build_prompt(messages);

        debug!(binary = %self.binary, "Sending streaming prompt to local agent");

        let mut child = Command::new(&self.binary)
            .arg("--print")
            .arg(&prompt)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to execute '{}'. Is it installed and in your PATH?",
                    self.binary
                )
            })?;

        let stdout = child
            .stdout
            .take()
            .context("Failed to capture stdout from local agent")?;

        let binary = self.binary.clone();
        let reader = tokio::io::BufReader::new(stdout);
        let lines = reader.lines();

        let stream = futures_util::stream::unfold(
            (lines, String::new(), binary, Some(child)),
            |(mut lines, mut assembled, binary, mut child)| async move {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let text = format!("{}\n", line);
                        assembled.push_str(&text);
                        Some((
                            Ok(StreamEvent::Delta(text)),
                            (lines, assembled, binary, child),
                        ))
                    }
                    Ok(None) => {
                        // Wait for process to finish
                        if let Some(ref mut c) = child {
                            let _ = c.wait().await;
                        }
                        if !assembled.is_empty() {
                            let content = assembled.trim().to_string();
                            let response = ChatResponse {
                                content,
                                model: binary.clone(),
                                usage: None,
                            };
                            assembled.clear();
                            Some((
                                Ok(StreamEvent::Done(response)),
                                (lines, assembled, binary, None),
                            ))
                        } else {
                            None
                        }
                    }
                    Err(e) => Some((Err(e.into()), (lines, assembled, binary, child))),
                }
            },
        );

        Ok(Box::pin(stream))
    }
}
