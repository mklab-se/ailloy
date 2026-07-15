use std::collections::BTreeMap;
use std::io::{self, Write};

use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;

use ailloy::client::{
    Provider, create_provider_from_node, merge_chat_defaults, merge_image_defaults,
};
use ailloy::config::Config;
use ailloy::types::{
    Background, ChatOptions, ImageFormat, ImageOptions, InputFidelity, Message, Moderation,
    StreamEvent,
};

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

    generate_and_save(
        provider.as_ref(),
        prompt,
        args,
        node.node_defaults.as_ref(),
        quiet,
    )
    .await
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

    // The interview conversation uses the chat node — honor its per-node
    // chat defaults (e.g. chat.temperature) for every stream below.
    let interview_options = {
        let mut opts = ChatOptions::default();
        if let Some(defaults) = &chat_node.node_defaults {
            merge_chat_defaults(&mut opts, defaults);
        }
        opts
    };

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
        let mut stream = chat_provider
            .chat_stream(&history, Some(&interview_options))
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
        let mut stream = chat_provider
            .chat_stream(&history, Some(&interview_options))
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
                let mut stream = chat_provider
                    .chat_stream(&history, Some(&interview_options))
                    .await?;
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

    generate_and_save(
        provider.as_ref(),
        prompt,
        args,
        node.node_defaults.as_ref(),
        quiet,
    )
    .await
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

/// Generate image(s) with the given provider and prompt, then write each
/// variant to disk and print metadata (unless `quiet`).
async fn generate_and_save(
    provider: &dyn Provider,
    prompt: &str,
    args: &ImageArgs,
    node_defaults: Option<&BTreeMap<String, String>>,
    quiet: bool,
) -> Result<()> {
    let mut options = build_image_options(args)?;
    // Fill unset fields from per-node defaults; explicit flags already won.
    if let Some(defaults) = node_defaults {
        merge_image_defaults(&mut options, defaults);
    }

    let images = if quiet {
        provider.generate_images(prompt, Some(&options)).await?
    } else {
        let spinner = Spinner::start("Generating image...");
        let result = provider.generate_images(prompt, Some(&options)).await;
        spinner.stop();
        result?
    };

    if images.is_empty() {
        anyhow::bail!("Provider returned no images");
    }

    let base_output = args
        .output
        .clone()
        .unwrap_or_else(|| auto_filename(&images[0].format.to_string()));

    for (i, image) in images.iter().enumerate() {
        let output = variant_path(&base_output, i);
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
    }

    if !quiet {
        if let Some(usage) = images.iter().find_map(|img| img.usage.as_ref()) {
            eprintln!(
                "{} {} prompt + {} completion = {} total",
                "Tokens:".dimmed(),
                usage.prompt_tokens.to_string().dimmed(),
                usage.completion_tokens.to_string().dimmed(),
                usage.total_tokens.to_string().dimmed(),
            );
        }
    }

    Ok(())
}

/// Build [`ImageOptions`] from CLI flags, parsing enum flags via their
/// `FromStr` impls so invalid values surface actionable errors.
pub(crate) fn build_image_options(args: &ImageArgs) -> Result<ImageOptions> {
    let output_format = args
        .format
        .as_deref()
        .map(str::parse::<ImageFormat>)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let background = args
        .background
        .as_deref()
        .map(str::parse::<Background>)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let moderation = args
        .moderation
        .as_deref()
        .map(str::parse::<Moderation>)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let input_fidelity = args
        .fidelity
        .as_deref()
        .map(str::parse::<InputFidelity>)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;

    let options = ImageOptions {
        size: args.size.as_deref().and_then(parse_size),
        quality: args.quality.clone(),
        style: args.style.clone(),
        output_format,
        compression: args.compression,
        n: args.variants,
        background,
        moderation,
        input_fidelity,
        reference_images: args
            .reference
            .iter()
            .map(std::path::PathBuf::from)
            .collect(),
        mask: args.mask.as_ref().map(std::path::PathBuf::from),
    };

    options.validate()?;

    Ok(options)
}

