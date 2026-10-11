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

    #[error("Operation not supported: {0}")]
    Unsupported(String),

    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("{0}")]
    Other(String),
}

/// What kind of failure an error is, so callers can react without parsing
/// provider messages: tell the user to top up, fix a key, wait, or fix the
/// request.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The key is missing, invalid, revoked or lacks permission.
    Auth,
    /// The account is out of credits or quota; waiting will not help.
    QuotaExhausted,
    /// Too many requests right now; retrying later will help.
    RateLimited,
    /// The provider is overloaded or failing; retrying later may help.
    Unavailable,
    /// The request itself was rejected (bad model, malformed question).
    InvalidRequest,
    /// The provider could not be reached (DNS, connect, timeout).
    Network,
    /// Anything else.
    Other,
}

/// A non-success HTTP response from a provider. Displays exactly the
/// message the provider error has always had; [`ProviderError::kind`]
/// classifies it.
#[derive(Debug)]
pub struct ProviderError {
    pub status: u16,
    pub kind: ErrorKind,
    message: String,
}

impl ProviderError {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

/// Build the error for a non-success response: `body` is the raw response
/// body (used only to classify), `message` is what the error displays.
pub(crate) fn api_error(status: u16, body: &str, message: String) -> anyhow::Error {
    anyhow::Error::new(ProviderError {
        status,
        kind: classify_status(status, body),
        message,
    })
}

/// Classify an HTTP status and response body. Out-of-credit responses
/// differ per provider (OpenAI: 429 `insufficient_quota`; Anthropic: 400
/// "credit balance is too low"; others 402), so the body is checked for
/// billing wording before the status decides.
pub fn classify_status(status: u16, body: &str) -> ErrorKind {
    let lower = body.to_ascii_lowercase();
    let billing = [
        "insufficient_quota",
        "current quota",
        "credit balance",
        "insufficient credit",
        "out of credit",
        "no credits",
        "credits exhausted",
        "insufficient balance",
        "billing",
        "payment required",
    ]
    .iter()
    .any(|w| lower.contains(w));
    match status {
        402 => ErrorKind::QuotaExhausted,
        400 | 403 | 429 if billing => ErrorKind::QuotaExhausted,
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimited,
        408 | 500 | 502 | 503 | 504 | 529 => ErrorKind::Unavailable,
        400 | 404 | 409 | 413 | 422 => ErrorKind::InvalidRequest,
        _ => ErrorKind::Other,
    }
}

impl ClientError {
    /// Classify this error; see [`ErrorKind`].
    pub fn kind(&self) -> ErrorKind {
        match self {
            ClientError::Http(e) => reqwest_kind(e),
            ClientError::Api { status, message } => classify_status(*status, message),
            ClientError::NotConfigured(_) => ErrorKind::Auth,
            ClientError::Json(_)
            | ClientError::BinaryNotFound { .. }
            | ClientError::Unsupported(_)
            | ClientError::NodeNotFound(_)
            | ClientError::Other(_) => ErrorKind::Other,
        }
    }
}

fn reqwest_kind(e: &reqwest::Error) -> ErrorKind {
    if let Some(status) = e.status() {
        return classify_status(status.as_u16(), "");
    }
    if e.is_timeout() || e.is_connect() || e.is_request() {
        ErrorKind::Network
    } else {
        ErrorKind::Other
    }
}

/// Classify any error returned by ailloy: walks the error chain for a
/// [`ProviderError`], [`ClientError`] or transport error.
pub fn error_kind(err: &anyhow::Error) -> ErrorKind {
    for cause in err.chain() {
        if let Some(e) = cause.downcast_ref::<ProviderError>() {
            return e.kind;
        }
        if let Some(e) = cause.downcast_ref::<ClientError>() {
            return e.kind();
        }
        if let Some(e) = cause.downcast_ref::<reqwest::Error>() {
            return reqwest_kind(e);
        }
    }
    ErrorKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_wording_wins_over_the_status() {
        let openai = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        assert_eq!(classify_status(429, openai), ErrorKind::QuotaExhausted);
        let anthropic = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#;
        assert_eq!(classify_status(400, anthropic), ErrorKind::QuotaExhausted);
        assert_eq!(classify_status(402, ""), ErrorKind::QuotaExhausted);
    }

    #[test]
    fn statuses_map_to_kinds() {
        let rate =
            r#"{"error":{"message":"Rate limit reached for gpt-5 on tokens per min (TPM)"}}"#;
        assert_eq!(classify_status(429, rate), ErrorKind::RateLimited);
        assert_eq!(classify_status(401, ""), ErrorKind::Auth);
        assert_eq!(classify_status(403, "forbidden"), ErrorKind::Auth);
        assert_eq!(classify_status(529, ""), ErrorKind::Unavailable);
        assert_eq!(classify_status(503, ""), ErrorKind::Unavailable);
        assert_eq!(classify_status(422, ""), ErrorKind::InvalidRequest);
        assert_eq!(classify_status(400, "bad model"), ErrorKind::InvalidRequest);
        assert_eq!(classify_status(418, ""), ErrorKind::Other);
    }

    #[test]
    fn error_kind_finds_the_provider_error_under_context() {
        let err = api_error(
            429,
            "insufficient_quota",
            "OpenAI API error (429): x".into(),
        )
        .context("chat failed");
        assert_eq!(error_kind(&err), ErrorKind::QuotaExhausted);
        assert_eq!(err.root_cause().to_string(), "OpenAI API error (429): x");
        let api = anyhow::Error::new(ClientError::Api {
            status: 401,
            message: "TypeSafe rejected the API key".into(),
        });
        assert_eq!(error_kind(&api), ErrorKind::Auth);
        assert_eq!(error_kind(&anyhow::anyhow!("boom")), ErrorKind::Other);
    }
}
