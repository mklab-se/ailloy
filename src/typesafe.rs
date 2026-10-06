//! TypeSafe System One client (Jev): native eval with calibrated probabilities.
//!
//! One `POST {endpoint}/v1/systemone` answers every question in a request;
//! TypeSafe judges each question independently, so batching is free.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::client::Provider;
use crate::error::ClientError;
use crate::eval::{
    Answer, Calibration, EvalResponse, Question, Questions, choice_answer, validate_questions,
};
use crate::types::{ChatOptions, ChatResponse, Message, Usage};

pub const DEFAULT_MODEL: &str = "jev-latest";
pub const DEFAULT_ENDPOINT: &str = "https://api.typesafe.ai";
pub const ENV_VAR: &str = "TYPESAFE_API_KEY";

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(8);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// Client for TypeSafe's System One API.
pub struct TypeSafeClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
    node_id: Option<String>,
}

impl TypeSafeClient {
    /// A client for `model` (e.g. `jev-latest`); `endpoint` defaults to
    /// `https://api.typesafe.ai`.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        endpoint: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            client,
            api_key: api_key.into(),
            model: model.into(),
            endpoint: endpoint
                .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string())
                .trim_end_matches('/')
                .to_string(),
            node_id: None,
        }
    }

    /// Name the config node, so errors can point at `ailloy ai config set-key <node>`.
    pub fn with_node_id(mut self, id: impl Into<String>) -> Self {
        self.node_id = Some(id.into());
        self
    }
}

fn question_wire(question: &Question) -> Value {
    match question {
        Question::YesNo {
            instructions,
            criteria,
        } => {
            let mut q = json!({"type": "noul", "instructions": instructions});
            if let Some(c) = criteria {
                let mut map = Map::new();
                if let Some(yes) = &c.yes {
                    map.insert("true".into(), yes.clone());
                }
                if let Some(no) = &c.no {
                    map.insert("false".into(), no.clone());
                }
                if !map.is_empty() {
                    q["criteria"] = Value::Object(map);
                }
            }
            q
        }
        Question::Choice {
            instructions,
            options,
        } => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().unwrap_or(Value::Null)))
                .collect();
            json!({"type": "choice", "instructions": instructions, "criteria": criteria})
        }
        Question::Score {
            instructions,
            levels,
        } => json!({"type": "score", "instructions": instructions, "criteria": levels}),
    }
}

pub(crate) fn request_body(model: &str, state: &Value, questions: &Questions) -> Value {
    let wire: Map<String, Value> = questions
        .iter()
        .map(|(id, q)| (id.clone(), question_wire(q)))
        .collect();
    json!({"model": model, "state": state, "questions": wire})
}

#[derive(Deserialize)]
struct WireResponse {
    model: String,
    answers: BTreeMap<String, WireAnswer>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

pub(crate) fn parse_response(body: &str, questions: &Questions) -> Result<EvalResponse> {
    let wire: WireResponse = serde_json::from_str(body)
        .with_context(|| format!("TypeSafe returned an unexpected response: {body}"))?;
    let mut answers = BTreeMap::new();
    for (id, question) in questions {
        let raw = wire
            .answers
            .get(id)
            .ok_or_else(|| anyhow!("TypeSafe returned no answer for question '{id}'"))?;
        let answer = match (question, raw) {
            (Question::YesNo { .. }, WireAnswer::Noul { noul }) => Answer::YesNo {
                probability: noul.clamp(0.0, 1.0),
            },
            (
                Question::Choice { .. },
                WireAnswer::Choice {
                    probabilities,
                    confidence,
                },
            ) => match choice_answer(probabilities.clone()) {
                Answer::Choice {
                    choice,
                    probabilities,
                    ..
                } => Answer::Choice {
                    choice,
                    probabilities,
                    confidence: *confidence,
                },
                other => other,
            },
            (
                Question::Score { levels, .. },
                WireAnswer::Score {
                    score,
                    probabilities,
                    confidence,
                },
            ) => {
                let per_level = (0..levels.len())
                    .map(|i| probabilities.get(&i.to_string()).copied().unwrap_or(0.0))
                    .collect();
                Answer::Score {
                    score: *score,
                    probabilities: per_level,
                    confidence: *confidence,
                }
            }
            _ => bail!(
                "TypeSafe answered question '{id}' with a different type than asked ({})",
                question.kind()
            ),
        };
        answers.insert(id.clone(), answer);
    }
    Ok(EvalResponse {
        answers,
        model: wire.model,
        usage: wire.usage.map(|u| Usage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: u.input_tokens + u.output_tokens,
        }),
        calibration: Calibration::Measured,
        rationale: BTreeMap::new(),
    })
}

