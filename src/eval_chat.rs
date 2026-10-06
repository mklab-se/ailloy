//! Eval emulation over chat: one structured-output chat call per question.
//!
//! Chat models answer batched questions with cross-question influence (order
//! effects and flattened distributions, measured 2026-10-06), so each question
//! gets its own call. Calls run concurrently, at most [`MAX_IN_FLIGHT`] at once.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{FutureExt, StreamExt};
use serde_json::{Value, json};

use crate::client::Provider;
use crate::eval::{
    Answer, Calibration, EvalResponse, Question, Questions, choice_answer, normalize_probabilities,
    score_answer, validate_questions,
};
use crate::types::{ChatOptions, Message, Usage};

/// Maximum concurrent chat calls per evaluation.
pub(crate) const MAX_IN_FLIGHT: usize = 4;

const SCHEMA_NAME: &str = "eval_answer";

const SYSTEM_PROMPT: &str = "You are an evaluator inside automated software. \
You receive a STATE and exactly one QUESTION about it. Judge only what the \
question asks, using only the STATE. Give honest, calibrated probabilities: \
use values near 0 or 1 only when the STATE makes the answer clear. Respond \
with JSON only, matching the schema.";

fn render(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn strict_object(properties: serde_json::Map<String, Value>) -> Value {
    let required: Vec<Value> = properties.keys().cloned().map(Value::String).collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn probability_map_schema<I: IntoIterator<Item = String>>(keys: I) -> Value {
    let props = keys
        .into_iter()
        .map(|k| (k, json!({"type": "number"})))
        .collect();
    strict_object(props)
}

/// JSON schema for one question's answer.
pub(crate) fn question_schema(question: &Question) -> Value {
    let mut props = serde_json::Map::new();
    match question {
        Question::YesNo { .. } => {
            props.insert("probability".into(), json!({"type": "number"}));
        }
        Question::Choice { options, .. } => {
            props.insert(
                "probabilities".into(),
                probability_map_schema(options.keys().cloned()),
            );
        }
        Question::Score { levels, .. } => {
            props.insert(
                "probabilities".into(),
                probability_map_schema((0..levels.len()).map(|i| i.to_string())),
            );
        }
    }
    props.insert("rationale".into(), json!({"type": "string"}));
    strict_object(props)
}

/// User prompt for one question.
pub(crate) fn question_prompt(state: &Value, question: &Question) -> String {
    let mut out = format!("STATE:\n{}\n\n", render(state));
    match question {
        Question::YesNo {
            instructions,
            criteria,
        } => {
            out.push_str(&format!("QUESTION (yes/no): {}\n", render(instructions)));
            if let Some(c) = criteria {
                if let Some(yes) = &c.yes {
                    out.push_str(&format!("A yes means: {}\n", render(yes)));
                }
                if let Some(no) = &c.no {
                    out.push_str(&format!("A no means: {}\n", render(no)));
                }
            }
            out.push_str(
                "\nGive `probability`: the probability (0 to 1) that the answer is yes, \
                 and a one-sentence `rationale`.",
            );
        }
        Question::Choice {
            instructions,
            options,
        } => {
            out.push_str(&format!(
                "QUESTION (choose one): {}\nOptions:\n",
                render(instructions)
            ));
            for (key, description) in options {
                match description {
                    Some(d) => out.push_str(&format!("- {key}: {}\n", render(d))),
                    None => out.push_str(&format!("- {key}\n")),
                }
            }
            out.push_str(
                "\nGive `probabilities`: a probability for every option (they should sum \
                 to 1), and a one-sentence `rationale`.",
            );
        }
        Question::Score {
            instructions,
            levels,
        } => {
            out.push_str(&format!(
                "QUESTION (score on ordered levels): {}\nLevels, lowest first:\n",
                render(instructions)
            ));
            for (i, level) in levels.iter().enumerate() {
                out.push_str(&format!("{i}: {}\n", render(level)));
            }
            out.push_str(
                "\nGive `probabilities`: a probability for every level number (they should \
                 sum to 1), and a one-sentence `rationale`.",
            );
        }
    }
    out
}

fn strip_fences(raw: &str) -> &str {
    let text = raw.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .unwrap_or(text);
    text.trim_end_matches("```").trim()
}

fn number(value: Option<&Value>, id: &str, field: &str) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("question '{id}': the model's answer has no numeric '{field}'"))
}