/// Compute the output path for the `index`-th image variant (0-based).
/// `index == 0` returns `path` unchanged; subsequent variants get a
/// `-2`, `-3`, ... suffix inserted before the last extension (or appended if
/// there is none).
pub(crate) fn variant_path(path: &str, index: usize) -> String {
    if index == 0 {
        return path.to_string();
    }
    let suffix = format!("-{}", index + 1);

    let (dir, file_name) = match path.rfind('/') {
        Some(pos) => (&path[..=pos], &path[pos + 1..]),
        None => ("", path),
    };

    let new_file_name = match file_name.rfind('.') {
        Some(pos) if pos > 0 => format!("{}{}{}", &file_name[..pos], suffix, &file_name[pos..]),
        _ => format!("{}{}", file_name, suffix),
    };

    format!("{}{}", dir, new_file_name)
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

    // --- Task 1.7: variant_path ---

    #[test]
    fn test_variant_path_first_index_unchanged() {
        assert_eq!(variant_path("out.png", 0), "out.png");
        assert_eq!(variant_path("a/b.out.png", 0), "a/b.out.png");
    }

    #[test]
    fn test_variant_path_suffixes_before_extension() {
        assert_eq!(variant_path("out.png", 1), "out-2.png");
        assert_eq!(variant_path("out.png", 2), "out-3.png");
    }

    #[test]
    fn test_variant_path_preserves_directory_and_last_extension_only() {
        assert_eq!(variant_path("a/b.out.png", 1), "a/b.out-2.png");
    }

    #[test]
    fn test_variant_path_no_extension_appends_suffix() {
        assert_eq!(variant_path("noext", 1), "noext-2");
        assert_eq!(variant_path("dir/sub/noext", 1), "dir/sub/noext-2");
    }

    // --- Task 1.7: build_image_options ---

    #[test]
    fn test_build_image_options_defaults() {
        let args = ImageArgs::default();
        let opts = build_image_options(&args).unwrap();
        assert_eq!(opts.size, None);
        assert_eq!(opts.output_format, None);
        assert_eq!(opts.compression, None);
        assert_eq!(opts.n, None);
        assert_eq!(opts.background, None);
        assert_eq!(opts.moderation, None);
        assert_eq!(opts.input_fidelity, None);
        assert!(opts.reference_images.is_empty());
        assert_eq!(opts.mask, None);
    }

    #[test]
    fn test_build_image_options_maps_all_flags() {
        let args = ImageArgs {
            size: Some("1024x1024".to_string()),
            quality: Some("hd".to_string()),
            format: Some("webp".to_string()),
            compression: Some(80),
            variants: Some(3),
            background: Some("opaque".to_string()),
            moderation: Some("low".to_string()),
            fidelity: Some("high".to_string()),
            reference: vec!["ref1.png".to_string(), "ref2.png".to_string()],
            mask: Some("mask.png".to_string()),
            ..Default::default()
        };
        let opts = build_image_options(&args).unwrap();
        assert_eq!(opts.size, Some((1024, 1024)));
        assert_eq!(opts.quality.as_deref(), Some("hd"));
        assert_eq!(opts.output_format, Some(ailloy::types::ImageFormat::Webp));
        assert_eq!(opts.compression, Some(80));
        assert_eq!(opts.n, Some(3));
        assert_eq!(opts.background, Some(ailloy::types::Background::Opaque));
        assert_eq!(opts.moderation, Some(ailloy::types::Moderation::Low));
        assert_eq!(
            opts.input_fidelity,
            Some(ailloy::types::InputFidelity::High)
        );
        assert_eq!(
            opts.reference_images,
            vec![
                std::path::PathBuf::from("ref1.png"),
                std::path::PathBuf::from("ref2.png"),
            ]
        );
        assert_eq!(opts.mask, Some(std::path::PathBuf::from("mask.png")));
    }

    #[test]
    fn test_build_image_options_invalid_format_is_actionable() {
        let args = ImageArgs {
            format: Some("bmp".to_string()),
            ..Default::default()
        };
        let err = build_image_options(&args).unwrap_err().to_string();
        assert!(err.contains("bmp"));
        assert!(err.contains("png"));
    }

    #[test]
    fn test_build_image_options_invalid_background_is_actionable() {
        let args = ImageArgs {
            background: Some("invisible".to_string()),
            ..Default::default()
        };
        let err = build_image_options(&args).unwrap_err().to_string();
        assert!(err.contains("invisible"));
    }

    // --- Task 6.2: CLI honors per-node default parameters ---

    #[test]
    fn test_node_defaults_fill_unset_image_options() {
        // No CLI flags → build_image_options yields all-None, then node
        // defaults populate the unset fields.
        let opts_from_flags = build_image_options(&ImageArgs::default()).unwrap();
        let mut opts = opts_from_flags;
        let defaults = BTreeMap::from([
            ("image.format".to_string(), "jpeg".to_string()),
            ("image.compression".to_string(), "80".to_string()),
            ("image.quality".to_string(), "low".to_string()),
        ]);
        merge_image_defaults(&mut opts, &defaults);
        assert_eq!(opts.output_format, Some(ImageFormat::Jpeg));
        assert_eq!(opts.compression, Some(80));
        assert_eq!(opts.quality.as_deref(), Some("low"));
    }

    #[test]
    fn test_explicit_image_flags_win_over_node_defaults() {
        // Explicit flags are already Some; merging node defaults must not
        // clobber them.
        let args = ImageArgs {
            format: Some("webp".to_string()),
            compression: Some(50),
            quality: Some("hd".to_string()),
            ..Default::default()
        };
        let mut opts = build_image_options(&args).unwrap();
        let defaults = BTreeMap::from([
            ("image.format".to_string(), "jpeg".to_string()),
            ("image.compression".to_string(), "80".to_string()),
            ("image.quality".to_string(), "low".to_string()),
        ]);
        merge_image_defaults(&mut opts, &defaults);
        assert_eq!(opts.output_format, Some(ImageFormat::Webp));
        assert_eq!(opts.compression, Some(50));
        assert_eq!(opts.quality.as_deref(), Some("hd"));
    }

    #[test]
    fn test_build_image_options_validate_rejects_bad_combo() {
        // compression without output_format triggers ImageOptions::validate()
        let args = ImageArgs {
            compression: Some(50),
            ..Default::default()
        };
        let err = build_image_options(&args).unwrap_err().to_string();
        assert!(err.contains("compression"));
        assert!(err.contains("output_format"));
    }
}