/// Question ID from a validation `loc` such as `["body","questions","q",...]`.
fn question_from_loc(item: &Value) -> Option<String> {
    let loc = item.get("loc")?.as_array()?;
    let pos = loc.iter().position(|v| v == "questions")?;
    loc.get(pos + 1)?.as_str().map(str::to_string)
}

/// An actionable message for a non-success response.
pub(crate) fn format_error(
    status: u16,
    request_id: Option<&str>,
    body: &str,
    node_id: Option<&str>,
    model: &str,
) -> String {
    let node = node_id.unwrap_or("<node>");
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("detail").cloned());
    let base = match (status, &detail) {
        (401, _) => format!(
            "TypeSafe rejected the API key for node '{node}'. Check TYPESAFE_API_KEY or \
             run 'ailloy ai config set-key {node}'."
        ),
        (_, Some(Value::Object(obj))) => {
            let message = obj.get("message").and_then(Value::as_str).unwrap_or("");
            if message.starts_with("Unknown model") {
                format!(
                    "TypeSafe does not know model '{model}' ({message}). Set the node's model \
                     to 'jev-latest' or a versioned id such as 'jev-1.13.0'."
                )
            } else {
                format!("TypeSafe error (HTTP {status}): {message}")
            }
        }
        (_, Some(Value::Array(items))) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| {
                    let msg = item.get("msg").and_then(Value::as_str).unwrap_or("invalid");
                    match question_from_loc(item) {
                        Some(q) => format!("question '{q}': {msg}"),
                        None => msg.to_string(),
                    }
                })
                .collect();
            format!(
                "TypeSafe rejected the request (HTTP {status}): {}. Fix the question \
                 definition and retry.",
                parts.join("; ")
            )
        }
        (_, Some(Value::String(s))) => {
            format!("TypeSafe rejected the request (HTTP {status}): {s}")
        }
        (429, _) | (529, _) => format!(
            "TypeSafe is rate limiting or overloaded (HTTP {status}) and retries did not \
             help; wait a moment and try again."
        ),
        _ => format!("TypeSafe error (HTTP {status}): {}", body.trim()),
    };
    match request_id {
        Some(id) => format!("{base} (request id {id})"),
        None => base,
    }
}

pub(crate) fn should_retry(status: u16) -> bool {
    matches!(status, 429 | 529)
}

/// `retry-after` seconds when given (capped at 30 s), else 500 ms doubling, capped at 8 s.
pub(crate) fn retry_delay(attempt: u32, retry_after: Option<&str>) -> Duration {
    if let Some(secs) = retry_after.and_then(|s| s.trim().parse::<u64>().ok()) {
        return Duration::from_secs(secs).min(MAX_RETRY_AFTER);
    }
    BASE_DELAY
        .checked_mul(2u32.saturating_pow(attempt))
        .unwrap_or(MAX_BACKOFF)
        .min(MAX_BACKOFF)
}

pub(crate) struct RawResponse {
    pub status: u16,
    pub request_id: Option<String>,
    pub retry_after: Option<String>,
    pub body: String,
}

/// Run `send` up to `max_attempts` times while the status is retryable.
/// `floor` is added to every computed delay (tests pass zero).
pub(crate) async fn with_retries<F, Fut>(
    max_attempts: u32,
    floor: Duration,
    mut send: F,
) -> Result<RawResponse>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<RawResponse>>,
{
    let mut attempt = 0;
    loop {
        let raw = send().await?;
        attempt += 1;
        if !should_retry(raw.status) || attempt >= max_attempts {
            return Ok(raw);
        }
        let delay = retry_delay(attempt - 1, raw.retry_after.as_deref()) + floor;
        tokio::time::sleep(delay).await;
    }
}

#[async_trait]
impl Provider for TypeSafeClient {
    fn name(&self) -> &str {
        "typesafe"
    }

    async fn chat(
        &self,
        _messages: &[Message],
        _options: Option<&ChatOptions>,
    ) -> Result<ChatResponse> {
        Err(ClientError::Unsupported(
            "chat on a TypeSafe node: Jev answers typed eval questions only; pick a chat node \
             with --node or set defaults.chat"
                .to_string(),
        )
        .into())
    }

