//! Shared request/response builder for the OpenAI-family image APIs
//! (OpenAI, Azure OpenAI, Microsoft Foundry).
//!
//! This module builds request bodies/multipart forms and parses responses;
//! it performs no HTTP itself — callers (`openai.rs`, `azure.rs`,
//! `foundry.rs`) own the transport.
//!
//! Consumed by `openai.rs` and `azure.rs` (Tasks 1.5); `foundry.rs` wires in
//! separately (Task 1.6).

use anyhow::Context;
use base64::Engine;
use serde::Deserialize;

use crate::types::{ImageFormat, ImageOptions, ImageResponse, Usage};

/// Which OpenAI-family image API shape to build a request for.
///
/// `AzureGptImage` behaves like `GptImage` except it additionally rejects
/// `webp` output (Azure/Foundry only expose png/jpeg for gpt-image models).
pub(crate) enum ImageFlavor {
    GptImage,
    DallE,
    AzureGptImage,
}

/// Does this request need the multipart edits endpoint (`images/edits`)
/// rather than plain JSON generation (`images/generations`)?
///
/// True whenever reference images are supplied to edit/compose from.
pub(crate) fn wants_edits(opts: Option<&ImageOptions>) -> bool {
    opts.map(|o| !o.reference_images.is_empty())
        .unwrap_or(false)
}

/// Build the JSON body for `POST .../images/generations`.
///
/// Calls [`ImageOptions::validate`] first (generic range/combination rules),
/// then applies flavor-specific rules:
/// - `GptImage`/`AzureGptImage`: sets `output_format`, `output_compression`,
///   `background`, `moderation`, `n`; never sets `response_format`/`style`
///   (an explicit `style` is an error). `AzureGptImage` additionally rejects
///   `output_format: webp`.
/// - `DallE`: sets `response_format: "b64_json"` and optional `style`; `n`
///   is allowed, but any gpt-image-only parameter set explicitly
///   (`output_format`, `compression`, `background`, `moderation`,
///   `input_fidelity`, reference images) is an error.
pub(crate) fn build_generations_body(
    model: Option<&str>,
    prompt: &str,
    opts: Option<&ImageOptions>,
    flavor: ImageFlavor,
) -> anyhow::Result<serde_json::Value> {
    if let Some(o) = opts {
        o.validate()?;
    }

    let mut body = serde_json::json!({ "prompt": prompt });
    if let Some(model) = model {
        body["model"] = serde_json::json!(model);
    }
    if let Some(size) = opts.and_then(|o| o.size) {
        body["size"] = serde_json::json!(format!("{}x{}", size.0, size.1));
    }
    if let Some(quality) = opts.and_then(|o| o.quality.as_deref()) {
        body["quality"] = serde_json::json!(quality);
    }
    if let Some(n) = opts.and_then(|o| o.n) {
        body["n"] = serde_json::json!(n);
    }

    match flavor {
        ImageFlavor::GptImage | ImageFlavor::AzureGptImage => {
            if opts.and_then(|o| o.style.as_deref()).is_some() {
                anyhow::bail!(
                    "style is a DALL·E-only parameter and is not supported by gpt-image models; remove style, or use output_format/background/moderation instead."
                );
            }
            if let Some(format) = opts.and_then(|o| o.output_format) {
                if matches!(flavor, ImageFlavor::AzureGptImage) && format == ImageFormat::Webp {
                    anyhow::bail!(
                        "Azure/Foundry support png and jpeg output only; webp is not supported. Set output_format to png or jpeg."
                    );
                }
                body["output_format"] = serde_json::json!(format.as_str());
            }
            if let Some(compression) = opts.and_then(|o| o.compression) {
                body["output_compression"] = serde_json::json!(compression);
            }
            if let Some(background) = opts.and_then(|o| o.background) {
                body["background"] = serde_json::json!(background.as_str());
            }
            if let Some(moderation) = opts.and_then(|o| o.moderation) {
                body["moderation"] = serde_json::json!(moderation.as_str());
            }
        }
        ImageFlavor::DallE => {
            body["response_format"] = serde_json::json!("b64_json");
            if let Some(style) = opts.and_then(|o| o.style.as_deref()) {
                body["style"] = serde_json::json!(style);
            }
            // Collect ALL explicitly-set gpt-image-only params into one error,
            // so no offending param is masked by another (e.g. compression can
            // only ever be set together with output_format, per validate()).
            let mut offending: Vec<&str> = Vec::new();
            if opts.and_then(|o| o.output_format).is_some() {
                offending.push("output_format");
            }
            if opts.and_then(|o| o.compression).is_some() {
                offending.push("compression");
            }
            if opts.and_then(|o| o.background).is_some() {
                offending.push("background");
            }
            if opts.and_then(|o| o.moderation).is_some() {
                offending.push("moderation");
            }
            if opts.and_then(|o| o.input_fidelity).is_some() {
                offending.push("input_fidelity");
            }
            if opts
                .map(|o| !o.reference_images.is_empty())
                .unwrap_or(false)
            {
                offending.push("reference_images");
            }
            if !offending.is_empty() {
                anyhow::bail!(
                    "The following gpt-image-only parameters are not supported by DALL·E models: {}. Remove them, or switch to a gpt-image model.",
                    offending.join(", ")
                );
            }
        }
    }

    Ok(body)
}

