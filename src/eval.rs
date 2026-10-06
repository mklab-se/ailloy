//! Typed evaluation: yes/no, choice and score questions over a state.
//!
//! The three question types mirror TypeSafe's System One primitives (Noul,
//! Choice, Score). TypeSafe nodes answer them natively with calibrated
//! probabilities; chat nodes answer them through structured output with
//! self-reported probabilities (see [`Calibration`]).

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::Usage;

/// Maximum number of options in a [`Question::Choice`].
pub const MAX_CHOICE_OPTIONS: usize = 255;
/// Maximum number of levels in a [`Question::Score`].
pub const MAX_SCORE_LEVELS: usize = 10;

/// Questions keyed by an ID you choose; answers come back under the same IDs.
pub type Questions = BTreeMap<String, Question>;

/// Optional descriptions of what a yes and a no mean.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct YesNoCriteria {
    pub yes: Option<Value>,
    pub no: Option<Value>,
}

/// A typed question about a state.
///
/// Instructions, option descriptions and level descriptions are JSON values:
/// a plain string, or structured JSON when the question needs extra data.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Question {
    /// Yes/no: answered with the probability that the answer is yes.
    YesNo {
        instructions: Value,
        criteria: Option<YesNoCriteria>,
    },
    /// One option from a set (2 to 255 options).
    Choice {
        instructions: Value,
        options: BTreeMap<String, Option<Value>>,
    },
    /// A position on ordered levels (2 to 10), lowest first.
    Score {
        instructions: Value,
        levels: Vec<Value>,
    },
}

impl Question {
    /// A yes/no question.
    pub fn yes_no(instructions: impl Into<Value>) -> Self {
        Self::YesNo {
            instructions: instructions.into(),
            criteria: None,
        }
    }

    /// A choice question; add options with [`Question::option`].
    pub fn choice(instructions: impl Into<Value>) -> Self {
        Self::Choice {
            instructions: instructions.into(),
            options: BTreeMap::new(),
        }
    }

    /// A score question; add levels with [`Question::level`] or [`Question::levels`].
    pub fn score(instructions: impl Into<Value>) -> Self {
        Self::Score {
            instructions: instructions.into(),
            levels: Vec::new(),
        }
    }

    /// Describe what a yes means (YesNo only; no effect on other types).
    pub fn yes(mut self, meaning: impl Into<Value>) -> Self {
        if let Self::YesNo { criteria, .. } = &mut self {
            criteria.get_or_insert_with(Default::default).yes = Some(meaning.into());
        }
        self
    }

    /// Describe what a no means (YesNo only; no effect on other types).
    pub fn no(mut self, meaning: impl Into<Value>) -> Self {
        if let Self::YesNo { criteria, .. } = &mut self {
            criteria.get_or_insert_with(Default::default).no = Some(meaning.into());
        }
        self
    }

    /// Add an option with a description (Choice only).
    pub fn option(mut self, key: impl Into<String>, description: impl Into<Value>) -> Self {
        if let Self::Choice { options, .. } = &mut self {
            options.insert(key.into(), Some(description.into()));
        }
        self
    }

    /// Add an option without a description (Choice only).
    pub fn option_bare(mut self, key: impl Into<String>) -> Self {
        if let Self::Choice { options, .. } = &mut self {
            options.insert(key.into(), None);
        }
        self
    }

    /// Append one level (Score only).
    pub fn level(mut self, description: impl Into<Value>) -> Self {
        if let Self::Score { levels, .. } = &mut self {
            levels.push(description.into());
        }
        self
    }

    /// Append several levels in order, lowest first (Score only).
    pub fn levels<I, T>(mut self, descriptions: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Value>,
    {
        if let Self::Score { levels, .. } = &mut self {
            levels.extend(descriptions.into_iter().map(Into::into));
        }
        self
    }

    /// `"yes_no"`, `"choice"` or `"score"`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::YesNo { .. } => "yes_no",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }

    /// The question's instructions.
    pub fn instructions(&self) -> &Value {
        match self {
            Self::YesNo { instructions, .. }
            | Self::Choice { instructions, .. }
            | Self::Score { instructions, .. } => instructions,
        }
    }
}

/// A typed answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Answer {
    YesNo {
        probability: f64,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        /// Probability-weighted level, `0.0..=(levels - 1)`.
        score: f64,
        /// One probability per level, lowest level first.
        probabilities: Vec<f64>,
        confidence: f64,
    },
}