    async fn evaluate(&self, state: &Value, questions: &Questions) -> Result<EvalResponse> {
        validate_questions(questions)?;
        let url = format!("{}/v1/systemone", self.endpoint);
        let url = url.as_str();
        let body = request_body(&self.model, state, questions);
        let raw = with_retries(MAX_ATTEMPTS, Duration::ZERO, || {
            let request = self
                .client
                .post(url)
                .bearer_auth(&self.api_key)
                .json(&body);
            async move {
                let response = request.send().await.with_context(|| {
                    format!("could not reach TypeSafe at {url}; check your network and the node's endpoint")
                })?;
                let header = |name: &str| {
                    response
                        .headers()
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                };
                let status = response.status().as_u16();
                let request_id = header("x-typesafe-request-id");
                let retry_after = header("retry-after");
                let body = response.text().await.unwrap_or_default();
                Ok(RawResponse {
                    status,
                    request_id,
                    retry_after,
                    body,
                })
            }
        })
        .await?;
        if !(200..300).contains(&raw.status) {
            return Err(ClientError::Api {
                status: raw.status,
                message: format_error(
                    raw.status,
                    raw.request_id.as_deref(),
                    &raw.body,
                    self.node_id.as_deref(),
                    &self.model,
                ),
            }
            .into());
        }
        parse_response(&raw.body, questions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{Question, Questions};
    use serde_json::json;

    fn questions() -> Questions {
        let mut qs = Questions::new();
        qs.insert(
            "urgent".into(),
            Question::yes_no("Urgent?")
                .yes("time-sensitive")
                .no("no rush"),
        );
        qs.insert(
            "team".into(),
            Question::choice("Which team?")
                .option("billing", "payments")
                .option_bare("other"),
        );
        qs.insert(
            "mood".into(),
            Question::score("How frustrated?").levels(["Calm", "Frustrated", "Very angry"]),
        );
        qs
    }

    #[test]
    fn request_body_maps_question_types() {
        let body = request_body("jev-latest", &json!("text"), &questions());
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["state"], "text");
        assert_eq!(
            body["questions"]["urgent"],
            json!({"type": "noul", "instructions": "Urgent?",
                   "criteria": {"true": "time-sensitive", "false": "no rush"}})
        );
        assert_eq!(
            body["questions"]["team"],
            json!({"type": "choice", "instructions": "Which team?",
                   "criteria": {"billing": "payments", "other": null}})
        );
        assert_eq!(
            body["questions"]["mood"]["criteria"],
            json!(["Calm", "Frustrated", "Very angry"])
        );
    }

    #[test]
    fn yes_no_without_criteria_omits_the_field() {
        let mut qs = Questions::new();
        qs.insert("q".into(), Question::yes_no("ok?"));
        let body = request_body("m", &json!("s"), &qs);
        assert!(body["questions"]["q"].get("criteria").is_none());
    }

    // Shapes copied from a live response on 2026-10-06.
    const LIVE: &str = r#"{"model":"jev-1.13.0","answers":{
        "urgent":{"type":"noul","noul":0.99},
        "team":{"type":"choice","choice":"billing","confidence":1.0,
                "probabilities":{"billing":1.0,"other":0.0}},
        "mood":{"type":"score","score":2.0,"confidence":0.99,
                "legend":{"0":"Calm","1":"Frustrated","2":"Very angry"},
                "probabilities":{"0":0.0,"1":0.0,"2":1.0}}},
        "usage":{"input_tokens":462,"output_tokens":71}}"#;

    #[test]
    fn parses_live_response() {
        let resp = parse_response(LIVE, &questions()).unwrap();
        assert_eq!(resp.model, "jev-1.13.0");
        assert_eq!(resp.calibration, crate::eval::Calibration::Measured);
        assert_eq!(resp.answers["urgent"].as_yes_no(), Some(0.99));
        assert_eq!(resp.answers["team"].as_choice(), Some("billing"));
        assert_eq!(resp.answers["team"].confidence(), 1.0);
        assert_eq!(resp.answers["mood"].as_score(), Some(2.0));
        assert_eq!(resp.answers["mood"].confidence(), 0.99);
        assert!(resp.rationale.is_empty());
        let usage = resp.usage.unwrap();
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (462, 71, 533)
        );
    }

    #[test]
    fn parse_score_with_missing_level_key() {
        let body = r#"{"model":"jev-1.13.0","answers":{
            "mood":{"type":"score","score":1.0,"confidence":0.9,
                    "probabilities":{"1":1.0}}},"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let mut qs = Questions::new();
        qs.insert("mood".into(), Question::score("x").levels(["a", "b", "c"]));
        let resp = parse_response(body, &qs).unwrap();
        match &resp.answers["mood"] {
            crate::eval::Answer::Score { probabilities, .. } => {
                assert_eq!(probabilities, &vec![0.0, 1.0, 0.0]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn missing_answer_is_an_error_naming_the_question() {
        let body = r#"{"model":"m","answers":{},"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let err = parse_response(body, &questions()).unwrap_err().to_string();
        assert!(err.contains("'mood'") || err.contains("'team'"), "{err}");
    }

    // Error bodies copied from live responses on 2026-10-06.
    #[test]
    fn formats_auth_error() {
        let body = r#"{"detail":{"error_type":"authentication_error","message":"Cannot authenticate with the server. Please check your API key and try again."}}"#;
        let msg = format_error(
            401,
            Some("req_1"),
            body,
            Some("typesafe/jev-latest"),
            "jev-latest",
        );
        assert!(msg.contains("TYPESAFE_API_KEY"), "{msg}");
        assert!(
            msg.contains("ailloy ai config set-key typesafe/jev-latest"),
            "{msg}"
        );
        assert!(msg.contains("req_1"), "{msg}");
    }

    #[test]
    fn formats_unknown_model() {
        let body = r#"{"detail":{"error_type":"api_usage_error","message":"Unknown model: no-such-model"}}"#;
        let msg = format_error(400, None, body, None, "no-such-model");
        assert!(
            msg.contains("no-such-model") && msg.contains("jev-latest"),
            "{msg}"
        );
    }

    #[test]
    fn formats_validation_list_with_question_id() {
        let body = r#"{"detail":[{"type":"missing","loc":["body","questions","q","choice","criteria"],"msg":"Field required"}]}"#;
        let msg = format_error(422, None, body, None, "jev-latest");
        assert!(
            msg.contains("'q'") && msg.contains("Field required"),
            "{msg}"
        );
    }

    #[test]
    fn formats_plain_string_detail() {
        let body = r#"{"detail":"Too many score levels. Must have at most 10 levels."}"#;
        let msg = format_error(400, None, body, None, "jev-latest");
        assert!(msg.contains("Too many score levels"), "{msg}");
    }

    #[test]
    fn formats_non_json_body() {
        let msg = format_error(502, None, "<html>bad gateway</html>", None, "jev-latest");
        assert!(msg.contains("502") && msg.contains("bad gateway"), "{msg}");
    }

    #[test]
    fn retry_policy() {
        assert!(should_retry(429) && should_retry(529));
        assert!(!should_retry(400) && !should_retry(401) && !should_retry(500));
        assert_eq!(retry_delay(0, Some("3")), std::time::Duration::from_secs(3));
        assert_eq!(retry_delay(0, None), std::time::Duration::from_millis(500));
        assert_eq!(retry_delay(1, None), std::time::Duration::from_millis(1000));
        assert_eq!(retry_delay(10, None), std::time::Duration::from_secs(8));
        assert_eq!(
            retry_delay(0, Some("garbage")),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            retry_delay(0, Some("600")),
            std::time::Duration::from_secs(30),
            "capped"
        );
    }

    #[tokio::test]
    async fn retries_then_succeeds() {
        let mut calls = 0;
        let raw = with_retries(3, std::time::Duration::ZERO, || {
            calls += 1;
            let status = if calls < 3 { 429 } else { 200 };
            async move {
                Ok(RawResponse {
                    status,
                    request_id: None,
                    retry_after: Some("0".into()),
                    body: String::new(),
                })
            }
        })
        .await
        .unwrap();
        assert_eq!((raw.status, calls), (200, 3));
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let mut calls = 0;
        let raw = with_retries(3, std::time::Duration::ZERO, || {
            calls += 1;
            async move {
                Ok(RawResponse {
                    status: 529,
                    request_id: None,
                    retry_after: Some("0".into()),
                    body: String::new(),
                })
            }
        })
        .await
        .unwrap();
        assert_eq!((raw.status, calls), (529, 3));
    }

    #[tokio::test]
    async fn chat_is_unsupported_with_a_hint() {
        let client = TypeSafeClient::new("k", DEFAULT_MODEL, None);
        let err =
            crate::client::Provider::chat(&client, &[crate::types::Message::user("hi")], None)
                .await
                .unwrap_err()
                .to_string();
        assert!(err.contains("chat node"), "{err}");
    }
}
