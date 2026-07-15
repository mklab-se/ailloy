//! Library quickstart: chat via the user's configured default node.
//!
//! Run: cargo run --example chat

use ailloy::{ChatOptions, Client, Message};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Uses ~/.config/ailloy/config.yaml — run `ailloy ai config` once to set up.
    let client = Client::from_config()?;

    // Plain chat
    let response = client
        .chat(&[Message::user("Say hello in Swedish, one sentence.")])
        .await?;
    println!("{} (model: {})", response.content, response.model);

    // Structured JSON output
    let options = ChatOptions::builder()
        .json_schema(
            "cities",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "cities": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["cities"],
                "additionalProperties": false
            }),
        )
        .build();
    let response = client
        .chat_with(
            &[Message::user("List the three largest cities in Sweden.")],
            &options,
        )
        .await?;
    let parsed: serde_json::Value = serde_json::from_str(&response.content)?;
    println!("cities: {}", parsed["cities"]);

    if let Some(usage) = response.usage {
        println!(
            "tokens: {} in / {} out",
            usage.prompt_tokens, usage.completion_tokens
        );
    }

    // Attachments: only runs if an image/pdf/text path is passed as the first
    // CLI argument, so this example stays safe to run with no arguments.
    //
    //   cargo run --example chat -- path/to/screenshot.png
    if let Some(path) = std::env::args().nth(1) {
        let msg = Message::user_with_attachments(
            "What's in this file?",
            &[std::path::PathBuf::from(path)],
        )?;
        let response = client.chat(&[msg]).await?;
        println!("attachment response: {}", response.content);
    }

    Ok(())
}
