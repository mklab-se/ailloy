use std::io::{self, Write};

use ailloy::terminal::hyperlink;

pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A simple async spinner that prints to stderr.
pub struct Spinner {
    cancel: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

impl Spinner {
    pub fn start(message: &str) -> Self {
        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let msg = message.to_string();
        let handle = tokio::spawn(async move {
            let mut i = 0;
            loop {
                eprint!("\r{} {}", SPINNER_FRAMES[i % SPINNER_FRAMES.len()], msg);
                let _ = io::stderr().flush();
                i += 1;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(80)) => {}
                    _ = cancel_rx.changed() => break,
                }
            }
            // Clear the spinner line
            eprint!("\r{}\r", " ".repeat(msg.len() + 3));
            let _ = io::stderr().flush();
        });
        Self {
            cancel: cancel_tx,
            handle,
        }
    }

    pub fn stop(self) {
        let _ = self.cancel.send(true);
        // Don't block — the task will clean up on its own
        drop(self.handle);
    }
}

/// Strips `<think>...</think>` blocks from model output (used by reasoning models like Qwen, DeepSeek).
/// Returns the cleaned text for display. Works on complete strings (non-streaming).
pub fn strip_think_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[start + end + "</think>".len()..];
        } else {
            // Unclosed <think> — strip everything after it
            return result;
        }
    }
    result.push_str(remaining);
    // Trim leading whitespace that may follow a think block
    if result.starts_with('\n') {
        result = result.trim_start_matches('\n').to_string();
    }
    result
}

/// Streaming filter that suppresses `<think>...</think>` blocks from deltas.
pub struct ThinkFilter {
    /// Are we currently inside a `<think>` block?
    inside_think: bool,
    /// Strip leading newline on next visible output (after closing a think block).
    strip_next_newline: bool,
    /// Buffer for partial tag detection at chunk boundaries.
    pending: String,
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self {
            inside_think: false,
            strip_next_newline: false,
            pending: String::new(),
        }
    }

    /// Feed a delta and return the text that should be displayed.
    pub fn feed(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        let mut output = String::new();

        loop {
            if self.inside_think {
                if let Some(end) = self.pending.find("</think>") {
                    // Skip everything up to and including </think>
                    self.pending = self.pending[end + "</think>".len()..].to_string();
                    self.inside_think = false;
                    self.strip_next_newline = true;
                    continue;
                }
                // Might have a partial "</think" at the end — keep buffering
                if self.pending.len() > "</think>".len() {
                    // Safe to discard everything except the last few chars that could be a partial tag
                    let keep = "</think>".len() - 1;
                    self.pending = self.pending[self.pending.len() - keep..].to_string();
                }
                return output;
            }

            // Strip leading newline after think block close
            if self.strip_next_newline {
                if self.pending.starts_with('\n') {
                    self.pending = self.pending[1..].to_string();
                } else if self.pending.is_empty() {
                    // Newline might arrive in next delta — keep waiting
                    return output;
                }
                self.strip_next_newline = false;
            }

            // Not inside think block
            if let Some(start) = self.pending.find("<think>") {
                // Emit everything before <think>
                output.push_str(&self.pending[..start]);
                self.pending = self.pending[start + "<think>".len()..].to_string();
                self.inside_think = true;
                continue;
            }

            // Check for partial "<think" at the end of pending
            let mut partial_len = 0;
            for i in 1.."<think>".len() {
                if self.pending.ends_with(&"<think>"[..i]) {
                    partial_len = i;
                }
            }

            if partial_len > 0 {
                // Emit everything except the potential partial tag
                let safe = self.pending.len() - partial_len;
                output.push_str(&self.pending[..safe]);
                self.pending = self.pending[safe..].to_string();
            } else {
                // No partial match — emit everything
                output.push_str(&self.pending);
                self.pending.clear();
            }
            return output;
        }
    }

    /// Flush any remaining buffered content (call at end of stream).
    pub fn flush(&mut self) -> String {
        let remaining = std::mem::take(&mut self.pending);
        if self.inside_think {
            // Unclosed think block — don't emit
            String::new()
        } else {
            remaining
        }
    }
}

/// Create a terminal hyperlink for a file path.
pub fn file_hyperlink(path: &str) -> String {
    let url = std::fs::canonicalize(path)
        .map(|p| format!("file://{}", p.display()))
        .unwrap_or_default();
    if url.is_empty() {
        path.to_string()
    } else {
        hyperlink(&url, path)
    }
}
