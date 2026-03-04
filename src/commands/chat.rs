use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use futures_util::StreamExt;

use ailloy::client::create_provider_from_config;
use ailloy::config::Config;
use ailloy::terminal::hyperlink;
use ailloy::types::{ChatOptions, ImageOptions, Message, StreamEvent};

use crate::cli::ChatArgs;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

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
    let provider_name = if let Some(ref name) = args.provider {
        name.clone()
    } else {
        let (name, _) = config.default_provider_config()?;
        name.to_string()
    };
    let provider_config = config.provider_config(&provider_name)?;
    let provider = create_provider_from_config(&provider_name, provider_config)?;

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

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::Delta(text) => {
                    write!(output_writer, "{}", text)?;
                    output_writer.flush()?;
                }
                StreamEvent::Done(response) => {
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
            print!("{}", response.content);
        } else {
            println!("{}", response.content);
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
    let provider_name = if let Some(ref name) = args.provider {
        name.clone()
    } else {
        let (name, _) = config.provider_for_task("image").or_else(|_| {
            // Fall back to chat provider if no image-specific default
            config.default_provider_config()
        })?;
        name.to_string()
    };
    let provider_config = config.provider_config(&provider_name)?;
    let provider = create_provider_from_config(&provider_name, provider_config)?;

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

    let image = provider.generate_image(prompt, Some(&options)).await?;

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
    let provider_name = if let Some(ref name) = args.provider {
        name.clone()
    } else {
        let (name, _) = config.default_provider_config()?;
        name.to_string()
    };
    let provider_config = config.provider_config(&provider_name)?;
    let provider = create_provider_from_config(&provider_name, provider_config)?;

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
    args: ChatArgs,
    config: Config,
    initial_message: Option<String>,
    quiet: bool,
) -> Result<()> {
    let provider_name = if let Some(ref name) = args.provider {
        name.clone()
    } else {
        let (name, _) = config.default_provider_config()?;
        name.to_string()
    };
    let provider_config = config.provider_config(&provider_name)?;
    let provider = create_provider_from_config(&provider_name, provider_config)?;

    let version = env!("CARGO_PKG_VERSION");
    eprintln!(
        "{} v{} — {} ({})",
        "ailloy".bold(),
        version,
        provider_name.bold(),
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
            while let Some(event) = stream.next().await {
                match event? {
                    StreamEvent::Delta(text) => {
                        print!("{}", text);
                        io::stdout().flush()?;
                        assembled.push_str(&text);
                    }
                    StreamEvent::Done(_) => {
                        println!();
                    }
                }
            }
            history.push(Message::assistant(&assembled));
        } else {
            let response = provider.chat(&history, chat_options.as_ref()).await?;
            println!("{}", response.content);
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
            while let Some(event) = stream.next().await {
                match event? {
                    StreamEvent::Delta(text) => {
                        print!("{}", text);
                        io::stdout().flush()?;
                        assembled.push_str(&text);
                    }
                    StreamEvent::Done(_) => {
                        println!();
                    }
                }
            }
            history.push(Message::assistant(&assembled));
        } else {
            let response = provider.chat(&history, chat_options.as_ref()).await?;
            println!("{}", response.content);
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
