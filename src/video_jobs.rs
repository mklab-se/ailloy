//! Shared Azure/Foundry video (sora-2) request logic — OpenAI-style Videos API.
//!
//! Both `AzureOpenAiClient` and `FoundryClient` drive Sora video generation
//! through the OpenAI-compatible **Videos** surface:
//!
//! - `POST   {base}/openai/v1/videos`             — create a video
//! - `GET    {base}/openai/v1/videos/{id}`        — poll a video object
//! - `GET    {base}/openai/v1/videos/{id}/content`— download raw `video/mp4`
//! - `DELETE {base}/openai/v1/videos/{id}`        — delete a video
//!
//! (The legacy `.../video/generations/jobs?api-version=preview` surface is a
//! 404 on current Foundry/Azure resources — this module targets the working
//! `/videos` endpoints instead.) Per the crate's v1-vs-dated convention, the
//! `?api-version=` query is **omitted** on the unified v1 surface and appended
//! only for nodes configured with an explicit dated `api_version`.
//!
//! ## Wire shape
//!
//! Create requests send `{model, prompt, size:"WxH", seconds:"<string>"}` —
//! note that `seconds` is a **string** on the wire (documented values `"4"`,
//! `"8"`, `"12"`). A video object comes back as
//! `{id:"video_…", object:"video", status, size, seconds, error, …}` with
//! `status` in `queued | in_progress | completed | failed`, and `error`
//! holding `{code, message}` on failure. Artifacts expire ~24h after
//! completion.
//!
//! ## The `+` multi-id convention
//!
//! The Videos API has no `n_variants` field: one create call yields one
//! video. To keep the public [`VideoJob`] job-oriented API intact while
//! supporting `--variants > 1`, [`VideoJobsApi::create`] issues *N* create
//! POSTs and returns a single [`VideoJob`] whose `id` is the individual video
//! ids **joined with `+`** (e.g. `video_a+video_b`); `generation_ids` holds
//! the same ids as a list. [`VideoJobsApi::get`] and [`VideoJobsApi::delete`]
//! split the composite id on `+` and fan out per video, aggregating status
//! (all completed → Succeeded; any failed → Failed with that video's error;
//! otherwise Running/Queued). A single-variant id contains no `+`, so callers
//! that only ever use the plain API see ordinary OpenAI `video_…` ids. This
//! convention is user-visible through the low-level `get_video_job` /
//! `delete_video_job` API.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{VideoJob, VideoJobStatus, VideoOptions, VideoResponse};

/// Separator used to join per-video ids into a single composite job id when a
/// create call produces multiple variants. See the module docs.
const MULTI_ID_SEP: char = '+';

/// Shared client for the Azure/Foundry OpenAI-style Videos API.
pub(crate) struct VideoJobsApi<'a> {
    pub client: &'a reqwest::Client,
    pub base: String,
    pub api_version: Option<&'a str>,
    pub header: (&'static str, String),
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateVideoRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    /// `"WxH"` — e.g. `"720x1280"`.
    size: String,
    /// Clip duration in seconds, serialized as a **string** (`"4"`, `"8"`, …).
    seconds: String,
}

/// Build the create-video request body, applying defaults (`720x1280`, `"4"`s)
/// for any option left unset. `seconds` is emitted as a string per the wire
/// contract.
fn build_create_body<'a>(
    model: &'a str,
    prompt: &'a str,
    opts: Option<&VideoOptions>,
) -> CreateVideoRequest<'a> {
    let (width, height) = opts.and_then(|o| o.size).unwrap_or((720, 1280));
    let seconds = opts
        .and_then(|o| o.seconds)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "4".to_string());
    CreateVideoRequest {
        model,
        prompt,
        size: format!("{}x{}", width, height),
        seconds,
    }
}

/// A single video object as returned by `POST`/`GET .../videos[/{id}]`.
#[derive(Deserialize)]
struct VideoObjectWire {
    id: String,
    status: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    seconds: Option<String>,
    #[serde(default)]
    error: Option<VideoErrorWire>,
}

