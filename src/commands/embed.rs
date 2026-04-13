use anyhow::{Context, Result};
use colored::Colorize;

use ailloy::client::create_provider_from_node;
use ailloy::config::Config;

use crate::cli::EmbedArgs;

pub async fn run(args: EmbedArgs, quiet: bool) -> Result<()> {
    let config = Config::load()?;

    // --info: show embedding node metadata
    if args.info {
        return run_info(&args, &config);
    }

    // --azure-vectorizer: output vectorizer JSON
    if let Some(ref name) = args.azure_vectorizer {
        return run_azure_vectorizer(&args, &config, name);
    }

    // Embed text
    let text = args.text.as_deref().context(
        "No text provided. Usage: ailloy embed \"text to embed\"\n\
         Or use --info to show node metadata, --azure-vectorizer NAME for vectorizer config.",
    )?;

    let node_id = resolve_embed_node(args.node.as_deref(), &config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (embedding)",
            "Using:".dimmed(),
            provider.name().dimmed()
        );
    }

    let response = provider.embed(&[text], None).await?;

    let vector = response
        .embeddings
        .first()
        .context("No embedding returned")?;

    if args.full {
        // Print full vector as JSON
        println!("{}", serde_json::to_string(vector)?);
    } else {
        // Print summary
        println!("{} {}", "Model:".bold(), response.model);
        println!("{} {}", "Dimensions:".bold(), vector.len());
        if let Some(usage) = &response.usage {
            println!("{} {}", "Tokens:".bold(), usage.prompt_tokens);
        }
        // Preview: first 5 values
        let preview: Vec<String> = vector.iter().take(5).map(|v| format!("{:.6}", v)).collect();
        let suffix = if vector.len() > 5 {
            format!(", ... ({} more)", vector.len() - 5)
        } else {
            String::new()
        };
        println!("{} [{}{}]", "Vector:".bold(), preview.join(", "), suffix);
    }

    Ok(())
}

fn run_info(args: &EmbedArgs, config: &Config) -> Result<()> {
    let node_id = resolve_embed_node(args.node.as_deref(), config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let meta = node.embedding_metadata();

    println!("{} {}", "Node:".bold(), node_id);
    println!("{} {}", "Provider:".bold(), meta.provider);
    if let Some(model) = &meta.model {
        println!("{} {}", "Model:".bold(), model);
    }
    if let Some(endpoint) = &meta.endpoint {
        println!("{} {}", "Endpoint:".bold(), endpoint);
    }
    if let Some(deployment) = &meta.deployment {
        println!("{} {}", "Deployment:".bold(), deployment);
    }
    if let Some(dimensions) = meta.dimensions {
        println!("{} {}", "Dimensions:".bold(), dimensions);
    }

    Ok(())
}

fn run_azure_vectorizer(args: &EmbedArgs, config: &Config, name: &str) -> Result<()> {
    let node_id = resolve_embed_node(args.node.as_deref(), config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let meta = node.embedding_metadata();
    let vectorizer = meta.to_azure_search_vectorizer(name)?;
    println!("{}", serde_json::to_string_pretty(&vectorizer)?);
    Ok(())
}

fn resolve_embed_node(node_arg: Option<&str>, config: &Config) -> Result<String> {
    if let Some(node_id) = node_arg {
        let (resolved_id, _) = config.get_node(node_id).with_context(|| {
            format!(
                "Embedding node '{}' not found. Run `ailloy ai config` to add it.",
                node_id
            )
        })?;
        return Ok(resolved_id.to_string());
    }

    let (id, _) = config.default_node_for("embedding")?;
    Ok(id.to_string())
}
