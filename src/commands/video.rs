use std::sync::Mutex;

use anyhow::{Context, Result};
use colored::Colorize;

use std::collections::BTreeMap;

use ailloy::client::{Provider, create_provider_from_node, merge_video_defaults};
use ailloy::config::Config;
use ailloy::types::{VideoJob, VideoJobStatus, VideoOptions, VideoProgress};

use super::image::variant_path;
use super::util::{Spinner, file_hyperlink};
use crate::cli::VideoArgs;

pub async fn run(args: VideoArgs, quiet: bool) -> Result<()> {
    let config = Config::load()?;

    let message = args
        .message
        .as_deref()
        .context("No prompt provided. Use 'ailloy video \"description\"' to generate a video.")?;

    run_direct(&args, &config, message, quiet).await
}

async fn run_direct(args: &VideoArgs, config: &Config, prompt: &str, quiet: bool) -> Result<()> {
    let node_id = resolve_video_node(args.node.as_deref(), config)?;
    let (_, node) = config.get_node(&node_id).unwrap();
    let provider = create_provider_from_node(&node_id, node)?;

    if !quiet {
        eprintln!(
            "{} {} (video generation)",
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

/// Resolve the node to use for video generation from `--node` or the
/// capability default, with an actionable error pointing at `ailloy ai config`
/// and the provider requirement (Azure OpenAI / Microsoft Foundry with a
/// Sora deployment) when nothing is configured.
fn resolve_video_node(node_override: Option<&str>, config: &Config) -> Result<String> {
    if let Some(node_ref) = node_override {
        let (id, _) = config.get_node(node_ref).with_context(|| {
            format!(
                "Node '{}' not found. Run 'ailloy ai config' to configure nodes.",
                node_ref
            )
        })?;
        Ok(id.to_string())
    } else {
        let (id, _) = config.default_node_for("video").with_context(|| {
            "Video generation needs an Azure OpenAI or Microsoft Foundry node with a Sora \
             deployment. Run `ailloy ai config` to add one and set it as the video default \
             (`ailloy ai config set-default <node> --task video`)."
        })?;
        Ok(id.to_string())
    }
}

/// Generate video(s) with the given provider and prompt, then write each
/// variant to disk and print metadata (unless `quiet`).
async fn generate_and_save(
    provider: &dyn Provider,
    prompt: &str,
    args: &VideoArgs,
    node_defaults: Option<&BTreeMap<String, String>>,
    quiet: bool,
) -> Result<()> {
    let mut options = build_video_options(args)?;
    // Fill unset fields from per-node defaults; explicit flags already won.
    if let Some(defaults) = node_defaults {
        merge_video_defaults(&mut options, defaults);
    }

    // Track the last-seen job status so we only print on transitions, not
    // on every poll.
    let last_status: Mutex<Option<VideoJobStatus>> = Mutex::new(None);
    let progress_cb = |job: &VideoJob| {
        if quiet {
            return;
        }
        let mut last = last_status.lock().unwrap();
        if last.as_ref() != Some(&job.status) {
            eprintln!("{} {}", "Status:".dimmed(), job.status.to_string().dimmed());
            *last = Some(job.status.clone());
        }
    };

    let videos = if quiet {
        provider
            .generate_video(prompt, Some(&options), None)
            .await?
    } else {
        let spinner = Spinner::start("Generating video...");
        let progress: VideoProgress = &progress_cb;
        let result = provider
            .generate_video(prompt, Some(&options), Some(progress))
            .await;
        spinner.stop();
        result?
    };

    if videos.is_empty() {
        anyhow::bail!("Provider returned no videos");
    }

    let base_output = args.output.clone().unwrap_or_else(auto_filename);

    for (i, video) in videos.iter().enumerate() {
        let output = variant_path(&base_output, i);
        std::fs::write(&output, &video.data)
            .with_context(|| format!("Failed to write video to: {}", output))?;

        if !quiet {
            eprintln!(
                "{} {} ({}x{}, {}s)",
                "Saved to:".dimmed(),
                file_hyperlink(&output),
                video.width,
                video.height,
                video.duration_seconds,
            );
        }
    }

    if args.raw {
        for i in 0..videos.len() {
            println!("{}", variant_path(&base_output, i));
        }
    }

    Ok(())
}

/// Build [`VideoOptions`] from CLI flags, parsing `--size` and validating
/// the resulting option combination.
pub(crate) fn build_video_options(args: &VideoArgs) -> Result<VideoOptions> {
    let size = args.size.as_deref().map(parse_size).transpose()?;

    let options = VideoOptions {
        size,
        seconds: args.seconds,
        variants: args.variants,
    };

    options.validate()?;

    Ok(options)
}

/// Parse a `WxH` size string (e.g. `"1280x720"`), validating only the format
/// — not whether the API currently supports the given dimensions, since the
/// set of supported sizes may evolve independently of this CLI. Known
/// supported dimensions as of writing: 480x480, 854x480, 480x854, 720x720,
/// 1280x720, 720x1280, 1080x1080, 1920x1080, 1080x1920.
pub(crate) fn parse_size(s: &str) -> Result<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if let [w, h] = parts[..] {
        if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
            if w > 0 && h > 0 {
                return Ok((w, h));
            }
        }
    }
    anyhow::bail!(
        "Invalid video size '{}': expected WxH (e.g. 1280x720). Known supported dimensions: \
         480x480, 854x480, 720x720, 1280x720, 1080x1080, 1920x1080 (and their portrait \
         counterparts) — other sizes may work depending on provider support.",
        s
    )
}

fn auto_filename() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("ailloy-video-{}.mp4", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_size_valid() {
        assert_eq!(parse_size("1280x720").unwrap(), (1280, 720));
        assert_eq!(parse_size("1920x1080").unwrap(), (1920, 1080));
    }

    #[test]
    fn test_parse_size_missing_separator_is_actionable() {
        let err = parse_size("1280").unwrap_err().to_string();
        assert!(err.contains("1280"));
        assert!(err.contains("WxH"));
    }

    #[test]
    fn test_parse_size_non_numeric() {
        let err = parse_size("axb").unwrap_err().to_string();
        assert!(err.contains("axb"));
    }

    #[test]
    fn test_parse_size_missing_height() {
        assert!(parse_size("1280x").is_err());
    }

    #[test]
    fn test_parse_size_zero_rejected() {
        let err = parse_size("0x720").unwrap_err().to_string();
        assert!(err.contains("0x720"));
    }

    #[test]
    fn test_parse_size_empty() {
        assert!(parse_size("").is_err());
    }

    #[test]
    fn test_auto_filename() {
        let name = auto_filename();
        assert!(name.starts_with("ailloy-video-"));
        assert!(name.ends_with(".mp4"));
    }

    #[test]
    fn test_build_video_options_defaults() {
        let args = VideoArgs::default();
        let opts = build_video_options(&args).unwrap();
        assert_eq!(opts.size, None);
        assert_eq!(opts.seconds, None);
        assert_eq!(opts.variants, None);
    }

    #[test]
    fn test_build_video_options_maps_flags() {
        let args = VideoArgs {
            size: Some("1280x720".to_string()),
            seconds: Some(8),
            variants: Some(2),
            ..Default::default()
        };
        let opts = build_video_options(&args).unwrap();
        assert_eq!(opts.size, Some((1280, 720)));
        assert_eq!(opts.seconds, Some(8));
        assert_eq!(opts.variants, Some(2));
    }

    #[test]
    fn test_build_video_options_invalid_size_is_actionable() {
        let args = VideoArgs {
            size: Some("bogus".to_string()),
            ..Default::default()
        };
        let err = build_video_options(&args).unwrap_err().to_string();
        assert!(err.contains("bogus"));
        assert!(err.contains("WxH"));
    }

    #[test]
    fn test_build_video_options_validate_rejects_bad_seconds() {
        let args = VideoArgs {
            seconds: Some(999),
            ..Default::default()
        };
        let err = build_video_options(&args).unwrap_err().to_string();
        assert!(err.contains("seconds"));
    }

    #[test]
    fn test_build_video_options_validate_rejects_bad_variants() {
        let args = VideoArgs {
            variants: Some(0),
            ..Default::default()
        };
        let err = build_video_options(&args).unwrap_err().to_string();
        assert!(err.contains("variants"));
    }
}