/// Build the multipart form for `POST .../images/edits`.
///
/// Reads each reference image (and the optional mask) from disk via
/// `tokio::fs::read`, guessing the MIME type from the file extension
/// (`png`/`jpg`/`jpeg`/`webp`; anything else is an actionable error).
/// Calls [`ImageOptions::validate`] first, so e.g. a `mask` without
/// `reference_images` fails before any file I/O is attempted.
pub(crate) async fn build_edits_form(
    model: Option<&str>,
    prompt: &str,
    opts: &ImageOptions,
) -> anyhow::Result<reqwest::multipart::Form> {
    opts.validate()?;

    let mut form = reqwest::multipart::Form::new().text("prompt", prompt.to_string());
    if let Some(model) = model {
        form = form.text("model", model.to_string());
    }
    if let Some(size) = opts.size {
        form = form.text("size", format!("{}x{}", size.0, size.1));
    }
    if let Some(quality) = opts.quality.clone() {
        form = form.text("quality", quality);
    }
    if let Some(n) = opts.n {
        form = form.text("n", n.to_string());
    }
    if let Some(format) = opts.output_format {
        form = form.text("output_format", format.as_str());
    }
    if let Some(compression) = opts.compression {
        form = form.text("output_compression", compression.to_string());
    }
    if let Some(background) = opts.background {
        form = form.text("background", background.as_str());
    }
    if let Some(moderation) = opts.moderation {
        form = form.text("moderation", moderation.as_str());
    }
    if let Some(input_fidelity) = opts.input_fidelity {
        form = form.text("input_fidelity", input_fidelity.as_str());
    }

    for path in &opts.reference_images {
        form = form.part("image[]", file_part(path).await?);
    }
    if let Some(mask) = &opts.mask {
        form = form.part("mask", file_part(mask).await?);
    }

    Ok(form)
}

/// Read a reference/mask image from disk into a multipart form part, with
/// the file name preserved and MIME type guessed from its extension.
async fn file_part(path: &std::path::Path) -> anyhow::Result<reqwest::multipart::Part> {
    let bytes = tokio::fs::read(path).await.with_context(|| {
        format!(
            "Failed to read image file '{}': check that the path exists and is readable.",
            path.display()
        )
    })?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());
    let mime = guess_mime(path)?;

    reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name)
        .mime_str(mime)
        .with_context(|| format!("Invalid MIME type '{}' for image part", mime))
}

