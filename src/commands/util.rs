use std::io::{self, Write};

use ailloy::config::{AiNode, Capability, ProviderKind};
use ailloy::pricing::{self, CostTracker};
use ailloy::terminal::hyperlink;
use ailloy::types::Usage;
use colored::Colorize;

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
        // Don't block: the task will clean up on its own
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
            // Unclosed <think>: strip everything after it
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
                // Might have a partial "</think" at the end, keep buffering
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
                    // Newline might arrive in next delta, keep waiting
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
                // No partial match: emit everything
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
            // Unclosed think block: don't emit
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

/// Estimated USD cost of one call to `node` for `cap`, or `None` when the
/// model has no known price.
pub fn estimate_cost(node: &AiNode, cap: &Capability, usage: &Usage) -> Option<f64> {
    pricing::price_for_node(node, cap)?.cost_of(usage)
}

/// Sum several usage blocks (image variants report one each on some models).
pub fn sum_usage<'a>(usages: impl IntoIterator<Item = &'a Usage>) -> Option<Usage> {
    usages.into_iter().fold(None, |acc: Option<Usage>, u| {
        let mut total = acc.unwrap_or(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        });
        total.prompt_tokens += u.prompt_tokens;
        total.completion_tokens += u.completion_tokens;
        total.total_tokens += u.total_tokens;
        Some(total)
    })
}

/// The node-default keys a user sets to price `cap` (for actionable hints).
fn price_keys(cap: &Capability) -> &'static str {
    match cap {
        Capability::Image => "image.price_input/image.price_output",
        Capability::Embedding => "embedding.price_input",
        Capability::Video => "video.price_second",
        _ => "chat.price_input/chat.price_output",
    }
}

/// Human text for an estimated cost: `≈ $0.0059`, `$0 (local)`, or an
/// actionable hint when the model has no known price. `None` for local CLI
/// agents, which bill through their own subscription.
pub fn cost_text(node: &AiNode, cap: &Capability, cost: Option<f64>) -> Option<String> {
    match (&node.provider, cost) {
        (ProviderKind::LocalAgent, None) => None,
        (ProviderKind::Ollama, _) => Some("$0 (local)".to_string()),
        (_, Some(c)) => Some(format!("\u{2248} {}", pricing::format_usd(c))),
        (_, None) => Some(format!(
            "unknown for '{}', set {} on the node in 'ailloy ai config'",
            node.detail(),
            price_keys(cap)
        )),
    }
}

/// The dimmed `Tokens: ... · Cost: ...` summary line printed after a call.
pub fn usage_line(node: &AiNode, cap: &Capability, usage: &Usage) -> String {
    let tokens = if *cap == Capability::Embedding {
        format!(
            "{} {}",
            "Tokens:".dimmed(),
            usage.prompt_tokens.to_string().dimmed()
        )
    } else {
        format!(
            "{} {} prompt + {} completion = {} total",
            "Tokens:".dimmed(),
            usage.prompt_tokens.to_string().dimmed(),
            usage.completion_tokens.to_string().dimmed(),
            usage.total_tokens.to_string().dimmed(),
        )
    };
    match cost_text(node, cap, estimate_cost(node, cap, usage)) {
        Some(cost) => format!("{tokens}  {} {}", "Cost:".dimmed(), cost.dimmed()),
        None => tokens,
    }
}

/// The dimmed `Cost: ...` line for `seconds` of generated video.
pub fn video_cost_line(node: &AiNode, seconds: f64) -> Option<String> {
    let cost =
        pricing::price_for_node(node, &Capability::Video).and_then(|p| p.cost_of_seconds(seconds));
    cost_text(node, &Capability::Video, cost)
        .map(|text| format!("{} {}", "Cost:".dimmed(), text.dimmed()))
}

/// The dimmed session summary: `Session cost: ≈ $0.031 over 4 calls`.
pub fn session_line(node: &AiNode, cap: &Capability, tracker: &CostTracker) -> String {
    let calls = tracker.calls();
    let plural = if calls == 1 { "call" } else { "calls" };
    let priced = calls - tracker.unpriced_calls();
    let amount = if priced == 0 {
        cost_text(node, cap, None).unwrap_or_else(|| "unknown".to_string())
    } else {
        let mut text = cost_text(node, cap, Some(tracker.total())).unwrap_or_default();
        if tracker.unpriced_calls() > 0 {
            text.push_str(&format!(
                " ({} without usage data)",
                tracker.unpriced_calls()
            ));
        }
        text
    };
    format!(
        "{} {} over {calls} {plural}",
        "Session cost:".dimmed(),
        amount.dimmed()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(provider: ProviderKind, model: &str) -> AiNode {
        let mut n = AiNode::new(provider);
        n.model = Some(model.to_string());
        n
    }

    fn usage(p: u32, c: u32) -> Usage {
        Usage {
            prompt_tokens: p,
            completion_tokens: c,
            total_tokens: p + c,
        }
    }

    #[test]
    fn usage_line_shows_estimated_cost() {
        colored::control::set_override(false);
        let n = node(ProviderKind::MicrosoftFoundry, "gpt-image-2.5-flare");
        let line = usage_line(&n, &Capability::Image, &usage(13, 196));
        assert_eq!(
            line,
            "Tokens: 13 prompt + 196 completion = 209 total  Cost: \u{2248} $0.0059"
        );
    }

    #[test]
    fn unknown_price_gives_actionable_hint() {
        colored::control::set_override(false);
        let n = node(ProviderKind::Anthropic, "claude-next");
        let line = usage_line(&n, &Capability::Chat, &usage(10, 10));
        assert!(line.contains("unknown for 'claude-next'"), "{line}");
        assert!(line.contains("chat.price_input"), "{line}");
        assert!(line.contains("ailloy ai config"), "{line}");
    }

    #[test]
    fn local_agents_show_no_cost_and_ollama_is_free() {
        let agent = node(ProviderKind::LocalAgent, "codex");
        assert_eq!(cost_text(&agent, &Capability::Chat, None), None);
        let ollama = node(ProviderKind::Ollama, "llama3");
        assert_eq!(
            cost_text(&ollama, &Capability::Chat, Some(0.0)).as_deref(),
            Some("$0 (local)")
        );
    }

    #[test]
    fn embedding_line_shows_only_input_tokens() {
        colored::control::set_override(false);
        let n = node(ProviderKind::MicrosoftFoundry, "text-embedding-3-large");
        let line = usage_line(&n, &Capability::Embedding, &usage(100, 0));
        assert_eq!(line, "Tokens: 100  Cost: \u{2248} <$0.0001");
    }

    #[test]
    fn sum_usage_adds_variants() {
        let all = [usage(10, 100), usage(10, 100)];
        let total = sum_usage(all.iter()).unwrap();
        assert_eq!((total.prompt_tokens, total.completion_tokens), (20, 200));
        assert!(sum_usage(std::iter::empty()).is_none());
    }

    #[test]
    fn session_line_totals_and_flags_unpriced_calls() {
        colored::control::set_override(false);
        let n = node(ProviderKind::MicrosoftFoundry, "gpt-6.1-sol");
        let mut t = CostTracker::new();
        t.record(Some(0.01));
        t.record(Some(0.02));
        assert_eq!(
            session_line(&n, &Capability::Chat, &t),
            "Session cost: \u{2248} $0.030 over 2 calls"
        );
        t.record(None);
        assert!(session_line(&n, &Capability::Chat, &t).contains("(1 without usage data)"));
    }
}
