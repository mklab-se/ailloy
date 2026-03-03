//! Error types for AI provider clients.

use thiserror::Error;

/// Errors that can occur when interacting with AI providers.
#[derive(Debug, Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("JSON parsing failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Provider not configured: {0}")]
    NotConfigured(String),

    #[error("Binary not found: {binary}. Is it installed?")]
    BinaryNotFound { binary: String },

    #[error("{0}")]
    Other(String),
}
