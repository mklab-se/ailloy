//! Shared Azure/Foundry video-jobs (sora-2) request logic.
//!
//! Both `AzureOpenAiClient` and `FoundryClient` expose the same
//! `/openai/v1/video/generations/...` surface for asynchronous video
//! generation. This module centralizes URL construction, request/response
//! wire types, and error handling shared by both providers' `create_video_job`
//! / `get_video_job` / `download_video` / `delete_video_job` implementations.
//!
//! `download` first fetches the generation object (width/height/n_seconds),
//! then fetches the raw content bytes, and assembles a [`VideoResponse`] from
//! both — `VideoJob::generation_ids` stays a plain `Vec<String>`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::{VideoJob, VideoJobStatus, VideoOptions, VideoResponse};

/// Shared client for the Azure/Foundry video generation jobs API.
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
struct CreateJobRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    width: u32,
    height: u32,
    n_seconds: u32,
    n_variants: u8,
}

/// Build the create-job request body, applying defaults (1280x720, 5s, 1
/// variant) for any option left unset.
fn build_create_body<'a>(
    model: &'a str,
    prompt: &'a str,
    opts: Option<&VideoOptions>,
) -> CreateJobRequest<'a> {
    let (width, height) = opts.and_then(|o| o.size).unwrap_or((1280, 720));
    let n_seconds = opts.and_then(|o| o.seconds).unwrap_or(5);
    let n_variants = opts.and_then(|o| o.variants).unwrap_or(1);
    CreateJobRequest {
        model,
        prompt,
        width,
        height,
        n_seconds,
        n_variants,
    }
}

#[derive(Deserialize)]
struct JobResponseWire {
    id: String,
    status: String,
    #[serde(default)]
    generations: Vec<GenerationRefWire>,
    #[serde(default)]
    failure_reason: Option<String>,
}

#[derive(Deserialize)]
struct GenerationRefWire {
    id: String,
}

/// Generation metadata as returned by `GET .../video/generations/{id}`.
#[derive(Deserialize)]
struct GenerationDetail {
    width: u32,
    height: u32,
    n_seconds: u32,
}

/// Parse a job status/detail JSON body into a [`VideoJob`], mapping the wire
/// status string via [`VideoJobStatus::from_str`]. An unrecognized status
/// propagates an error naming the raw value and the job id.
fn parse_job_response(text: &str) -> Result<VideoJob> {
    let wire: JobResponseWire = serde_json::from_str(text).with_context(|| {
        format!(
            "Failed to parse Azure/Foundry video jobs API job response: {}",
            text
        )
    })?;
    let status = wire.status.parse::<VideoJobStatus>().map_err(|e| {
        anyhow::anyhow!(
            "Azure/Foundry video jobs API returned an unrecognized status for job '{}': {}",
            wire.id,
            e
        )
    })?;
    Ok(VideoJob {
        id: wire.id,
        status,
        generation_ids: wire.generations.into_iter().map(|g| g.id).collect(),
        failure_reason: wire.failure_reason,
    })
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

/// Actionable error for a non-2xx response from the video jobs API that
/// isn't a 404 (which gets the more specific expiry-aware message below).
fn video_api_error(status: u16, body: &str, action: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Azure/Foundry video jobs API: failed to {} (HTTP {}): {}",
        action,
        status,
        body
    )
}

/// 404 on a job/generation/content lookup: artifacts expire ~24h after
/// creation, so the actionable next step is to re-create the job.
fn expired_artifact_error(kind: &str, id: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Azure/Foundry video jobs API: {} '{}' not found (HTTP 404). \
         Video job artifacts expire after ~24 hours; re-create the video generation job to get a fresh one.",
        kind,
        id
    )
}

