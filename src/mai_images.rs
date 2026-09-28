//! Request builders for Microsoft's MAI image models (`MAI-Image-2.x`) on
//! Microsoft Foundry.
//!
//! MAI models are not served on the OpenAI-compatible
//! `/openai/v1/images/*` surface (it answers HTTP 404 "Requested path is not
//! found"); they have their own Microsoft-managed endpoints:
//!
//! - `POST {base}/mai/v1/images/generations` (JSON)
//! - `POST {base}/mai/v1/images/edits` (multipart, reference images in
//!   repeated `image` fields, up to five)
//!
//! The request shape differs from gpt-image: the output size is sent as
//! integer `width`/`height` (a `size` string is silently ignored), the output
//! is always PNG, and each call returns exactly one image (`n` is silently
//! ignored), so callers issue one request per requested image. Responses use
//! the same `data[].b64_json` shape, parsed by
//! [`crate::openai_images::parse_images_response`].
//!
//! Like `openai_images`, this module performs no HTTP itself.

use crate::openai_images::{file_part, guess_mime};
use crate::types::{ImageFormat, ImageOptions};

/// Minimum width and height accepted by MAI image models, in pixels.
const MIN_SIDE: u32 = 768;
/// Maximum `width * height` for MAI-Image-2.6 models (1536x1536).
const MAX_PIXELS_2_6: u64 = 2_359_296;
/// Maximum `width * height` for MAI-Image-2.5 models (1024x1024).
const MAX_PIXELS_2_5: u64 = 1_048_576;
/// Maximum number of reference images per edit request.
const MAX_REFERENCE_IMAGES: usize = 5;

/// Is this Foundry deployment an MAI image model?
///
/// Matches deployment names containing `mai-image` (case-insensitive), which
/// covers the default deployment names (`MAI-Image-2.6-Flash`,
/// `MAI-Image-2.5-Pro`, ...). A custom deployment name that doesn't mention
/// the model can't be detected and goes through the OpenAI-compatible path.
pub(crate) fn is_mai_image_model(name: &str) -> bool {
    name.to_ascii_lowercase().contains("mai-image")
}

/// Generations endpoint for a normalized Foundry base URL. MAI endpoints are
/// unversioned: no `?api-version`, regardless of the node's `api_version`.
pub(crate) fn generations_url(base: &str) -> String {
    format!("{base}/mai/v1/images/generations")
}

/// Edits endpoint for a normalized Foundry base URL; see [`generations_url`].
pub(crate) fn edits_url(base: &str) -> String {
    format!("{base}/mai/v1/images/edits")
}

/// Largest allowed `width * height` for the given model/deployment name.
/// Unknown versions get the 2.6 limit and are left to the service to police.
fn max_pixels(model: &str) -> u64 {
    if model.contains("2.5") {
        MAX_PIXELS_2_5
    } else {
        MAX_PIXELS_2_6
    }
}

/// Number of requests to issue: MAI returns one image per call.
pub(crate) fn request_count(opts: Option<&ImageOptions>) -> u8 {
    opts.and_then(|o| o.n).unwrap_or(1).max(1)
}

/// Validate options for an MAI request, returning an actionable error for
/// anything the MAI API can't honor instead of letting it be silently
/// ignored.
pub(crate) fn validate(model: &str, opts: &ImageOptions) -> anyhow::Result<()> {
    opts.validate()?;

    let mut offending: Vec<&str> = Vec::new();
    if opts.quality.is_some() {
        offending.push("quality");
    }
    if opts.style.is_some() {
        offending.push("style");
    }
    if opts.compression.is_some() {
        offending.push("compression");
    }
    if opts.background.is_some() {
        offending.push("background");
    }
    if opts.moderation.is_some() {
        offending.push("moderation");
    }
    if opts.input_fidelity.is_some() {
        offending.push("input_fidelity");
    }
    if opts.mask.is_some() {
        offending.push("mask");
    }
    if !offending.is_empty() {
        anyhow::bail!(
            "The following parameters are not supported by MAI image models ('{}'): {}. Remove them (check the node's image.* defaults too), or switch to a gpt-image node.",
            model,
            offending.join(", ")
        );
    }

    if let Some(format) = opts.output_format
        && format != ImageFormat::Png
    {
        anyhow::bail!(
            "MAI image models ('{}') only produce PNG output; {} is not supported. Save to a .png file or set the format to png.",
            model,
            format.as_str()
        );
    }

    if let Some((width, height)) = opts.size {
        if width < MIN_SIDE || height < MIN_SIDE {
            anyhow::bail!(
                "Invalid size {}x{} for MAI image model '{}': width and height must each be at least {} pixels.",
                width,
                height,
                model,
                MIN_SIDE
            );
        }
        let limit = max_pixels(model);
        if u64::from(width) * u64::from(height) > limit {
            anyhow::bail!(
                "Invalid size {}x{} for MAI image model '{}': width x height must not exceed {} pixels (e.g. {}).",
                width,
                height,
                model,
                limit,
                if limit == MAX_PIXELS_2_5 {
                    "1024x1024"
                } else {
                    "1536x1536"
                }
            );
        }
    }

    if opts.reference_images.len() > MAX_REFERENCE_IMAGES {
        anyhow::bail!(
            "MAI image models accept at most {} reference images per edit, got {}. Remove some reference images.",
            MAX_REFERENCE_IMAGES,
            opts.reference_images.len()
        );
    }
    for path in &opts.reference_images {
        let mime = guess_mime(path)?;
        if mime != "image/png" && mime != "image/jpeg" {
            anyhow::bail!(
                "MAI image models accept PNG or JPEG reference images only; '{}' is not supported. Convert it to png or jpg.",
                path.display()
            );
        }
    }

    Ok(())
}