/// Error detail on a failed video object.
#[derive(Deserialize)]
struct VideoErrorWire {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl VideoErrorWire {
    /// Best available human-readable message: prefer `message`, fall back to
    /// `code`, then a generic string.
    fn describe(&self) -> String {
        self.message
            .clone()
            .or_else(|| self.code.clone())
            .unwrap_or_else(|| "unknown error".to_string())
    }
}

/// Parse a single video object JSON body.
fn parse_video_object(text: &str) -> Result<VideoObjectWire> {
    serde_json::from_str(text).with_context(|| {
        format!(
            "Failed to parse Azure/Foundry video object response: {}",
            text
        )
    })
}

/// Parse `"WxH"` into `(width, height)`, defaulting to `(0, 0)` when absent or
/// malformed.
fn parse_size_field(size: Option<&str>) -> (u32, u32) {
    size.and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
    })
    .unwrap_or((0, 0))
}

/// Aggregate per-video statuses into a single job status: any failed → Failed;
/// else any cancelled → Cancelled; else all succeeded → Succeeded; else any
/// in-progress → Running; else Queued.
fn aggregate_status(statuses: &[VideoJobStatus]) -> VideoJobStatus {
    if statuses.contains(&VideoJobStatus::Failed) {
        VideoJobStatus::Failed
    } else if statuses.contains(&VideoJobStatus::Cancelled) {
        VideoJobStatus::Cancelled
    } else if !statuses.is_empty() && statuses.iter().all(|s| *s == VideoJobStatus::Succeeded) {
        VideoJobStatus::Succeeded
    } else if statuses.iter().any(|s| {
        matches!(
            s,
            VideoJobStatus::Running | VideoJobStatus::Processing | VideoJobStatus::Preprocessing
        )
    }) {
        VideoJobStatus::Running
    } else {
        VideoJobStatus::Queued
    }
}

/// Assemble a [`VideoJob`] from the fetched/created video objects, using
/// `composite_id` as the job id (the `+`-joined multi-id).
fn assemble_job(composite_id: String, objects: &[VideoObjectWire]) -> Result<VideoJob> {
    let mut statuses = Vec::with_capacity(objects.len());
    let mut failure_reason = None;
    for obj in objects {
        let status = obj.status.parse::<VideoJobStatus>().map_err(|e| {
            anyhow::anyhow!(
                "Azure/Foundry Videos API returned an unrecognized status for video '{}': {}",
                obj.id,
                e
            )
        })?;
        if status == VideoJobStatus::Failed && failure_reason.is_none() {
            let msg = obj
                .error
                .as_ref()
                .map(VideoErrorWire::describe)
                .unwrap_or_else(|| "unknown error".to_string());
            failure_reason = Some(format!("video '{}' failed: {}", obj.id, msg));
        }
        statuses.push(status);
    }
    Ok(VideoJob {
        id: composite_id,
        status: aggregate_status(&statuses),
        generation_ids: objects.iter().map(|o| o.id.clone()).collect(),
        failure_reason,
    })
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

/// Actionable error for a non-2xx response from the Videos API that isn't a
/// 404 (which gets the more specific expiry-aware message below).
fn video_api_error(status: u16, body: &str, action: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Azure/Foundry Videos API: failed to {} (HTTP {}): {}",
        action,
        status,
        body
    )
}

/// 404 on a video/content lookup: artifacts expire ~24h after completion, so
/// the actionable next step is to re-create the video.
fn expired_artifact_error(kind: &str, id: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Azure/Foundry Videos API: {} '{}' not found (HTTP 404). \
         Video artifacts expire ~24 hours after completion; re-create the video to get a fresh one.",
        kind,
        id
    )
}

/// Split a composite job id (`+`-joined) into its constituent video ids.
fn split_ids(id: &str) -> Vec<&str> {
    id.split(MULTI_ID_SEP).filter(|s| !s.is_empty()).collect()
}