fn probability_map(root: &Value, id: &str, expected: &[String]) -> Result<Vec<f64>> {
    let map = root
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            anyhow!("question '{id}': the model's answer has no 'probabilities' object")
        })?;
    if let Some(extra) = map.keys().find(|k| !expected.contains(k)) {
        bail!("question '{id}': the model answered with unknown key '{extra}'");
    }
    expected
        .iter()
        .map(|key| {
            number(map.get(key), id, key)
                .map_err(|_| anyhow!("question '{id}': the model's answer is missing key '{key}'"))
        })
        .collect()
}

/// Parse one chat reply into an answer and its rationale.
pub(crate) fn parse_chat_answer(
    id: &str,
    question: &Question,
    raw: &str,
) -> Result<(Answer, String)> {
    let root: Value = serde_json::from_str(strip_fences(raw)).with_context(|| {
        format!("question '{id}': the model did not return valid JSON (got: {raw})")
    })?;
    let rationale = root
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let answer = match question {
        Question::YesNo { .. } => {
            let p = number(root.get("probability"), id, "probability")?;
            let p = if p.is_finite() {
                p.clamp(0.0, 1.0)
            } else {
                0.5
            };
            Answer::YesNo { probability: p }
        }
        Question::Choice { options, .. } => {
            let keys: Vec<String> = options.keys().cloned().collect();
            let probs = normalize_probabilities(&probability_map(&root, id, &keys)?);
            choice_answer(keys.into_iter().zip(probs).collect())
        }
        Question::Score { levels, .. } => {
            let keys: Vec<String> = (0..levels.len()).map(|i| i.to_string()).collect();
            score_answer(normalize_probabilities(&probability_map(&root, id, &keys)?))
        }
    };
    Ok((answer, rationale))
}

fn add_usage(total: &mut Option<Usage>, more: Option<Usage>) {
    if let Some(u) = more {
        let t = total.get_or_insert(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        });
        t.prompt_tokens += u.prompt_tokens;
        t.completion_tokens += u.completion_tokens;
        t.total_tokens += u.total_tokens;
    }
}

type QuestionOutcome = Result<(Answer, String, String, Option<Usage>)>;

/// One question, one chat call. Returns the question id with its outcome.
async fn ask_question<P: Provider + ?Sized>(
    provider: &P,
    state: &Value,
    id: &str,
    question: &Question,
) -> (String, QuestionOutcome) {
    let options = ChatOptions::builder()
        .json_schema(SCHEMA_NAME, question_schema(question))
        .build();
    let messages = [
        Message::system(SYSTEM_PROMPT),
        Message::user(question_prompt(state, question)),
    ];
    let outcome = async {
        let response = provider
            .chat(&messages, Some(&options))
            .await
            .with_context(|| format!("evaluating question '{id}' on {} failed", provider.name()))?;
        let (answer, rationale) = parse_chat_answer(id, question, &response.content)?;
        Ok((answer, rationale, response.model, response.usage))
    }
    .await;
    (id.to_string(), outcome)
}