impl VideoJobsApi<'_> {
    fn api_version_param(&self) -> &str {
        self.api_version.unwrap_or("preview")
    }

    pub(crate) fn jobs_url(&self) -> String {
        format!(
            "{}/openai/v1/video/generations/jobs?api-version={}",
            self.base,
            self.api_version_param()
        )
    }

    pub(crate) fn job_url(&self, id: &str) -> String {
        format!(
            "{}/openai/v1/video/generations/jobs/{}?api-version={}",
            self.base,
            id,
            self.api_version_param()
        )
    }

    pub(crate) fn generation_url(&self, id: &str) -> String {
        format!(
            "{}/openai/v1/video/generations/{}?api-version={}",
            self.base,
            id,
            self.api_version_param()
        )
    }

    pub(crate) fn content_url(&self, id: &str) -> String {
        format!(
            "{}/openai/v1/video/generations/{}/content/video?api-version={}",
            self.base,
            id,
            self.api_version_param()
        )
    }

    /// Create an asynchronous video generation job.
    pub async fn create(
        &self,
        model: &str,
        prompt: &str,
        opts: Option<&VideoOptions>,
    ) -> Result<VideoJob> {
        let body = build_create_body(model, prompt, opts);

        let response = self
            .client
            .post(self.jobs_url())
            .header(self.header.0, &self.header.1)
            .json(&body)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry video jobs API (create job)")?;

        let status = response.status();
        let text = response
            .text()
            .await
            .context("Failed to read Azure/Foundry video jobs API create-job response")?;

        if !status.is_success() {
            return Err(video_api_error(
                status.as_u16(),
                &text,
                "create video generation job",
            ));
        }

        parse_job_response(&text)
    }

    /// Fetch the current state of a video generation job.
    pub async fn get(&self, id: &str) -> Result<VideoJob> {
        let response = self
            .client
            .get(self.job_url(id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry video jobs API (get job)")?;

        let status = response.status();
        let text = response
            .text()
            .await
            .context("Failed to read Azure/Foundry video jobs API get-job response")?;

        if status.as_u16() == 404 {
            return Err(expired_artifact_error("video generation job", id));
        }
        if !status.is_success() {
            return Err(video_api_error(
                status.as_u16(),
                &text,
                "get video generation job",
            ));
        }

        parse_job_response(&text)
    }

    /// Download a completed video generation: fetch the generation object
    /// for width/height/n_seconds, then fetch the raw content bytes, and
    /// assemble a [`VideoResponse`] from both.
    pub async fn download(&self, generation_id: &str) -> Result<VideoResponse> {
        let meta_response = self
            .client
            .get(self.generation_url(generation_id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry video jobs API (get generation)")?;

        let meta_status = meta_response.status();
        let meta_text = meta_response
            .text()
            .await
            .context("Failed to read Azure/Foundry video jobs API generation response")?;

        if meta_status.as_u16() == 404 {
            return Err(expired_artifact_error("video generation", generation_id));
        }
        if !meta_status.is_success() {
            return Err(video_api_error(
                meta_status.as_u16(),
                &meta_text,
                "get video generation metadata",
            ));
        }

        let detail: GenerationDetail = serde_json::from_str(&meta_text).with_context(|| {
            format!(
                "Failed to parse Azure/Foundry video generation metadata for '{}': {}",
                generation_id, meta_text
            )
        })?;

        let content_response = self
            .client
            .get(self.content_url(generation_id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry video jobs API (download content)")?;

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
            width: detail.width,
            height: detail.height,
            duration_seconds: detail.n_seconds,
        })
    }

    /// Delete a video generation job and its artifacts.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let response = self
            .client
            .delete(self.job_url(id))
            .header(self.header.0, &self.header.1)
            .send()
            .await
            .context("Failed to send request to Azure/Foundry video jobs API (delete job)")?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(expired_artifact_error("video generation job", id));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(video_api_error(
                status.as_u16(),
                &body,
                "delete video generation job",
            ));
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
    fn v1_urls_use_preview_api_version() {
        let client = reqwest::Client::new();
        let a = api("https://r.example.com", None, &client);
        assert_eq!(
            a.jobs_url(),
            "https://r.example.com/openai/v1/video/generations/jobs?api-version=preview"
        );
        assert_eq!(
            a.job_url("job-1"),
            "https://r.example.com/openai/v1/video/generations/jobs/job-1?api-version=preview"
        );
        assert_eq!(
            a.generation_url("gen-1"),
            "https://r.example.com/openai/v1/video/generations/gen-1?api-version=preview"
        );
        assert_eq!(
            a.content_url("gen-1"),
            "https://r.example.com/openai/v1/video/generations/gen-1/content/video?api-version=preview"
        );
    }

    #[test]
    fn dated_urls_use_explicit_api_version() {
        let client = reqwest::Client::new();
        let a = api("https://r.example.com", Some("2025-04-01-preview"), &client);
        assert_eq!(
            a.jobs_url(),
            "https://r.example.com/openai/v1/video/generations/jobs?api-version=2025-04-01-preview"
        );
        assert_eq!(
            a.job_url("job-1"),
            "https://r.example.com/openai/v1/video/generations/jobs/job-1?api-version=2025-04-01-preview"
        );
        assert_eq!(
            a.generation_url("gen-1"),
            "https://r.example.com/openai/v1/video/generations/gen-1?api-version=2025-04-01-preview"
        );
        assert_eq!(
            a.content_url("gen-1"),
            "https://r.example.com/openai/v1/video/generations/gen-1/content/video?api-version=2025-04-01-preview"
        );
    }

    // --- create-body JSON ---

    #[test]
    fn create_body_defaults() {
        let body = build_create_body("my-deployment", "a cat riding a bike", None);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "my-deployment");
        assert_eq!(json["prompt"], "a cat riding a bike");
        assert_eq!(json["width"], 1280);
        assert_eq!(json["height"], 720);
        assert_eq!(json["n_seconds"], 5);
        assert_eq!(json["n_variants"], 1);
    }

    #[test]
    fn create_body_explicit_options_mapping() {
        let opts = VideoOptions {
            size: Some((1920, 1080)),
            seconds: Some(12),
            variants: Some(3),
        };
        let body = build_create_body("dep", "prompt", Some(&opts));
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["width"], 1920);
        assert_eq!(json["height"], 1080);
        assert_eq!(json["n_seconds"], 12);
        assert_eq!(json["n_variants"], 3);
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
        assert_eq!(json["width"], 1280);
        assert_eq!(json["height"], 720);
        assert_eq!(json["n_seconds"], 8);
        assert_eq!(json["n_variants"], 1);
    }

    // --- job JSON parsing ---

    #[test]
    fn parse_job_full_with_generations_and_failure_reason() {
        let text = r#"{"id":"job-1","status":"succeeded","generations":[{"id":"gen-1","width":1280,"height":720,"n_seconds":5},{"id":"gen-2","width":1280,"height":720,"n_seconds":5}],"failure_reason":null}"#;
        let job = parse_job_response(text).unwrap();
        assert_eq!(job.id, "job-1");
        assert_eq!(job.status, VideoJobStatus::Succeeded);
        assert_eq!(
            job.generation_ids,
            vec!["gen-1".to_string(), "gen-2".to_string()]
        );
        assert!(job.failure_reason.is_none());
    }

    #[test]
    fn parse_job_with_failure_reason_set() {
        let text = r#"{"id":"job-2","status":"failed","generations":[],"failure_reason":"content policy violation"}"#;
        let job = parse_job_response(text).unwrap();
        assert_eq!(job.status, VideoJobStatus::Failed);
        assert!(job.generation_ids.is_empty());
        assert_eq!(
            job.failure_reason.as_deref(),
            Some("content policy violation")
        );
    }

    #[test]
    fn parse_job_minimal_no_generations_no_failure_reason() {
        let text = r#"{"id":"job-3","status":"queued"}"#;
        let job = parse_job_response(text).unwrap();
        assert_eq!(job.id, "job-3");
        assert_eq!(job.status, VideoJobStatus::Queued);
        assert!(job.generation_ids.is_empty());
        assert!(job.failure_reason.is_none());
    }

    #[test]
    fn parse_job_every_status_string() {
        for s in [
            "queued",
            "preprocessing",
            "running",
            "processing",
            "succeeded",
            "failed",
            "cancelled",
        ] {
            let text = format!(r#"{{"id":"j","status":"{}"}}"#, s);
            let job = parse_job_response(&text)
                .unwrap_or_else(|e| panic!("status '{}' should parse but got error: {}", s, e));
            assert_eq!(job.status.to_string(), s);
        }
    }

    #[test]
    fn parse_job_unknown_status_errors_naming_value_and_job_id() {
        let text = r#"{"id":"job-x","status":"weird-status"}"#;
        let err = parse_job_response(text).unwrap_err().to_string();
        assert!(err.contains("weird-status"), "error was: {}", err);
        assert!(err.contains("job-x"), "error was: {}", err);
    }
}