/// Guess a part's MIME type from its file extension.
fn guess_mime(path: &std::path::Path) -> anyhow::Result<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        other => anyhow::bail!(
            "Unsupported image file extension '{}' for '{}'; supported types: png, jpg, jpeg, webp.",
            other,
            path.display()
        ),
    }
}

#[derive(Deserialize)]
struct ImagesApiResponse {
    data: Vec<ImagesApiData>,
    usage: Option<ImagesApiUsage>,
}

#[derive(Deserialize)]
struct ImagesApiData {
    b64_json: Option<String>,
    revised_prompt: Option<String>,
}

#[derive(Deserialize)]
struct ImagesApiUsage {
    input_tokens: u32,
    output_tokens: u32,
    total_tokens: u32,
}

/// Parse an `images/generations` or `images/edits` JSON response body into
/// one [`ImageResponse`] per `data[]` entry.
///
/// Each image is base64-decoded, its dimensions read from magic bytes (via
/// [`crate::types::image_dimensions`]) falling back to `fallback_size` and
/// then `(1024, 1024)`, and its format detected from magic bytes (falling
/// back to `Png` when undetectable). `usage`, when present, is mapped
/// (`input_tokens` -> `prompt_tokens`, `output_tokens` -> `completion_tokens`)
/// and attached to the *first* image only, to avoid double-counting tokens
/// across a multi-image response.
pub(crate) fn parse_images_response(
    body: &str,
    fallback_size: Option<(u32, u32)>,
) -> anyhow::Result<Vec<ImageResponse>> {
    let parsed: ImagesApiResponse =
        serde_json::from_str(body).context("Failed to parse image generation response")?;

    let usage = parsed.usage.map(|u| Usage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
    });

    let mut images = Vec::with_capacity(parsed.data.len());
    for (i, item) in parsed.data.into_iter().enumerate() {
        let b64 = item
            .b64_json
            .as_deref()
            .context("No base64 image data in response")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .context("Failed to decode base64 image data")?;

        let format = detect_format(&data);
        let (width, height) = crate::types::image_dimensions(&data)
            .or(fallback_size)
            .unwrap_or((1024, 1024));

        images.push(ImageResponse {
            data,
            width,
            height,
            format,
            revised_prompt: item.revised_prompt,
            usage: if i == 0 { usage.clone() } else { None },
        });
    }

    Ok(images)
}