/// Answer every question with its own chat call, at most [`MAX_IN_FLIGHT`]
/// at once. Any failing question fails the whole evaluation.
pub(crate) async fn evaluate_via_chat<P: Provider + ?Sized>(
    provider: &P,
    state: &Value,
    questions: &Questions,
) -> Result<EvalResponse> {
    validate_questions(questions)?;
    // Boxing the futures first keeps the stream's item type free of closure
    // lifetimes, which otherwise trips rustc's higher-ranked inference when
    // this runs inside an `async_trait` method.
    let pending: Vec<futures_util::future::BoxFuture<'_, (String, QuestionOutcome)>> = questions
        .iter()
        .map(|(id, question)| ask_question(provider, state, id, question).boxed())
        .collect();
    let results: Vec<(String, QuestionOutcome)> = futures_util::stream::iter(pending)
        .buffer_unordered(MAX_IN_FLIGHT)
        .collect()
        .await;

    let mut ordered: BTreeMap<String, (Answer, String, String, Option<Usage>)> = BTreeMap::new();
    for (id, outcome) in results {
        ordered.insert(id, outcome?);
    }
    let mut response = EvalResponse {
        answers: BTreeMap::new(),
        model: String::new(),
        usage: None,
        calibration: Calibration::SelfReported,
        rationale: BTreeMap::new(),
    };
    for (id, (answer, rationale, model, usage)) in ordered {
        if response.model.is_empty() {
            response.model = model;
        }
        add_usage(&mut response.usage, usage);
        if !rationale.is_empty() {
            response.rationale.insert(id.clone(), rationale);
        }
        response.answers.insert(id, answer);
    }
    Ok(response)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::Question;
    use serde_json::json;

    #[test]
    fn schema_for_each_question_type() {
        let s = question_schema(&Question::yes_no("ok?"));
        assert_eq!(s["required"], json!(["probability", "rationale"]));
        assert_eq!(s["additionalProperties"], json!(false));

        let s = question_schema(&Question::choice("pick").option_bare("a").option_bare("b"));
        assert_eq!(
            s["properties"]["probabilities"]["required"],
            json!(["a", "b"])
        );
        assert_eq!(
            s["properties"]["probabilities"]["additionalProperties"],
            json!(false)
        );

        let s = question_schema(&Question::score("rate").levels(["lo", "mid", "hi"]));
        assert_eq!(
            s["properties"]["probabilities"]["required"],
            json!(["0", "1", "2"])
        );
    }

    #[test]
    fn prompt_contains_state_question_and_criteria() {
        let p = question_prompt(
            &json!("ticket text"),
            &Question::yes_no("Urgent?").yes("time-sensitive"),
        );
        assert!(p.contains("ticket text") && p.contains("Urgent?") && p.contains("time-sensitive"));

        let p = question_prompt(
            &json!({"input": "x", "context": "y"}),
            &Question::score("Rate").levels(["Calm", "Angry"]),
        );
        assert!(
            p.contains("\"context\": \"y\""),
            "structured state is pretty JSON: {p}"
        );
        assert!(p.contains("0: Calm") && p.contains("1: Angry"));

        let p = question_prompt(
            &json!("s"),
            &Question::choice("Team?")
                .option("billing", "payments")
                .option_bare("other"),
        );
        assert!(p.contains("- billing: payments") && p.contains("- other"));
    }

    #[test]
    fn parses_yes_no_answer() {
        let (a, why) = parse_chat_answer(
            "q",
            &Question::yes_no("ok?"),
            r#"{"probability": 0.8, "rationale": "because"}"#,
        )
        .unwrap();
        assert_eq!(a.as_yes_no(), Some(0.8));
        assert_eq!(why, "because");
    }

    #[test]
    fn parses_fenced_chat_answer() {
        let raw = "```json\n{\"probability\": 1.4, \"rationale\": \"r\"}\n```";
        let (a, _) = parse_chat_answer("q", &Question::yes_no("ok?"), raw).unwrap();
        assert_eq!(a.as_yes_no(), Some(1.0), "clamped to 1");
    }

    #[test]
    fn chat_answer_with_out_of_range_probabilities_is_normalized() {
        let q = Question::choice("pick").option_bare("a").option_bare("b");
        let (a, _) = parse_chat_answer(
            "team",
            &q,
            r#"{"probabilities": {"a": 3, "b": -1}, "rationale": ""}"#,
        )
        .unwrap();
        assert_eq!(a.as_choice(), Some("a"));
        assert!((a.confidence() - 1.0).abs() < 1e-9);

        let q = Question::score("rate").levels(["lo", "hi"]);
        let (a, _) = parse_chat_answer(
            "s",
            &q,
            r#"{"probabilities": {"0": 0, "1": 0}, "rationale": ""}"#,
        )
        .unwrap();
        assert!(
            (a.as_score().unwrap() - 0.5).abs() < 1e-9,
            "all zero -> uniform"
        );
    }

    #[test]
    fn missing_or_extra_keys_name_the_question() {
        let q = Question::choice("pick").option_bare("a").option_bare("b");
        let err = parse_chat_answer(
            "team",
            &q,
            r#"{"probabilities": {"a": 1}, "rationale": ""}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'team'") && err.contains("'b'"), "{err}");
        let err = parse_chat_answer(
            "team",
            &q,
            r#"{"probabilities": {"a": 1, "b": 0, "c": 0}, "rationale": ""}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'team'") && err.contains("'c'"), "{err}");
        let err = parse_chat_answer("q", &Question::yes_no("ok?"), "not json")
            .unwrap_err()
            .to_string();
        assert!(err.contains("'q'"), "{err}");
    }
}
