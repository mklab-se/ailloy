use anyhow::Result;
use colored::Colorize;

use ailloy::config::Config;
use ailloy::provider::{create_provider, create_provider_by_name};
use ailloy::types::Message;

use crate::cli::ChatArgs;

pub async fn run(args: ChatArgs, quiet: bool) -> Result<()> {
    let config = Config::load()?;

    let provider = if let Some(ref name) = args.provider {
        create_provider_by_name(name, &config)?
    } else {
        create_provider(&config)?
    };

    let mut messages = Vec::new();

    if let Some(system) = args.system {
        messages.push(Message::system(system));
    }

    messages.push(Message::user(args.message));

    if !quiet {
        eprintln!("{} {}", "Using:".dimmed(), provider.model().dimmed());
    }

    let response = provider.chat(&messages).await?;

    println!("{}", response.content);

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

    Ok(())
}
