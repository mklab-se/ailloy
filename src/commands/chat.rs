use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;

use ailloy::client::create_provider_from_node;
use ailloy::config::Config;
use ailloy::terminal::hyperlink;
use ailloy::types::{ChatOptions, ImageOptions, Message, StreamEvent};

use crate::cli::ChatArgs;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A simple async spinner that prints to stderr.
struct Spinner {
    cancel: tokio::sync::watch::Sender<bool>,
    handle: tokio::task::JoinHandle<()>,
}

impl Spinner {
    fn start(message: &str) -> Self {
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

    fn stop(self) {
        let _ = self.cancel.send(true);
        // Don't block — the task will clean up on its own
        drop(self.handle);
    }
}

/// Strips `<think>...</think>` blocks from model output (used by reasoning models like Qwen, DeepSeek).
/// Returns the cleaned text for display. Works on complete strings (non-streaming).
fn strip_think_blocks(text: &str) -> String {
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
struct ThinkFilter {
    /// Are we currently inside a `<think>` block?
    inside_think: bool,
    /// Strip leading newline on next visible output (after closing a think block).
    strip_next_newline: bool,
    /// Buffer for partial tag detection at chunk boundaries.
    pending: String,
}

impl ThinkFilter {
    fn new() -> Self {
        Self {
            inside_think: false,
            strip_next_newline: false,
            pending: String::new(),
        }
    }

    /// Feed a delta and return the text that should be displayed.
    fn feed(&mut self, text: &str) -> String {
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
    fn flush(&mut self) -> String {
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
fn file_hyperlink(path: &str) -> String {
    let url = std::fs::canonicalize(path)
        .map(|p| format!("file://{}", p.display()))
        .unwrap_or_default();
    if url.is_empty() {
        path.to_string()
    } else {
        hyperlink(&url, path)
    }
}
const SVG_SYSTEM_PROMPT: &str =
    "Generate valid SVG markup. Output only the raw SVG code with no explanation or markdown.";

/// Resolve the node to use from args and config.
fn resolve_node_id(args: &ChatArgs, config: &Config, task: &str) -> Result<String> {
    if let Some(ref node_ref) = args.effective_node() {
        let (id, _) = config.get_node(node_ref).with_context(|| {
            format!(
                "Node '{}' not found. Run `ailloy nodes list` to see configured nodes.",
                node_ref
            )
        })?;
        Ok(id.to_string())
    } else {
        let (id, _) = config.default_node_for(task)?;
        Ok(id.to_string())
    }
}

pub async fn run(args: ChatArgs, quiet: bool) -> Result<()> {
    let raw = args.raw;
    let config = Config::load()?;

    // Detect piped stdin
    let stdin_content = if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read from stdin")?;
        if buf.is_empty() { None } else { Some(buf) }
    } else {
        None
    };

    // Determine the message
    let message = match (&args.message, &stdin_content) {
        (Some(msg), Some(stdin)) => Some(format!("{}\n\n{}", msg, stdin)),
        (Some(msg), None) => Some(msg.clone()),
        (None, Some(stdin)) => Some(stdin.clone()),
        (None, None) => None,
    };

    // Interactive mode
    if args.interactive {
        return run_interactive(args, config, message, quiet).await;
    }

    // Need a message for non-interactive mode
    let message = message.context(
        "No message provided. Use 'ailloy \"message\"' or pipe via stdin, or use -i for interactive mode.",
    )?;

    // Determine if this is an image generation request
    if let Some(ref output) = args.output {
        let ext = Path::new(output)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        if let Some(ref ext) = ext {
            if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                return run_image_generation(&args, &config, &message, output, quiet).await;
            }
            if ext == "svg" {
                return run_svg_generation(&args, &config, &message, output, quiet).await;
            }
        }
    }

    // Regular chat
    let node_id = resolve_node_id(&args, &config, "chat")?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    let mut messages = Vec::new();
    if let Some(system) = &args.system {
        messages.push(Message::system(system));
    }
    messages.push(Message::user(&message));

    let options = build_chat_options(&args);

    if !quiet {
        eprintln!("{} {}", "Using:".dimmed(), provider.name().dimmed());
    }

    if args.stream {
        // Streaming mode
        let mut stream = provider.chat_stream(&messages, options.as_ref()).await?;

        let mut output_writer: Box<dyn Write> = if let Some(ref path) = args.output {
            Box::new(
                std::fs::File::create(path)
                    .with_context(|| format!("Failed to create output file: {}", path))?,
            )
        } else {
            Box::new(io::stdout())
        };

        let mut think_filter = ThinkFilter::new();
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::Delta(text) => {
                    let filtered = think_filter.feed(&text);
                    if !filtered.is_empty() {
                        write!(output_writer, "{}", filtered)?;
                        output_writer.flush()?;
                    }
                }
                StreamEvent::Done(response) => {
                    let remaining = think_filter.flush();
                    if !remaining.is_empty() {
                        write!(output_writer, "{}", remaining)?;
                    }
                    if !raw {
                        writeln!(output_writer)?;
                    }
                    if !quiet {
                        if let Some(usage) = &response.usage {
                            eprintln!(
                                "\n{} {} prompt + {} completion = {} total",
                                "Tokens:".dimmed(),
                                usage.prompt_tokens.to_string().dimmed(),
                                usage.completion_tokens.to_string().dimmed(),
                                usage.total_tokens.to_string().dimmed(),
                            );
                        }
                    }
                }
            }
        }
    } else {
        // Non-streaming mode
        let response = provider.chat(&messages, options.as_ref()).await?;

        if let Some(ref path) = args.output {
            std::fs::write(path, &response.content)
                .with_context(|| format!("Failed to write output to: {}", path))?;
            if !quiet {
                eprintln!("{} {}", "Saved to:".dimmed(), file_hyperlink(path));
            }
        } else if raw {
            print!("{}", strip_think_blocks(&response.content));
        } else {
            println!("{}", strip_think_blocks(&response.content));
        }

        if !quiet {
            if let Some(usage) = &response.usage {
                eprintln!(
                    "\n{} {} prompt + {} completion = {} total",
                    "Tokens:".dimmed(),
                    usage.prompt_tokens.to_string().dimmed(),
                    usage.completion_tokens.to_string().dimmed(),
                    usage.total_tokens.to_string().dimmed(),
                );
            }
        }
    }

    Ok(())
}

async fn run_image_generation(
    args: &ChatArgs,
    config: &Config,
    prompt: &str,
    output: &str,
    quiet: bool,
) -> Result<()> {
    let node_id = resolve_node_id(args, config, "image")
        .or_else(|_| resolve_node_id(args, config, "chat"))?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (image generation)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let options = ImageOptions {
        size: None,
        quality: None,
        style: None,
    };

    let image = if quiet {
        provider.generate_image(prompt, Some(&options)).await?
    } else {
        let spinner = Spinner::start("Generating image...");
        let result = provider.generate_image(prompt, Some(&options)).await;
        spinner.stop();
        result?
    };

    std::fs::write(output, &image.data)
        .with_context(|| format!("Failed to write image to: {}", output))?;

    if !quiet {
        eprintln!(
            "{} {} ({}x{}, {})",
            "Saved to:".dimmed(),
            file_hyperlink(output),
            image.width,
            image.height,
            image.format
        );
        if let Some(revised) = &image.revised_prompt {
            eprintln!("{} {}", "Revised prompt:".dimmed(), revised.dimmed());
        }
    }

    Ok(())
}

async fn run_svg_generation(
    args: &ChatArgs,
    config: &Config,
    prompt: &str,
    output: &str,
    quiet: bool,
) -> Result<()> {
    let node_id = resolve_node_id(args, config, "chat")?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (SVG via chat)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let messages = vec![Message::system(SVG_SYSTEM_PROMPT), Message::user(prompt)];

    let options = build_chat_options(args);
    let response = provider.chat(&messages, options.as_ref()).await?;

    std::fs::write(output, &response.content)
        .with_context(|| format!("Failed to write SVG to: {}", output))?;

    if !quiet {
        eprintln!("{} {}", "Saved to:".dimmed(), file_hyperlink(output));
    }

    Ok(())
}

async fn run_interactive(
    mut args: ChatArgs,
    config: Config,
    initial_message: Option<String>,
    quiet: bool,
) -> Result<()> {
    // Always stream in interactive mode for real-time token display
    args.stream = true;
    let node_id = resolve_node_id(&args, &config, "chat")?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    let version = env!("CARGO_PKG_VERSION");
    eprintln!(
        "{} v{} — {} ({})",
        "ailloy".bold(),
        version,
        node_id.bold(),
        provider.name().dimmed()
    );
    eprintln!(
        "Type {} for commands, {} to exit.",
        "/help".bold(),
        "/quit".bold()
    );
    eprintln!();

    let mut history: Vec<Message> = Vec::new();

    if let Some(system) = &args.system {
        history.push(Message::system(system));
    }

    let chat_options = build_chat_options(&args);

    // Handle initial message if provided
    if let Some(msg) = initial_message {
        history.push(Message::user(&msg));

        if args.stream {
            let mut stream = provider
                .chat_stream(&history, chat_options.as_ref())
                .await?;
            let mut assembled = String::new();
            let mut think_filter = ThinkFilter::new();
            while let Some(event) = stream.next().await {
                match event? {
                    StreamEvent::Delta(text) => {
                        assembled.push_str(&text);
                        let filtered = think_filter.feed(&text);
                        if !filtered.is_empty() {
                            print!("{}", filtered);
                            io::stdout().flush()?;
                        }
                    }
                    StreamEvent::Done(_) => {
                        let remaining = think_filter.flush();
                        if !remaining.is_empty() {
                            print!("{}", remaining);
                        }
                        println!();
                    }
                }
            }
            history.push(Message::assistant(&assembled));
        } else {
            let response = provider.chat(&history, chat_options.as_ref()).await?;
            println!("{}", strip_think_blocks(&response.content));
            history.push(Message::assistant(&response.content));
        }
        println!();
    }

    // REPL loop
    loop {
        eprint!("{} ", ">".bold());
        io::stderr().flush()?;

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        // Handle commands
        match input {
            "/quit" | "/exit" | "/q" => break,
            "/clear" => {
                // Keep system prompt if any
                let system = history
                    .iter()
                    .find(|m| m.role == ailloy::types::Role::System)
                    .cloned();
                history.clear();
                if let Some(sys) = system {
                    history.push(sys);
                }
                eprintln!("{}", "History cleared.".dimmed());
                continue;
            }
            "/help" => {
                eprintln!("{}", "Commands:".bold());
                eprintln!("  {} — Exit the session", "/quit".bold());
                eprintln!("  {} — Clear conversation history", "/clear".bold());
                eprintln!("  {} — Show this help", "/help".bold());
                continue;
            }
            _ if input.starts_with('/') => {
                eprintln!(
                    "{} Unknown command: {}. Type {} for help.",
                    "!".yellow().bold(),
                    input,
                    "/help".bold()
                );
                continue;
            }
            _ => {}
        }

        history.push(Message::user(input));

        if args.stream {
            let mut stream = provider
                .chat_stream(&history, chat_options.as_ref())
                .await?;
            let mut assembled = String::new();
            let mut think_filter = ThinkFilter::new();
            while let Some(event) = stream.next().await {
                match event? {
                    StreamEvent::Delta(text) => {
                        assembled.push_str(&text);
                        let filtered = think_filter.feed(&text);
                        if !filtered.is_empty() {
                            print!("{}", filtered);
                            io::stdout().flush()?;
                        }
                    }
                    StreamEvent::Done(_) => {
                        let remaining = think_filter.flush();
                        if !remaining.is_empty() {
                            print!("{}", remaining);
                        }
                        println!();
                    }
                }
            }
            history.push(Message::assistant(&assembled));
        } else {
            let response = provider.chat(&history, chat_options.as_ref()).await?;
            println!("{}", strip_think_blocks(&response.content));
            history.push(Message::assistant(&response.content));
        }

        if !quiet {
            println!();
        }
    }

    Ok(())
}

fn build_chat_options(args: &ChatArgs) -> Option<ChatOptions> {
    if args.max_tokens.is_some() || args.temperature.is_some() {
        Some(ChatOptions {
            max_tokens: args.max_tokens,
            temperature: args.temperature,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_think_blocks_simple() {
        let input = "<think>some reasoning</think>\nHello!";
        assert_eq!(strip_think_blocks(input), "Hello!");
    }

    #[test]
    fn test_strip_think_blocks_no_think() {
        assert_eq!(strip_think_blocks("Just text"), "Just text");
    }

    #[test]
    fn test_strip_think_blocks_multiple() {
        let input = "<think>a</think>Hello <think>b</think>world";
        assert_eq!(strip_think_blocks(input), "Hello world");
    }

    #[test]
    fn test_strip_think_blocks_unclosed() {
        let input = "<think>still thinking...";
        assert_eq!(strip_think_blocks(input), "");
    }

    #[test]
    fn test_think_filter_streaming_complete_tags() {
        let mut filter = ThinkFilter::new();
        assert_eq!(filter.feed("<think>"), "");
        assert_eq!(filter.feed("reasoning here"), "");
        assert_eq!(filter.feed("</think>"), "");
        assert_eq!(filter.feed("\nHello!"), "Hello!");
        assert_eq!(filter.flush(), "");
    }

    #[test]
    fn test_think_filter_streaming_split_open_tag() {
        let mut filter = ThinkFilter::new();
        assert_eq!(filter.feed("<thi"), "");
        assert_eq!(filter.feed("nk>"), "");
        assert_eq!(filter.feed("thinking..."), "");
        assert_eq!(filter.feed("</think>"), "");
        assert_eq!(filter.feed("Answer"), "Answer");
    }

    #[test]
    fn test_think_filter_no_think() {
        let mut filter = ThinkFilter::new();
        assert_eq!(filter.feed("Hello "), "Hello ");
        assert_eq!(filter.feed("world"), "world");
        assert_eq!(filter.flush(), "");
    }

    #[test]
    fn test_think_filter_text_before_think() {
        let mut filter = ThinkFilter::new();
        assert_eq!(filter.feed("Prefix<think>"), "Prefix");
        assert_eq!(filter.feed("hidden</think>Visible"), "Visible");
    }

    #[test]
    fn test_think_filter_flush_inside_think() {
        let mut filter = ThinkFilter::new();
        filter.feed("<think>unclosed");
        assert_eq!(filter.flush(), "");
    }
}
