use std::io::{self, Write};

use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;

use ailloy::client::create_provider_from_node;
use ailloy::config::Config;
use ailloy::types::{ImageOptions, Message, StreamEvent};

use super::util::{Spinner, ThinkFilter, file_hyperlink, strip_think_blocks};
use crate::cli::ImageArgs;

const IMAGE_SYSTEM_PROMPT: &str = "\
You are Ailloy, a creative image generation assistant. Your job is to help \
the user describe the perfect image they want to create.

Ask about: subject, style (photorealistic, illustration, oil painting, etc.), \
mood, lighting, colors, composition, and any specific details.

Keep your questions focused and concise — one or two questions at a time.

When you and the user have agreed on a description, output the final prompt \
wrapped exactly like this:

[GENERATE: <the complete image generation prompt>]

The user can ask you to refine and regenerate at any time.";

pub async fn run(args: ImageArgs, quiet: bool) -> Result<()> {
    let config = Config::load()?;

    if args.interactive {
        return run_interactive(args, config, quiet).await;
    }

    let message = args.message.as_deref().context(
        "No prompt provided. Use 'ailloy image \"description\"' or -i for interactive mode.",
    )?;

    run_direct(&args, &config, message, quiet).await
}

async fn run_direct(args: &ImageArgs, config: &Config, prompt: &str, quiet: bool) -> Result<()> {
    let node_id = resolve_image_node(args.node.as_deref(), config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (image generation)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let options = build_image_options(args);

    let image = if quiet {
        provider.generate_image(prompt, Some(&options)).await?
    } else {
        let spinner = Spinner::start("Generating image...");
        let result = provider.generate_image(prompt, Some(&options)).await;
        spinner.stop();
        result?
    };

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| auto_filename(&image.format.to_string()));

    std::fs::write(&output, &image.data)
        .with_context(|| format!("Failed to write image to: {}", output))?;

    if !quiet {
        eprintln!(
            "{} {} ({}x{}, {})",
            "Saved to:".dimmed(),
            file_hyperlink(&output),
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

async fn run_interactive(args: ImageArgs, config: Config, quiet: bool) -> Result<()> {
    // We need a chat node for the interview and an image node for generation
    let chat_node_id = {
        let (id, _) = config.default_node_for("chat")?;
        id.to_string()
    };
    let image_node_id = resolve_image_node(args.node.as_deref(), &config)?;

    let (_, chat_node) = config.get_node(&chat_node_id).unwrap();
    let chat_provider = create_provider_from_node(&chat_node_id, chat_node)?;

    let version = env!("CARGO_PKG_VERSION");
    eprintln!(
        "{} v{} — {} ({})",
        "ailloy image".bold(),
        version,
        chat_node_id.bold(),
        chat_provider.name().dimmed()
    );
    eprintln!(
        "Type {} for commands, {} to exit.",
        "/help".bold(),
        "/quit".bold()
    );

    let mut history: Vec<Message> = vec![Message::system(IMAGE_SYSTEM_PROMPT)];
    let mut last_suggested_prompt: Option<String> = None;

    // Generate greeting from the model
    history.push(Message::user(
        "Greet me briefly and tell me you'll help me create an image. \
         Ask what I'd like to create.",
    ));

    eprintln!();
    {
        let spinner = Spinner::start("Thinking...");
        let mut stream = chat_provider.chat_stream(&history, None).await?;
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
                history = vec![Message::system(IMAGE_SYSTEM_PROMPT)];
                last_suggested_prompt = None;
                eprintln!("{}", "History cleared.".dimmed());
                continue;
            }
            "/generate" => {
                if let Some(ref prompt) = last_suggested_prompt {
                    generate_image(&args, &config, &image_node_id, prompt, quiet).await?;
                } else {
                    eprintln!(
                        "{} No prompt suggested yet. Describe what you want first.",
                        "!".yellow().bold()
                    );
                }
                continue;
            }
            "/help" => {
                eprintln!("{}", "Commands:".bold());
                eprintln!(
                    "  {} — Generate image from last suggested prompt",
                    "/generate".bold()
                );
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

        // Stream AI response
        let mut stream = chat_provider.chat_stream(&history, None).await?;
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

        // Check for [GENERATE: ...] marker in the response
        let display_text = strip_think_blocks(&assembled);
        if let Some(prompt) = extract_generate_prompt(&display_text) {
            last_suggested_prompt = Some(prompt.clone());
            println!();

            // Ask for confirmation before generating
            eprint!("{} Generate this image? [Y/n] ", "?".green().bold());
            io::stderr().flush()?;
            let mut confirm = String::new();
            io::stdin().read_line(&mut confirm)?;
            let confirm = confirm.trim().to_lowercase();
            if confirm.is_empty() || confirm == "y" || confirm == "yes" {
                generate_image(&args, &config, &image_node_id, &prompt, quiet).await?;
            } else {
                // Tell the model the user wants to keep refining
                history.push(Message::user(
                    "I'm not happy with that prompt yet. \
                     Ask me what I'd like to change.",
                ));

                let spinner = Spinner::start("Thinking...");
                let mut stream = chat_provider.chat_stream(&history, None).await?;
                spinner.stop();

                let mut followup = String::new();
                let mut think_filter = ThinkFilter::new();
                while let Some(event) = stream.next().await {
                    match event? {
                        StreamEvent::Delta(text) => {
                            followup.push_str(&text);
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
                history.push(Message::assistant(&followup));
            }
        } else {
            // Check if there's a suggested prompt we can store (without marker)
            last_suggested_prompt = extract_suggested_prompt(&display_text);
        }

        if !quiet {
            println!();
        }
    }

    Ok(())
}

/// Generate an image and save to file.
async fn generate_image(
    args: &ImageArgs,
    config: &Config,
    image_node_id: &str,
    prompt: &str,
    quiet: bool,
) -> Result<()> {
    let (_, node) = config.get_node(image_node_id).unwrap();
    let provider = create_provider_from_node(image_node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (image generation)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let options = build_image_options(args);

    let image = if quiet {
        provider.generate_image(prompt, Some(&options)).await?
    } else {
        let spinner = Spinner::start("Generating image...");
        let result = provider.generate_image(prompt, Some(&options)).await;
        spinner.stop();
        result?
    };

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| auto_filename(&image.format.to_string()));

    std::fs::write(&output, &image.data)
        .with_context(|| format!("Failed to write image to: {}", output))?;

    if !quiet {
        eprintln!(
            "{} {} ({}x{}, {})",
            "Saved to:".dimmed(),
            file_hyperlink(&output),
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_image_node(node_override: Option<&str>, config: &Config) -> Result<String> {
    if let Some(node_ref) = node_override {
        let (id, _) = config.get_node(node_ref).with_context(|| {
            format!(
                "Node '{}' not found. Run 'ailloy ai config' to configure nodes.",
                node_ref
            )
        })?;
        Ok(id.to_string())
    } else {
        let (id, _) = config.default_node_for("image")?;
        Ok(id.to_string())
    }
}

fn build_image_options(args: &ImageArgs) -> ImageOptions {
    ImageOptions {
        size: args.size.as_deref().and_then(parse_size),
        quality: args.quality.clone(),
        style: args.style.clone(),
    }
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() == 2 {
        if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
            return Some((w, h));
        }
    }
    None
}

fn auto_filename(format_str: &str) -> String {
    let ext = match format_str.to_lowercase().as_str() {
        "jpeg" => "jpg",
        "webp" => "webp",
        _ => "png",
    };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("image_{}.{}", secs, ext)
}

/// Extract prompt from `[GENERATE: <prompt>]` marker.
fn extract_generate_prompt(text: &str) -> Option<String> {
    let start_marker = "[GENERATE:";
    let start = text.find(start_marker)?;
    let after = &text[start + start_marker.len()..];
    let end = after.find(']')?;
    let prompt = after[..end].trim();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt.to_string())
    }
}

/// Try to extract the last quoted or prominent prompt suggestion from AI text.
/// This is a best-effort heuristic for storing the last suggestion.
fn extract_suggested_prompt(text: &str) -> Option<String> {
    // Look for text in quotes that looks like a prompt (at least 20 chars)
    let mut last_quoted = None;
    let mut remaining = text;
    while let Some(start) = remaining.find('"') {
        let after = &remaining[start + 1..];
        if let Some(end) = after.find('"') {
            let quoted = &after[..end];
            if quoted.len() >= 20 {
                last_quoted = Some(quoted.to_string());
            }
            remaining = &after[end + 1..];
        } else {
            break;
        }
    }
    last_quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_filename() {
        let name = auto_filename("png");
        assert!(name.starts_with("image_"));
        assert!(name.ends_with(".png"));
    }

    #[test]
    fn test_auto_filename_jpeg() {
        let name = auto_filename("jpeg");
        assert!(name.ends_with(".jpg"));
    }

    #[test]
    fn test_parse_size_valid() {
        assert_eq!(parse_size("1024x1024"), Some((1024, 1024)));
        assert_eq!(parse_size("512x768"), Some((512, 768)));
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size("1024"), None);
        assert_eq!(parse_size("abcxdef"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn test_extract_generate_prompt() {
        assert_eq!(
            extract_generate_prompt(
                "Here is the prompt: [GENERATE: A cat in space wearing a top hat]"
            ),
            Some("A cat in space wearing a top hat".to_string())
        );
    }

    #[test]
    fn test_extract_generate_prompt_missing() {
        assert_eq!(extract_generate_prompt("No marker here"), None);
    }

    #[test]
    fn test_extract_generate_prompt_empty() {
        assert_eq!(extract_generate_prompt("[GENERATE: ]"), None);
    }

    #[test]
    fn test_extract_suggested_prompt() {
        let text = "How about this: \"A photorealistic image of a golden retriever playing in autumn leaves\"";
        let result = extract_suggested_prompt(text);
        assert!(result.is_some());
        assert!(result.unwrap().contains("golden retriever"));
    }

    #[test]
    fn test_extract_suggested_prompt_short_quotes() {
        // Short quotes should be ignored
        assert_eq!(extract_suggested_prompt("Use \"vivid\" style"), None);
    }
}
