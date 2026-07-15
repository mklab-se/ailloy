//! Library quickstart: video generation via the user's configured video node.
//!
//! Requires a node with the `video` capability (currently Azure OpenAI /
//! Microsoft Foundry with a Sora deployment) set as the default for `video`.
//! Run: cargo run --example video

use ailloy::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Uses ~/.config/ailloy/config.yaml — run `ailloy ai config` once to set up
    // a node with the `video` capability, then `ailloy ai config set-default
    // <node> --task video` (or `defaults.video` in the config file directly).
    let client = Client::for_capability("video")?;

    let videos = client
        .generate_video("A drone shot over a coastal cliff at sunrise")
        .await?;
    let video = videos
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("provider returned no videos"))?;

    let output = "ailloy-video-example.mp4";
    std::fs::write(output, &video.data)?;
    println!(
        "saved {} ({}x{}, {}s)",
        output, video.width, video.height, video.duration_seconds
    );

    Ok(())
}