/// Detect an image format from its magic bytes, falling back to `Png` when
/// the data doesn't match a known signature.
fn detect_format(data: &[u8]) -> ImageFormat {
    if data.len() >= 8 && data[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return ImageFormat::Png;
    }
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        return ImageFormat::Jpeg;
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return ImageFormat::Webp;
    }
    ImageFormat::Png
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Background, ImageFormat, ImageOptions, InputFidelity, Moderation};

    // --- build_generations_body: gpt-image flavor ---

    #[test]
    fn gpt_image_sets_expected_fields_never_response_format_or_style() {
        let opts = ImageOptions::builder()
            .size(1024, 1024)
            .quality("high")
            .output_format(ImageFormat::Jpeg)
            .compression(80)
            .n(2)
            .background(Background::Opaque)
            .moderation(Moderation::Low)
            .build();
        let body = build_generations_body(
            Some("gpt-image-1"),
            "a cat",
            Some(&opts),
            ImageFlavor::GptImage,
        )
        .unwrap();
        assert_eq!(body["model"], "gpt-image-1");
        assert_eq!(body["prompt"], "a cat");
        assert_eq!(body["size"], "1024x1024");
        assert_eq!(body["quality"], "high");
        assert_eq!(body["output_format"], "jpeg");
        assert_eq!(body["output_compression"], 80);
        assert_eq!(body["n"], 2);
        assert_eq!(body["background"], "opaque");
        assert_eq!(body["moderation"], "low");
        assert!(body.get("response_format").is_none());
        assert!(body.get("style").is_none());
    }

    #[test]
    fn gpt_image_style_set_errors() {
        #[allow(deprecated)]
        let opts = ImageOptions::builder().style("natural").build();
        let err = build_generations_body(
            Some("gpt-image-1"),
            "a cat",
            Some(&opts),
            ImageFlavor::GptImage,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("style"));
        assert!(err.contains("gpt-image") || err.contains("not supported"));
    }

    #[test]
    fn azure_gpt_image_webp_errors() {
        let opts = ImageOptions::builder()
            .output_format(ImageFormat::Webp)
            .build();
        let err = build_generations_body(
            Some("my-deployment"),
            "a cat",
            Some(&opts),
            ImageFlavor::AzureGptImage,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("png"));
        assert!(err.contains("jpeg"));
    }

    #[test]
    fn azure_gpt_image_png_ok() {
        let opts = ImageOptions::builder()
            .output_format(ImageFormat::Png)
            .build();
        let body = build_generations_body(
            Some("my-deployment"),
            "a cat",
            Some(&opts),
            ImageFlavor::AzureGptImage,
        )
        .unwrap();
        assert_eq!(body["output_format"], "png");
    }

    #[test]
    fn gpt_image_no_opts_minimal_body() {
        let body =
            build_generations_body(Some("gpt-image-1"), "a cat", None, ImageFlavor::GptImage)
                .unwrap();
        assert_eq!(body["prompt"], "a cat");
        assert_eq!(body["model"], "gpt-image-1");
        assert!(body.get("size").is_none());
        assert!(body.get("output_format").is_none());
    }

    #[test]
    fn model_is_optional() {
        let body = build_generations_body(None, "a cat", None, ImageFlavor::GptImage).unwrap();
        assert!(body.get("model").is_none());
        assert_eq!(body["prompt"], "a cat");
    }

    // --- build_generations_body: DALL·E flavor ---

    #[test]
    fn dalle_sets_response_format_and_style() {
        #[allow(deprecated)]
        let opts = ImageOptions::builder().style("vivid").n(1).build();
        let body =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap();
        assert_eq!(body["response_format"], "b64_json");
        assert_eq!(body["style"], "vivid");
        assert_eq!(body["n"], 1);
    }

    #[test]
    fn dalle_allows_n_without_error() {
        let opts = ImageOptions::builder().n(4).build();
        let body =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap();
        assert_eq!(body["n"], 4);
    }

    #[test]
    fn dalle_rejects_output_format() {
        let opts = ImageOptions::builder()
            .output_format(ImageFormat::Png)
            .build();
        let err =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        assert!(err.contains("output_format"));
        assert!(err.contains("DALL"));
    }

    #[test]
    fn dalle_rejects_compression() {
        // compression can only be set together with output_format (validate()
        // enforces jpeg/webp), so the error must name BOTH offending params —
        // compression must not be masked by output_format.
        let opts = ImageOptions::builder()
            .output_format(ImageFormat::Jpeg)
            .compression(50)
            .build();
        let err =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        assert!(
            err.contains("compression"),
            "error must name compression: {err}"
        );
        assert!(
            err.contains("output_format"),
            "error must name output_format: {err}"
        );
        assert!(err.contains("DALL"));
    }

    #[test]
    fn dalle_rejects_background() {
        let opts = ImageOptions::builder()
            .background(Background::Opaque)
            .build();
        let err =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        assert!(err.contains("background"));
        assert!(err.contains("DALL"));
    }

    #[test]
    fn dalle_rejects_moderation() {
        let opts = ImageOptions::builder().moderation(Moderation::Auto).build();
        let err =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        assert!(err.contains("moderation"));
        assert!(err.contains("DALL"));
    }

    #[test]
    fn dalle_rejects_input_fidelity() {
        let opts = ImageOptions::builder()
            .input_fidelity(InputFidelity::High)
            .reference_image(std::path::PathBuf::from("ref.png"))
            .build();
        let err =
            build_generations_body(Some("dall-e-2"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        assert!(err.contains("input_fidelity"));
        assert!(err.contains("DALL"));
    }

    #[test]
    fn dalle_rejects_reference_images() {
        let opts = ImageOptions::builder()
            .reference_image(std::path::PathBuf::from("ref.png"))
            .build();
        let err =
            build_generations_body(Some("dall-e-2"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        assert!(
            err.contains("reference_images"),
            "error must name reference_images: {err}"
        );
        assert!(err.contains("DALL"));
    }

    #[test]
    fn dalle_lists_all_offending_params_in_one_error() {
        let opts = ImageOptions::builder()
            .output_format(ImageFormat::Webp)
            .compression(30)
            .background(Background::Auto)
            .moderation(Moderation::Low)
            .build();
        let err =
            build_generations_body(Some("dall-e-3"), "a cat", Some(&opts), ImageFlavor::DallE)
                .unwrap_err()
                .to_string();
        for param in ["output_format", "compression", "background", "moderation"] {
            assert!(err.contains(param), "error must name {param}: {err}");
        }
        assert!(err.contains("DALL"));
    }

    #[test]
    fn generic_validation_runs_before_flavor_rules() {
        // compression without output_format is a generic ImageOptions::validate()
        // error, and must surface even for a flavor that would otherwise accept
        // compression (GptImage).
        let opts = ImageOptions::builder().compression(50).build();
        let err = build_generations_body(
            Some("gpt-image-1"),
            "a cat",
            Some(&opts),
            ImageFlavor::GptImage,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("compression"));
        assert!(err.contains("output_format"));
    }

    // --- wants_edits ---

    #[test]
    fn wants_edits_true_when_reference_images_present() {
        let opts = ImageOptions::builder()
            .reference_image(std::path::PathBuf::from("ref.png"))
            .build();
        assert!(wants_edits(Some(&opts)));
    }

    #[test]
    fn wants_edits_false_when_empty_or_none() {
        assert!(!wants_edits(None));
        assert!(!wants_edits(Some(&ImageOptions::default())));
    }

    // --- parse_images_response ---

    fn minimal_png_bytes() -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&32u32.to_be_bytes());
        png
    }

    #[test]
    fn parse_single_image_with_usage() {
        let png = minimal_png_bytes();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        let body = format!(
            r#"{{"created":1,"data":[{{"b64_json":"{b64}","revised_prompt":"a fluffy cat"}}],"usage":{{"input_tokens":5,"input_tokens_details":{{"text_tokens":5,"image_tokens":0}},"output_tokens":10,"total_tokens":15}}}}"#
        );
        let images = parse_images_response(&body, None).unwrap();
        assert_eq!(images.len(), 1);
        let img = &images[0];
        assert_eq!(img.width, 64);
        assert_eq!(img.height, 32);
        assert_eq!(img.format, ImageFormat::Png);
        assert_eq!(img.revised_prompt.as_deref(), Some("a fluffy cat"));
        let usage = img.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.completion_tokens, 10);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn parse_multi_image_usage_only_on_first() {
        let png = minimal_png_bytes();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        let body = format!(
            r#"{{"created":1,"data":[{{"b64_json":"{b64}"}},{{"b64_json":"{b64}"}}],"usage":{{"input_tokens":5,"output_tokens":10,"total_tokens":15}}}}"#
        );
        let images = parse_images_response(&body, None).unwrap();
        assert_eq!(images.len(), 2);
        assert!(images[0].usage.is_some());
        assert!(images[1].usage.is_none());
    }

    #[test]
    fn parse_no_usage_is_none() {
        let png = minimal_png_bytes();
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        let body = format!(r#"{{"created":1,"data":[{{"b64_json":"{b64}"}}]}}"#);
        let images = parse_images_response(&body, None).unwrap();
        assert!(images[0].usage.is_none());
    }

    #[test]
    fn parse_dimension_fallback_when_undetectable() {
        // Not valid image magic bytes, so image_dimensions() returns None.
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"not-an-image");
        let body = format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#);
        let images = parse_images_response(&body, Some((640, 480))).unwrap();
        assert_eq!(images[0].width, 640);
        assert_eq!(images[0].height, 480);
    }

    #[test]
    fn parse_dimension_fallback_default_1024() {
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"not-an-image");
        let body = format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#);
        let images = parse_images_response(&body, None).unwrap();
        assert_eq!(images[0].width, 1024);
        assert_eq!(images[0].height, 1024);
    }

    #[test]
    fn parse_format_detection_jpeg() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.extend_from_slice(&[0; 20]);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg);
        let body = format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#);
        let images = parse_images_response(&body, None).unwrap();
        assert_eq!(images[0].format, ImageFormat::Jpeg);
    }

    #[test]
    fn parse_format_detection_webp() {
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(&[0; 16]);
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &webp);
        let body = format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#);
        let images = parse_images_response(&body, None).unwrap();
        assert_eq!(images[0].format, ImageFormat::Webp);
    }

    #[test]
    fn parse_format_detection_fallback_png() {
        let b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"not-an-image");
        let body = format!(r#"{{"data":[{{"b64_json":"{b64}"}}]}}"#);
        let images = parse_images_response(&body, None).unwrap();
        assert_eq!(images[0].format, ImageFormat::Png);
    }

    #[test]
    fn parse_missing_b64_errors() {
        let body = r#"{"data":[{"revised_prompt":"x"}]}"#;
        let err = parse_images_response(body, None).unwrap_err().to_string();
        assert!(!err.is_empty());
    }

    #[test]
    fn parse_invalid_json_errors() {
        let err = parse_images_response("not json", None)
            .unwrap_err()
            .to_string();
        assert!(!err.is_empty());
    }

    // --- build_edits_form ---

    #[tokio::test]
    async fn edits_form_builds_from_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let ref_path = dir.path().join("ref.png");
        tokio::fs::write(&ref_path, minimal_png_bytes())
            .await
            .unwrap();
        let mask_path = dir.path().join("mask.png");
        tokio::fs::write(&mask_path, minimal_png_bytes())
            .await
            .unwrap();

        let opts = ImageOptions::builder()
            .reference_image(ref_path)
            .mask(mask_path)
            .input_fidelity(InputFidelity::High)
            .build();

        let result = build_edits_form(Some("gpt-image-1"), "add a hat", &opts).await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
    }

    #[tokio::test]
    async fn edits_form_missing_file_errors() {
        let opts = ImageOptions::builder()
            .reference_image(std::path::PathBuf::from("/nonexistent/path/ref.png"))
            .build();

        let err = build_edits_form(Some("gpt-image-1"), "add a hat", &opts)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("ref.png"));
    }

    #[tokio::test]
    async fn edits_form_unsupported_extension_errors() {
        let dir = tempfile::tempdir().unwrap();
        let ref_path = dir.path().join("ref.bmp");
        tokio::fs::write(&ref_path, minimal_png_bytes())
            .await
            .unwrap();

        let opts = ImageOptions::builder().reference_image(ref_path).build();
        let err = build_edits_form(Some("gpt-image-1"), "add a hat", &opts)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("png"));
        assert!(err.contains("jpeg") || err.contains("jpg"));
        assert!(err.contains("webp"));
    }

    #[tokio::test]
    async fn edits_form_validates_opts_first() {
        // mask without reference images is a generic ImageOptions::validate()
        // error and must surface before any file I/O is attempted.
        let opts = ImageOptions::builder()
            .mask(std::path::PathBuf::from("/nonexistent/mask.png"))
            .build();
        let err = build_edits_form(Some("gpt-image-1"), "add a hat", &opts)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("mask"));
        assert!(err.contains("reference"));
    }
}