/// Build the JSON body for `POST /mai/v1/images/generations`. Call
/// [`validate`] first.
pub(crate) fn build_generations_body(
    model: &str,
    prompt: &str,
    opts: Option<&ImageOptions>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "model": model, "prompt": prompt });
    if let Some((width, height)) = opts.and_then(|o| o.size) {
        body["width"] = serde_json::json!(width);
        body["height"] = serde_json::json!(height);
    }
    body
}

/// Build the multipart form for `POST /mai/v1/images/edits`. Call
/// [`validate`] first.
pub(crate) async fn build_edits_form(
    model: &str,
    prompt: &str,
    opts: &ImageOptions,
) -> anyhow::Result<reqwest::multipart::Form> {
    let mut form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("prompt", prompt.to_string());
    if let Some((width, height)) = opts.size {
        form = form
            .text("width", width.to_string())
            .text("height", height.to_string());
    }
    for path in &opts.reference_images {
        form = form.part("image", file_part(path).await?);
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Background, ImageOptions};

    #[test]
    fn detects_mai_image_deployments() {
        assert!(is_mai_image_model("MAI-Image-2.6-Flash"));
        assert!(is_mai_image_model("MAI-Image-2.5-Pro"));
        assert!(is_mai_image_model("mai-image-2.6"));
        assert!(!is_mai_image_model("gpt-image-2"));
        assert!(!is_mai_image_model("dall-e-3"));
        assert!(!is_mai_image_model("my-custom-deployment"));
    }

    #[test]
    fn urls_use_mai_surface_without_api_version() {
        let base = "https://acct.services.ai.azure.com";
        assert_eq!(
            generations_url(base),
            "https://acct.services.ai.azure.com/mai/v1/images/generations"
        );
        assert_eq!(
            edits_url(base),
            "https://acct.services.ai.azure.com/mai/v1/images/edits"
        );
    }

    #[test]
    fn generations_body_sends_width_and_height_not_size() {
        let opts = ImageOptions::builder().size(1024, 768).build();
        let body = build_generations_body("MAI-Image-2.6-Flash", "an apple", Some(&opts));
        assert_eq!(body["model"], "MAI-Image-2.6-Flash");
        assert_eq!(body["prompt"], "an apple");
        assert_eq!(body["width"], 1024);
        assert_eq!(body["height"], 768);
        assert!(body.get("size").is_none());
        assert!(body.get("n").is_none());
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn generations_body_without_options_is_minimal() {
        let body = build_generations_body("MAI-Image-2.6-Flash", "an apple", None);
        assert_eq!(
            body,
            serde_json::json!({"model": "MAI-Image-2.6-Flash", "prompt": "an apple"})
        );
    }

    #[test]
    fn request_count_follows_n() {
        assert_eq!(request_count(None), 1);
        let opts = ImageOptions::builder().n(3).build();
        assert_eq!(request_count(Some(&opts)), 3);
    }

    #[test]
    fn validate_accepts_plain_and_png_requests() {
        let opts = ImageOptions::builder()
            .size(1536, 1536)
            .output_format(ImageFormat::Png)
            .n(2)
            .build();
        validate("MAI-Image-2.6-Flash", &opts).unwrap();
        validate("MAI-Image-2.6-Flash", &ImageOptions::default()).unwrap();
    }

    #[test]
    fn validate_rejects_gpt_image_only_params_together() {
        let opts = ImageOptions::builder()
            .quality("high")
            .background(Background::Opaque)
            .build();
        let err = validate("MAI-Image-2.6-Flash", &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("quality"), "{err}");
        assert!(err.contains("background"), "{err}");
        assert!(err.contains("MAI"), "{err}");
    }

    #[test]
    fn validate_rejects_non_png_output() {
        let opts = ImageOptions::builder()
            .output_format(ImageFormat::Jpeg)
            .build();
        let err = validate("MAI-Image-2.6-Flash", &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("PNG"), "{err}");
        assert!(err.contains("jpeg"), "{err}");
    }

    #[test]
    fn validate_enforces_minimum_side() {
        let opts = ImageOptions::builder().size(512, 1024).build();
        let err = validate("MAI-Image-2.6-Flash", &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("768"), "{err}");
    }

    #[test]
    fn validate_enforces_pixel_budget_per_version() {
        let big = ImageOptions::builder().size(1536, 1536).build();
        validate("MAI-Image-2.6", &big).unwrap();
        let err = validate("MAI-Image-2.5-Flash", &big)
            .unwrap_err()
            .to_string();
        assert!(err.contains("1048576"), "{err}");

        let too_big = ImageOptions::builder().size(2048, 1536).build();
        let err = validate("MAI-Image-2.6-Flash", &too_big)
            .unwrap_err()
            .to_string();
        assert!(err.contains("2359296"), "{err}");
    }

    #[test]
    fn validate_rejects_too_many_or_webp_references() {
        let mut opts = ImageOptions {
            reference_images: (0..6)
                .map(|i| std::path::PathBuf::from(format!("r{i}.png")))
                .collect(),
            ..Default::default()
        };
        let err = validate("MAI-Image-2.6-Flash", &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("at most 5"), "{err}");

        opts.reference_images = vec![std::path::PathBuf::from("ref.webp")];
        let err = validate("MAI-Image-2.6-Flash", &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("PNG or JPEG"), "{err}");
    }

    #[test]
    fn validate_rejects_mask() {
        let opts = ImageOptions {
            reference_images: vec![std::path::PathBuf::from("ref.png")],
            mask: Some(std::path::PathBuf::from("mask.png")),
            ..Default::default()
        };
        let err = validate("MAI-Image-2.6-Flash", &opts)
            .unwrap_err()
            .to_string();
        assert!(err.contains("mask"), "{err}");
    }
}
