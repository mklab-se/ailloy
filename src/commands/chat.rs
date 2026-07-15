use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;

use ailloy::client::{
    create_provider_from_node, merge_chat_defaults, merge_image_defaults, merge_video_defaults,
};
use ailloy::config::{AiNode, Config};
use ailloy::types::{ChatOptions, ImageOptions, Message, StreamEvent, VideoOptions};

use super::util::{Spinner, ThinkFilter, file_hyperlink, strip_think_blocks};
use crate::cli::ChatArgs;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

const SVG_SYSTEM_PROMPT: &str =
    "Generate valid SVG markup. Output only the raw SVG code with no explanation or markdown.";

/// What kind of generation a chat `-o <path>` output extension routes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputKind {
    Image,
    Svg,
    Video,
    Text,
}

/// Classify an output path's extension for `-o` routing.
fn output_kind(path: &str) -> OutputKind {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some(e) if IMAGE_EXTENSIONS.contains(&e) => OutputKind::Image,
        Some("svg") => OutputKind::Svg,
        Some("mp4") => OutputKind::Video,
        _ => OutputKind::Text,
    }
}

/// Build a user [`Message`], attaching files (if any) via
/// [`Message::user_with_attachments`]. With an empty `attach` list this is
/// equivalent to `Message::user(text)` (plain `Text` content).
fn user_message(text: &str, attach: &[String]) -> Result<Message> {
    if attach.is_empty() {
        return Ok(Message::user(text));
    }
    let paths: Vec<std::path::PathBuf> = attach.iter().map(std::path::PathBuf::from).collect();
    Message::user_with_attachments(text, &paths)
}

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

    // Determine if this is an image/video/SVG generation request based on
    // the -o extension.
    if let Some(ref output) = args.output {
        match output_kind(output) {
            OutputKind::Image => {
                return run_image_generation(&args, &config, &message, output, quiet).await;
            }
            OutputKind::Svg => {
                return run_svg_generation(&args, &config, &message, output, quiet).await;
            }
            OutputKind::Video => {
                return run_video_generation(&args, &config, &message, output, quiet).await;
            }
            OutputKind::Text => {}
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
    messages.push(user_message(&message, &args.attach)?);

    let options = apply_node_chat_defaults(build_chat_options(&args)?, node);

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

    let mut options = ImageOptions::default();
    if let Some(defaults) = &node.node_defaults {
        merge_image_defaults(&mut options, defaults);
    }

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

async fn run_video_generation(
    args: &ChatArgs,
    config: &Config,
    prompt: &str,
    output: &str,
    quiet: bool,
) -> Result<()> {
    let node_id = resolve_node_id(args, config, "video")
        .or_else(|_| resolve_node_id(args, config, "chat"))?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (video generation)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let mut options = VideoOptions::default();
    if let Some(defaults) = &node.node_defaults {
        merge_video_defaults(&mut options, defaults);
    }

    let videos = if quiet {
        provider
            .generate_video(prompt, Some(&options), None)
            .await?
    } else {
        let spinner = Spinner::start("Generating video...");
        let result = provider.generate_video(prompt, Some(&options), None).await;
        spinner.stop();
        result?
    };

    let video = videos
        .into_iter()
        .next()
        .context("Provider returned no videos")?;

    std::fs::write(output, &video.data)
        .with_context(|| format!("Failed to write video to: {}", output))?;

    if !quiet {
        eprintln!(
            "{} {} ({}x{}, {}s)",
            "Saved to:".dimmed(),
            file_hyperlink(output),
            video.width,
            video.height,
            video.duration_seconds,
        );
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

    let messages = vec![
        Message::system(SVG_SYSTEM_PROMPT),
        user_message(prompt, &args.attach)?,
    ];

    let options = apply_node_chat_defaults(build_chat_options(args)?, node);
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

    let mut history: Vec<Message> = Vec::new();

    if let Some(system) = &args.system {
        history.push(Message::system(system));
    } else {
        history.push(Message::system(
            "You are Ailloy, a helpful AI assistant. Be concise and friendly.",
        ));
    }

    let chat_options = apply_node_chat_defaults(build_chat_options(&args)?, node);

    // Generate greeting or handle initial message. Attachments (if any) apply
    // only to this first user message.
    let greeting_msg = initial_message.unwrap_or_else(|| "Say hi briefly.".to_string());
    history.push(user_message(&greeting_msg, &args.attach)?);

    eprintln!();
    {
        let spinner = Spinner::start("Thinking...");
        let mut stream = provider
            .chat_stream(&history, chat_options.as_ref())
            .await?;
        spinner.stop();

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
    }
    println!();

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

/// Merge the resolved node's per-node chat defaults into the CLI-built options,
/// filling only fields the user did not set. Returns `Some` whenever the node
/// carries (non-empty) defaults so they still apply when no flags were passed.
fn apply_node_chat_defaults(opts: Option<ChatOptions>, node: &AiNode) -> Option<ChatOptions> {
    match node.node_defaults.as_ref().filter(|d| !d.is_empty()) {
        Some(defaults) => {
            let mut merged = opts.unwrap_or_default();
            merge_chat_defaults(&mut merged, defaults);
            Some(merged)
        }
        None => opts,
    }
}

fn build_chat_options(args: &ChatArgs) -> Result<Option<ChatOptions>> {
    if args.max_tokens.is_some() || args.temperature.is_some() || args.json || args.schema.is_some()
    {
        Ok(Some(ChatOptions {
            max_tokens: args.max_tokens,
            temperature: args.temperature,
            response_format: match (&args.schema, args.json) {
                (Some(path), _) => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("cannot read schema file {path}"))?;
                    let schema: serde_json::Value = serde_json::from_str(&text)
                        .with_context(|| format!("schema file {path} is not valid JSON"))?;
                    let name = Path::new(path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "schema".to_string());
                    Some(ailloy::types::ResponseFormat::JsonSchema {
                        name,
                        schema,
                        strict: true,
                    })
                }
                (None, true) => Some(ailloy::types::ResponseFormat::JsonObject),
                (None, false) => None,
            },
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Task 3.3: --attach plumbing ---

    #[test]
    fn test_user_message_no_attach_is_plain_text() {
        let msg = user_message("hello", &[]).unwrap();
        assert_eq!(msg.content.as_text(), Some("hello"));
        assert!(!msg.content.has_attachments());
    }

    #[test]
    fn test_user_message_with_attach_builds_parts() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("pic.png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();

        let msg = user_message("look at this", &[png.to_string_lossy().into_owned()]).unwrap();
        assert!(msg.content.has_attachments());
        let ailloy::types::MessageContent::Parts(parts) = &msg.content else {
            panic!("expected Parts");
        };
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn test_user_message_missing_file_errors() {
        let err = user_message("hi", &["/nonexistent/definitely/missing.png".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("Failed to read attachment"), "err: {err}");
    }

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

    // --- Task 2.4: -o extension routing ---

    #[test]
    fn test_output_kind_image_extensions() {
        assert_eq!(output_kind("out.png"), OutputKind::Image);
        assert_eq!(output_kind("out.jpg"), OutputKind::Image);
        assert_eq!(output_kind("out.jpeg"), OutputKind::Image);
        assert_eq!(output_kind("out.webp"), OutputKind::Image);
        assert_eq!(output_kind("OUT.PNG"), OutputKind::Image);
    }

    #[test]
    fn test_output_kind_svg() {
        assert_eq!(output_kind("out.svg"), OutputKind::Svg);
        assert_eq!(output_kind("out.SVG"), OutputKind::Svg);
    }

    #[test]
    fn test_output_kind_video() {
        assert_eq!(output_kind("out.mp4"), OutputKind::Video);
        assert_eq!(output_kind("out.MP4"), OutputKind::Video);
        assert_eq!(output_kind("dir/clip.mp4"), OutputKind::Video);
    }

    #[test]
    fn test_output_kind_text_fallback() {
        assert_eq!(output_kind("out.txt"), OutputKind::Text);
        assert_eq!(output_kind("out"), OutputKind::Text);
        assert_eq!(output_kind("out.json"), OutputKind::Text);
    }
}