impl Answer {
    /// Certainty in 0..1. For YesNo this is `|p - 0.5| * 2`.
    pub fn confidence(&self) -> f64 {
        match self {
            Self::YesNo { probability } => yes_no_confidence(*probability),
            Self::Choice { confidence, .. } | Self::Score { confidence, .. } => *confidence,
        }
    }

    /// `"yes_no"`, `"choice"` or `"score"`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::YesNo { .. } => "yes_no",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }

    /// The yes probability, for a YesNo answer.
    pub fn as_yes_no(&self) -> Option<f64> {
        match self {
            Self::YesNo { probability } => Some(*probability),
            _ => None,
        }
    }

    /// The chosen option, for a Choice answer.
    pub fn as_choice(&self) -> Option<&str> {
        match self {
            Self::Choice { choice, .. } => Some(choice),
            _ => None,
        }
    }

    /// The weighted score, for a Score answer.
    pub fn as_score(&self) -> Option<f64> {
        match self {
            Self::Score { score, .. } => Some(*score),
            _ => None,
        }
    }

    /// The score scaled to 0..1 (`score / (levels - 1)`), for a Score answer.
    pub fn normalized_score(&self) -> Option<f64> {
        match self {
            Self::Score {
                score,
                probabilities,
                ..
            } if probabilities.len() >= 2 => Some(score / (probabilities.len() - 1) as f64),
            _ => None,
        }
    }
}

/// Where an answer's probabilities come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Calibration {
    /// Calibrated probabilities from a judgment model (TypeSafe Jev).
    Measured,
    /// Probabilities stated by a chat model; not calibrated.
    SelfReported,
}