impl VideoJobsApi<'_> {
    /// `?api-version=…` query suffix — empty on the v1 surface, present only
    /// for nodes with an explicit dated api-version.
    fn query_suffix(&self) -> String {
        match self.api_version {
            Some(v) => format!("?api-version={}", v),
            None => String::new(),
        }
    }

    pub(crate) fn videos_url(&self) -> String {
        format!("{}/openai/v1/videos{}", self.base, self.query_suffix())
    }

    pub(crate) fn video_url(&self, id: &str) -> String {
        format!(
            "{}/openai/v1/videos/{}{}",
            self.base,
            id,
            self.query_suffix()
        )
    }

    pub(crate) fn content_url(&self, id: &str) -> String {
        format!(
            "{}/openai/v1/videos/{}/content{}",
            self.base,
            id,
            self.query_suffix()
        )
    }

    /// POST a single create-video request and parse the resulting video
    /// object.
    async fn create_one(
        &self,
        model: &str,
        prompt: &str,
        opts: Option<&VideoOptions>,
    ) -> Result<VideoObjectWire> {
        let body = build_create_body(model, prompt, opts);

        let response = self
            .client
            .post(self.videos_url())
            .header(self.header.0, &self.header.1)
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry Videos API (create video)")?;

        let status = response.status();
        let text = response
            .text()
            .await
            .context("Failed to read Azure/Foundry Videos API create response")?;

        if !status.is_success() {
            return Err(video_api_error(status.as_u16(), &text, "create video"));
        }

        parse_video_object(&text)
    }

    /// Create video(s) from a prompt. With `opts.variants > 1`, issues one
    /// create POST per variant and returns a single [`VideoJob`] whose id is
    /// the per-video ids joined with `+` (see module docs).
    pub async fn create(
        &self,
        model: &str,
        prompt: &str,
        opts: Option<&VideoOptions>,
    ) -> Result<VideoJob> {
        let variants = opts.and_then(|o| o.variants).unwrap_or(1).max(1);

        let mut objects = Vec::with_capacity(variants as usize);
        for _ in 0..variants {
            objects.push(self.create_one(model, prompt, opts).await?);
        }

        let composite_id = objects
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>()
            .join(&MULTI_ID_SEP.to_string());

        assemble_job(composite_id, &objects)
    }

    /// GET a single video object.
    async fn get_one(&self, id: &str) -> Result<VideoObjectWire> {
        let response = self
            .client
            .get(self.video_url(id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry Videos API (get video)")?;

        let status = response.status();
        let text = response
            .text()
            .await
            .context("Failed to read Azure/Foundry Videos API get response")?;

        if status.as_u16() == 404 {
            return Err(expired_artifact_error("video", id));
        }
        if !status.is_success() {
            return Err(video_api_error(status.as_u16(), &text, "get video"));
        }

        parse_video_object(&text)
    }

    /// Fetch the current state of a video job. Splits the composite id on `+`,
    /// GETs each video, and aggregates them into a single [`VideoJob`].
    pub async fn get(&self, id: &str) -> Result<VideoJob> {
        let ids = split_ids(id);
        let mut objects = Vec::with_capacity(ids.len());
        for vid in ids {
            objects.push(self.get_one(vid).await?);
        }
        assemble_job(id.to_string(), &objects)
    }

    /// Download a completed video: fetch the video object for size/seconds,
    /// then fetch the raw content bytes, and assemble a [`VideoResponse`].
    /// `generation_id` is a single video id (never a `+`-joined composite) —
    /// rejected before any HTTP call if it contains `+`.
    pub async fn download(&self, generation_id: &str) -> Result<VideoResponse> {
        if generation_id.contains(MULTI_ID_SEP) {
            anyhow::bail!(
                "'{}' looks like a composite multi-variant job id — pass one of \
                 VideoJob::generation_ids instead",
                generation_id
            );
        }

        let meta = self.get_one(generation_id).await?;
        let (width, height) = parse_size_field(meta.size.as_deref());
        let duration_seconds = meta
            .seconds
            .as_deref()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|s| s.round() as u32)
            .unwrap_or(0);

        let content_response = self
            .client
            .get(self.content_url(generation_id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry Videos API (download content)")?;

        let content_status = content_response.status();
        if content_status.as_u16() == 404 {
            return Err(expired_artifact_error("video content", generation_id));
        }
        if !content_status.is_success() {
            let body = content_response.text().await.unwrap_or_default();
            return Err(video_api_error(
                content_status.as_u16(),
                &body,
                "download video content",
            ));
        }

        let data = content_response
            .bytes()
            .await
            .context("Failed to read Azure/Foundry video content bytes")?
            .to_vec();

        Ok(VideoResponse {
            data,
            width,
            height,
            duration_seconds,
        })
    }

    /// DELETE a single video.
    async fn delete_one(&self, id: &str) -> Result<()> {
        let response = self
            .client
            .delete(self.video_url(id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry Videos API (delete video)")?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(expired_artifact_error("video", id));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(video_api_error(status.as_u16(), &body, "delete video"));
        }

        Ok(())
    }

    /// Delete a video job and its artifacts. Splits the composite id on `+`
    /// and deletes each constituent video.
    pub async fn delete(&self, id: &str) -> Result<()> {
        for vid in split_ids(id) {
            self.delete_one(vid).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api<'a>(
        base: &str,
        api_version: Option<&'a str>,
        client: &'a reqwest::Client,
    ) -> VideoJobsApi<'a> {
        VideoJobsApi {
            client,
            base: base.to_string(),
            api_version,
            header: ("Authorization", "Bearer test".to_string()),
        }
    }

    // --- URL builders ---

    #[test]
    fn v1_urls_omit_api_version() {
        let client = reqwest::Client::new();
        let a = api("https://r.example.com", None, &client);
        assert_eq!(a.videos_url(), "https://r.example.com/openai/v1/videos");
        assert_eq!(
            a.video_url("video_1"),
            "https://r.example.com/openai/v1/videos/video_1"
        );
        assert_eq!(
            a.content_url("video_1"),
            "https://r.example.com/openai/v1/videos/video_1/content"
        );
    }

    #[test]
    fn dated_urls_append_explicit_api_version() {
        let client = reqwest::Client::new();
        let a = api("https://r.example.com", Some("2025-04-01-preview"), &client);
        assert_eq!(
            a.videos_url(),
            "https://r.example.com/openai/v1/videos?api-version=2025-04-01-preview"
        );
        assert_eq!(
            a.video_url("video_1"),
            "https://r.example.com/openai/v1/videos/video_1?api-version=2025-04-01-preview"
        );
        assert_eq!(
            a.content_url("video_1"),
            "https://r.example.com/openai/v1/videos/video_1/content?api-version=2025-04-01-preview"
        );
    }

    // --- create-body JSON ---

    #[test]
    fn create_body_defaults() {
        let body = build_create_body("my-deployment", "a cat riding a bike", None);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "my-deployment");
        assert_eq!(json["prompt"], "a cat riding a bike");
        assert_eq!(json["size"], "720x1280");
        // seconds is serialized as a STRING on the wire.
        assert_eq!(json["seconds"], "4");
        assert!(json["seconds"].is_string());
        // No n_variants field on the Videos surface.
        assert!(json.get("n_variants").is_none());
    }

    #[test]
    fn create_body_explicit_options_mapping() {
        let opts = VideoOptions {
            size: Some((1280, 720)),
            seconds: Some(12),
            variants: Some(3),
        };
        let body = build_create_body("dep", "prompt", Some(&opts));
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["size"], "1280x720");
        assert_eq!(json["seconds"], "12");
        assert!(json["seconds"].is_string());
        // variants does not appear in the per-create body.
        assert!(json.get("variants").is_none());
    }

    #[test]
    fn create_body_partial_options_fill_remaining_defaults() {
        let opts = VideoOptions {
            size: None,
            seconds: Some(8),
            variants: None,
        };
        let body = build_create_body("dep", "prompt", Some(&opts));
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["size"], "720x1280");
        assert_eq!(json["seconds"], "8");
    }

    // --- multi-id join / split ---

    #[test]
    fn split_ids_single_has_no_separator() {
        assert_eq!(split_ids("video_abc"), vec!["video_abc"]);
    }

    #[test]
    fn split_ids_multi_round_trips() {
        assert_eq!(
            split_ids("video_a+video_b+video_c"),
            vec!["video_a", "video_b", "video_c"]
        );
    }

    #[test]
    fn split_ids_ignores_empty_segments() {
        assert_eq!(split_ids("video_a++video_b"), vec!["video_a", "video_b"]);
        assert!(split_ids("").is_empty());
    }

    // --- status aggregation matrix ---

    fn obj(id: &str, status: &str) -> VideoObjectWire {
        VideoObjectWire {
            id: id.to_string(),
            status: status.to_string(),
            size: None,
            seconds: None,
            error: None,
        }
    }

    #[test]
    fn aggregate_all_completed_is_succeeded() {
        let objs = [obj("a", "completed"), obj("b", "completed")];
        let job = assemble_job("a+b".to_string(), &objs).unwrap();
        assert_eq!(job.id, "a+b");
        assert_eq!(job.status, VideoJobStatus::Succeeded);
        assert_eq!(job.generation_ids, vec!["a".to_string(), "b".to_string()]);
        assert!(job.failure_reason.is_none());
    }

    #[test]
    fn aggregate_one_failed_is_failed_with_reason_and_id() {
        let objs = [
            obj("a", "completed"),
            VideoObjectWire {
                id: "b".to_string(),
                status: "failed".to_string(),
                size: None,
                seconds: None,
                error: Some(VideoErrorWire {
                    code: Some("content_policy".to_string()),
                    message: Some("blocked by content policy".to_string()),
                }),
            },
        ];
        let job = assemble_job("a+b".to_string(), &objs).unwrap();
        assert_eq!(job.status, VideoJobStatus::Failed);
        let reason = job.failure_reason.unwrap();
        assert!(reason.contains("b"), "reason: {}", reason);
        assert!(
            reason.contains("blocked by content policy"),
            "reason: {}",
            reason
        );
    }

    #[test]
    fn aggregate_mixed_running_is_running() {
        let objs = [obj("a", "completed"), obj("b", "in_progress")];
        let job = assemble_job("a+b".to_string(), &objs).unwrap();
        assert_eq!(job.status, VideoJobStatus::Running);
        assert!(job.failure_reason.is_none());
    }

    #[test]
    fn aggregate_all_queued_is_queued() {
        let objs = [obj("a", "queued"), obj("b", "queued")];
        let job = assemble_job("a+b".to_string(), &objs).unwrap();
        assert_eq!(job.status, VideoJobStatus::Queued);
    }

    #[test]
    fn aggregate_failed_error_falls_back_to_code_then_generic() {
        let objs = [VideoObjectWire {
            id: "b".to_string(),
            status: "failed".to_string(),
            size: None,
            seconds: None,
            error: Some(VideoErrorWire {
                code: Some("some_code".to_string()),
                message: None,
            }),
        }];
        let job = assemble_job("b".to_string(), &objs).unwrap();
        assert!(job.failure_reason.unwrap().contains("some_code"));

        let objs = [VideoObjectWire {
            id: "c".to_string(),
            status: "failed".to_string(),
            size: None,
            seconds: None,
            error: None,
        }];
        let job = assemble_job("c".to_string(), &objs).unwrap();
        assert!(job.failure_reason.unwrap().contains("unknown error"));
    }

    #[test]
    fn assemble_job_unknown_status_errors_naming_value_and_video_id() {
        let objs = [obj("video-x", "weird-status")];
        let err = assemble_job("video-x".to_string(), &objs)
            .unwrap_err()
            .to_string();
        assert!(err.contains("weird-status"), "error was: {}", err);
        assert!(err.contains("video-x"), "error was: {}", err);
    }

    // --- video-object parsing ---

    #[test]
    fn parse_video_object_full() {
        let text = r#"{"id":"video_1","object":"video","status":"completed","progress":100,"model":"sora-2","seconds":"8","size":"1280x720","created_at":1,"completed_at":2,"expires_at":3,"error":null}"#;
        let v = parse_video_object(text).unwrap();
        assert_eq!(v.id, "video_1");
        assert_eq!(v.status, "completed");
        assert_eq!(v.size.as_deref(), Some("1280x720"));
        assert_eq!(v.seconds.as_deref(), Some("8"));
        assert!(v.error.is_none());
        assert_eq!(parse_size_field(v.size.as_deref()), (1280, 720));
    }

    #[test]
    fn parse_video_object_minimal() {
        let text = r#"{"id":"video_2","status":"queued"}"#;
        let v = parse_video_object(text).unwrap();
        assert_eq!(v.id, "video_2");
        assert_eq!(v.status, "queued");
        assert!(v.size.is_none());
        assert!(v.seconds.is_none());
        assert!(v.error.is_none());
        assert_eq!(parse_size_field(v.size.as_deref()), (0, 0));
    }

    #[test]
    fn parse_video_object_with_error_field() {
        let text =
            r#"{"id":"video_3","status":"failed","error":{"code":"policy","message":"nope"}}"#;
        let v = parse_video_object(text).unwrap();
        assert_eq!(v.status, "failed");
        let e = v.error.unwrap();
        assert_eq!(e.describe(), "nope");
    }

    #[test]
    fn parse_size_field_handles_malformed() {
        assert_eq!(parse_size_field(Some("bogus")), (0, 0));
        assert_eq!(parse_size_field(Some("720x")), (0, 0));
        assert_eq!(parse_size_field(None), (0, 0));
    }

    // --- composite-id guard on download ---

    #[tokio::test]
    async fn download_rejects_composite_id_before_any_http_call() {
        let client = reqwest::Client::new();
        let a = api("https://r.example.com", None, &client);
        let err = a.download("a+b").await.unwrap_err().to_string();
        assert!(err.contains("a+b"), "error was: {}", err);
        assert!(
            err.contains("generation_ids"),
            "error should point at VideoJob::generation_ids, was: {}",
            err
        );
    }
}