/// The answers to one evaluation request.
#[derive(Debug, Clone)]
pub struct EvalResponse {
    pub answers: BTreeMap<String, Answer>,
    pub model: String,
    pub usage: Option<Usage>,
    pub calibration: Calibration,
    /// Per-question rationale; filled by chat backends, empty for TypeSafe.
    pub rationale: BTreeMap<String, String>,
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// Check question shapes before any request is sent.
pub fn validate_questions(questions: &Questions) -> Result<()> {
    if questions.is_empty() {
        bail!("an evaluation needs at least one question");
    }
    for (id, question) in questions {
        if is_blank(question.instructions()) {
            bail!("question '{id}' has empty instructions; say what should be judged");
        }
        match question {
            Question::YesNo { .. } => {}
            Question::Choice { options, .. } => {
                if !(2..=MAX_CHOICE_OPTIONS).contains(&options.len()) {
                    bail!(
                        "choice question '{id}' has {} option(s); a choice needs 2 to 255 options",
                        options.len()
                    );
                }
                if options.keys().any(|k| k.trim().is_empty()) {
                    bail!(
                        "choice question '{id}' has an empty option key; give every option a name"
                    );
                }
            }
            Question::Score { levels, .. } => {
                if !(2..=MAX_SCORE_LEVELS).contains(&levels.len()) {
                    bail!(
                        "score question '{id}' has {} level(s); a score needs 2 to 10 levels",
                        levels.len()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Clamp to 0..1 (non-finite values become 0) and rescale to sum to 1.
/// All-zero input becomes a uniform distribution.
pub fn normalize_probabilities(raw: &[f64]) -> Vec<f64> {
    if raw.is_empty() {
        return Vec::new();
    }
    let clamped: Vec<f64> = raw
        .iter()
        .map(|p| {
            if p.is_finite() {
                p.clamp(0.0, 1.0)
            } else {
                0.0
            }
        })
        .collect();
    let total: f64 = clamped.iter().sum();
    if total <= 0.0 {
        return vec![1.0 / raw.len() as f64; raw.len()];
    }
    clamped.iter().map(|p| p / total).collect()
}

/// TypeSafe's Choice confidence: `(n * peak - 1) / (n - 1)`, clamped to 0..1.
pub fn choice_confidence(probabilities: &[f64]) -> f64 {
    let n = probabilities.len();
    if n < 2 {
        return 1.0;
    }
    let peak = probabilities.iter().copied().fold(0.0_f64, f64::max);
    ((n as f64 * peak - 1.0) / (n as f64 - 1.0)).clamp(0.0, 1.0)
}

/// TypeSafe's Score confidence: `1 - spread / evenSpread`, clamped to 0..1,
/// where spread is the probability-weighted distance from the peak level and
/// evenSpread is the mean distance from the middle level.
pub fn score_confidence(probabilities: &[f64]) -> f64 {
    let n = probabilities.len();
    if n < 2 {
        return 1.0;
    }
    let peak = peak_index(probabilities);
    let spread: f64 = probabilities
        .iter()
        .enumerate()
        .map(|(i, p)| p * (i as f64 - peak as f64).abs())
        .sum();
    let middle = (n as f64 - 1.0) / 2.0;
    let even: f64 = (0..n).map(|i| (i as f64 - middle).abs()).sum::<f64>() / n as f64;
    if even <= 0.0 {
        return 1.0;
    }
    (1.0 - spread / even).clamp(0.0, 1.0)
}

/// YesNo confidence: the two-option Choice case, `|p - 0.5| * 2`.
pub fn yes_no_confidence(probability: f64) -> f64 {
    ((probability - 0.5).abs() * 2.0).clamp(0.0, 1.0)
}

/// `sum_i i * p_i`.
pub fn weighted_score(probabilities: &[f64]) -> f64 {
    probabilities
        .iter()
        .enumerate()
        .map(|(i, p)| i as f64 * p)
        .sum()
}

fn peak_index(probabilities: &[f64]) -> usize {
    let mut best = 0;
    for (i, p) in probabilities.iter().enumerate() {
        if *p > probabilities[best] {
            best = i;
        }
    }
    best
}

/// Build a Choice answer from normalized probabilities. Ties go to the first
/// option in key order.
pub fn choice_answer(probabilities: BTreeMap<String, f64>) -> Answer {
    let values: Vec<f64> = probabilities.values().copied().collect();
    let peak = peak_index(&values);
    let choice = probabilities.keys().nth(peak).cloned().unwrap_or_default();
    Answer::Choice {
        choice,
        confidence: choice_confidence(&values),
        probabilities,
    }
}

/// Build a Score answer from normalized per-level probabilities.
pub fn score_answer(probabilities: Vec<f64>) -> Answer {
    Answer::Score {
        score: weighted_score(&probabilities),
        confidence: score_confidence(&probabilities),
        probabilities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn builders_produce_expected_variants() {
        let q = Question::yes_no("Is it urgent?")
            .yes("time-sensitive")
            .no("no rush");
        match &q {
            Question::YesNo {
                instructions,
                criteria,
            } => {
                assert_eq!(instructions, &json!("Is it urgent?"));
                let c = criteria.as_ref().unwrap();
                assert_eq!(c.yes, Some(json!("time-sensitive")));
                assert_eq!(c.no, Some(json!("no rush")));
            }
            other => panic!("unexpected {other:?}"),
        }
        let q = Question::choice("Which team?")
            .option("billing", "payments")
            .option_bare("other");
        match &q {
            Question::Choice { options, .. } => {
                assert_eq!(options.get("billing"), Some(&Some(json!("payments"))));
                assert_eq!(options.get("other"), Some(&None));
            }
            other => panic!("unexpected {other:?}"),
        }
        let q = Question::score("How angry?")
            .levels(["Calm", "Angry"])
            .level("Furious");
        match &q {
            Question::Score { levels, .. } => assert_eq!(levels.len(), 3),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(q.kind(), "score");
    }

    #[test]
    fn builder_methods_ignore_other_variants() {
        let q = Question::yes_no("x").option("a", "b").level("c");
        assert!(matches!(q, Question::YesNo { criteria: None, .. }));
    }

    fn one(id: &str, q: Question) -> Questions {
        let mut m = Questions::new();
        m.insert(id.to_string(), q);
        m
    }

    #[test]
    fn validation_accepts_well_formed_questions() {
        let mut qs = one("a", Question::yes_no("ok?"));
        qs.insert(
            "b".into(),
            Question::choice("pick").option_bare("x").option_bare("y"),
        );
        qs.insert("c".into(), Question::score("rate").levels(["lo", "hi"]));
        validate_questions(&qs).unwrap();
    }

    #[test]
    fn validation_rejects_empty_question_map() {
        let err = validate_questions(&Questions::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least one question"), "{err}");
    }

    #[test]
    fn validation_rejects_blank_instructions() {
        let err = validate_questions(&one("q1", Question::yes_no("  ")))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'q1'") && err.contains("instructions"),
            "{err}"
        );
    }

    #[test]
    fn validation_enforces_choice_limits() {
        let err = validate_questions(&one("team", Question::choice("pick").option_bare("only")))
            .unwrap_err()
            .to_string();
        assert!(err.contains("'team'") && err.contains("2 to 255"), "{err}");
        let mut big = Question::choice("pick");
        for i in 0..256 {
            big = big.option_bare(format!("o{i}"));
        }
        assert!(validate_questions(&one("team", big)).is_err());
        let err = validate_questions(&one(
            "team",
            Question::choice("pick").option_bare("").option_bare("b"),
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("empty option key"), "{err}");
    }

    #[test]
    fn validation_enforces_score_limits() {
        let err = validate_questions(&one("s", Question::score("rate").level("one")))
            .unwrap_err()
            .to_string();
        assert!(err.contains("'s'") && err.contains("2 to 10"), "{err}");
        let eleven: Vec<String> = (0..11).map(|i| format!("l{i}")).collect();
        assert!(validate_questions(&one("s", Question::score("rate").levels(eleven))).is_err());
    }

    #[test]
    fn normalize_handles_garbage() {
        let p = normalize_probabilities(&[-1.0, 2.0, f64::NAN]);
        assert_eq!(p, vec![0.0, 1.0, 0.0]);
        let p = normalize_probabilities(&[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(p, vec![0.25; 4]);
        let p = normalize_probabilities(&[0.2, 0.2]);
        assert!(approx(p[0], 0.5) && approx(p[1], 0.5));
        assert!(normalize_probabilities(&[]).is_empty());
    }

    #[test]
    fn confidence_formulas_match_typesafe_examples() {
        // Choice example from TypeSafe's API docs: 0.88/0.12/0.0 -> about 0.82.
        assert!(approx(choice_confidence(&[0.88, 0.12, 0.0]), 0.82));
        assert!(approx(choice_confidence(&[1.0, 0.0, 0.0]), 1.0));
        assert!(approx(choice_confidence(&[0.5, 0.5]), 0.0));
        // Score example from TypeSafe's API docs: 0/0.95/0.05 -> about 0.925.
        assert!(approx(score_confidence(&[0.0, 0.95, 0.05]), 0.925));
        assert!(approx(score_confidence(&[0.0, 0.0, 1.0]), 1.0));
        // Uniform spread is the zero-confidence reference.
        assert!(approx(score_confidence(&[0.25, 0.25, 0.25, 0.25]), 0.0));
        assert!(approx(yes_no_confidence(0.95), 0.9));
        assert!(approx(yes_no_confidence(0.5), 0.0));
        assert!(approx(yes_no_confidence(0.0), 1.0));
    }

    #[test]
    fn weighted_score_and_normalized_score() {
        assert!(approx(weighted_score(&[0.0, 0.95, 0.05]), 1.05));
        let a = score_answer(vec![0.0, 0.95, 0.05]);
        assert!(approx(a.as_score().unwrap(), 1.05));
        assert!(approx(a.normalized_score().unwrap(), 0.525));
        let two = score_answer(vec![0.0, 1.0]);
        assert!(approx(two.normalized_score().unwrap(), 1.0));
        let ten = score_answer({
            let mut v = vec![0.0; 10];
            v[3] = 1.0;
            v
        });
        assert!(approx(ten.normalized_score().unwrap(), 3.0 / 9.0));
        assert_eq!(Answer::YesNo { probability: 0.3 }.normalized_score(), None);
    }

    #[test]
    fn choice_answer_picks_peak_and_breaks_ties_by_key_order() {
        let mut p = BTreeMap::new();
        p.insert("technical".to_string(), 0.4);
        p.insert("billing".to_string(), 0.4);
        p.insert("other".to_string(), 0.2);
        let a = choice_answer(p);
        assert_eq!(a.as_choice(), Some("billing"));
        assert!(approx(a.confidence(), choice_confidence(&[0.4, 0.4, 0.2])));
    }

    #[test]
    fn answer_serializes_with_type_tag() {
        let v = serde_json::to_value(Answer::YesNo { probability: 0.9 }).unwrap();
        assert_eq!(v, json!({"type": "yes_no", "probability": 0.9}));
        assert_eq!(Answer::YesNo { probability: 0.9 }.kind(), "yes_no");
    }
}
