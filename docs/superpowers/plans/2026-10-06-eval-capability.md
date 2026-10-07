# Eval Capability and TypeSafe Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `eval` capability (YesNo, Choice and Score questions over a state) served natively by a new `typesafe` provider (Jev) and emulated on every chat provider, and rebuild `ailloy eval` on it, released as ailloy 3.0.0.

**Architecture:** New library module `src/eval.rs` holds the public question/answer types, validation and confidence math. `Provider` gains `evaluate()`, whose default implementation (`src/eval_chat.rs`) asks a chat model one question per call, concurrently, via strict JSON schema; `src/typesafe.rs` overrides it with one `POST /v1/systemone`. `Client::eval` routes through `defaults.eval`, falling back to the default chat node. The CLI command `ailloy eval` is rewritten around question modes, gates and exit codes 0 to 4.

**Tech Stack:** Rust 2024 (MSRV 1.88), tokio, reqwest 0.13, async-trait, futures-util, serde/serde_json/serde_yaml, clap 4 derive, ratatui (config TUI).

**Spec:** `docs/superpowers/specs/2026-10-06-eval-capability-design.md`

## Global Constraints

- Edition 2024, MSRV 1.88; no new crate dependencies (everything needed is already in `Cargo.toml`).
- CI gate after every task: `cargo fmt --all -- --check && cargo clippy -- -D warnings && cargo test`; library-only build must also pass: `cargo build --no-default-features --lib`.
- No em-dashes (U+2014) in any text you write: code comments, docs, help text, errors, commit messages. Before every commit, after `git add`, run the em-dash check `git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'`; it must print nothing (pre-existing em-dashes on untouched lines are out of scope).
- All error messages must be actionable: say what went wrong, which node/question/file is involved, and what to do next.
- Config maps use `BTreeMap` (deterministic serialization).
- Question type names in ailloy: `YesNo`, `Choice`, `Score`. Wire value for YesNo on TypeSafe is `noul`.
- Capability config key: `eval`. Provider kind string: `typesafe`. Env var: `TYPESAFE_API_KEY`. Default model: `jev-latest`. Default endpoint: `https://api.typesafe.ai`.
- Limits: Choice 2 to 255 options; Score 2 to 10 levels.
- Chat emulation: one chat call per question, at most 4 in flight. TypeSafe: all questions in one request.
- TypeSafe HTTP timeout 120 s; retry HTTP 429 and 529 up to 3 attempts total with exponential backoff honoring `retry-after`.
- CLI exit codes: 0 pass, 1 gate failed (wins over 4), 2 usage/config, 3 provider, 4 below confidence floor.
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA
  ```
- Release version: 3.0.0 (set in Task 9, not before).

## Review Focus

1. A chat model returning probabilities that are negative, above 1, NaN, or all zero: answers must be normalized (uniform when all zero), never panic or produce NaN confidence. Test: Task 1 `normalize_handles_garbage` and Task 3 `chat_answer_with_out_of_range_probabilities_is_normalized`.
2. A chat model wrapping JSON in Markdown code fences (Anthropic uses prompted JSON): parsing must still succeed. Test: Task 3 `parses_fenced_chat_answer`.
3. A questions file with a `yes_no` entry that also sets `options`, or with zero/two type keys, or an `expect` key that is not an option: usage error (exit 2) naming the question ID, not a provider call. Test: Task 7 `questions_file_rejects_mixed_entry`, `questions_file_rejects_unknown_expect`.
4. Score probabilities from TypeSafe with a missing level key (server omits a zero-probability level): the missing level must read as 0.0, not shift later levels. Test: Task 4 `parse_score_with_missing_level_key`.
5. `defaults.eval` pointing at a node that no longer exists: a clear error naming the dangling node, not a silent fallback to chat. Test: Task 2 `default_eval_node_dangling_reference_errors`.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/eval.rs` (create) | Public types `Question`, `YesNoCriteria`, `Answer`, `Calibration`, `EvalResponse`; builders; `validate_questions`; probability/confidence math |
| `src/eval_chat.rs` (create, private) | Chat emulation: per-question prompt, JSON schema, answer parsing, concurrent `evaluate_via_chat` |
| `src/typesafe.rs` (create) | `TypeSafeClient`: wire mapping, error formatting, retry policy, `Provider::evaluate` override |
| `src/config.rs` (modify) | `Capability::Eval`, `ProviderKind::TypeSafe`, `#[non_exhaustive]`, capability lists, `supports_task`, `Config::default_eval_node` |
| `src/types.rs` (modify) | `Task::Evaluation`, `#[non_exhaustive]` on `Task` |
| `src/client.rs` (modify) | `Provider::evaluate` default, `Client::eval`/`eval_one`, eval routing, factory + builder arms for TypeSafe |
| `src/local_agent.rs` (modify) | `evaluate()` override returning `Unsupported` |
| `src/blocking.rs` (modify) | `eval` / `eval_one` mirrors |
| `src/discover.rs` (modify) | `TYPESAFE_API_KEY` discovery |
| `src/lib.rs` (modify) | module declarations and re-exports |
| `src/tui/forms.rs`, `src/tui/ui.rs`, `src/tui/mod.rs` (modify) | TypeSafe in provider selector, Eval column, TypeSafe-aware connectivity test |
| `src/config_tui.rs` (modify) | status line for `eval` fallback |
| `src/commands/ai.rs` (modify) | `ai test --all` pings TypeSafe nodes with a YesNo question |
| `src/cli.rs`, `src/main.rs`, `src/commands/eval.rs` (modify/rewrite) | new `ailloy eval` |
| `examples/eval.rs` (create), `examples/eval.sh` (rewrite) | usage examples |
| Docs: `README.md`, `src/doc/ai-reference.md`, `src/commands/skill.rs`, `CLAUDE.md`, `CHANGELOG.md`, `INSTALL.md`, `Cargo.toml` version | documentation and release metadata |

---

### Task 1: Eval types, validation and math (`src/eval.rs`)

**Files:**
- Create: `src/eval.rs`
- Modify: `src/lib.rs` (add `pub mod eval;` after `pub mod error;`, and re-export)

**Interfaces:**
- Consumes: `crate::types::Usage` (`prompt_tokens`, `completion_tokens`, `total_tokens: u32`).
- Produces (all `pub` in `crate::eval`):
  - `pub type Questions = std::collections::BTreeMap<String, Question>;`
  - `pub struct YesNoCriteria { pub yes: Option<Value>, pub no: Option<Value> }`
  - `#[non_exhaustive] pub enum Question { YesNo { instructions: Value, criteria: Option<YesNoCriteria> }, Choice { instructions: Value, options: BTreeMap<String, Option<Value>> }, Score { instructions: Value, levels: Vec<Value> } }`
  - `Question::yes_no(impl Into<Value>)`, `Question::choice(..)`, `Question::score(..)`, `.yes(v)`, `.no(v)`, `.option(key, desc)`, `.option_bare(key)`, `.level(v)`, `.levels(iter)`, `.kind() -> &'static str` (`"yes_no" | "choice" | "score"`), `.instructions() -> &Value`
  - `#[non_exhaustive] pub enum Answer { YesNo { probability: f64 }, Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 }, Score { score: f64, probabilities: Vec<f64>, confidence: f64 } }` with `confidence()`, `as_yes_no() -> Option<f64>`, `as_choice() -> Option<&str>`, `as_score() -> Option<f64>`, `normalized_score() -> Option<f64>`, `kind() -> &'static str`
  - `pub enum Calibration { Measured, SelfReported }`
  - `pub struct EvalResponse { pub answers: BTreeMap<String, Answer>, pub model: String, pub usage: Option<Usage>, pub calibration: Calibration, pub rationale: BTreeMap<String, String> }`
  - `pub fn validate_questions(&Questions) -> anyhow::Result<()>`
  - `pub fn normalize_probabilities(&[f64]) -> Vec<f64>`, `pub fn choice_confidence(&[f64]) -> f64`, `pub fn score_confidence(&[f64]) -> f64`, `pub fn yes_no_confidence(f64) -> f64`, `pub fn weighted_score(&[f64]) -> f64`, `pub fn choice_answer(BTreeMap<String, f64>) -> Answer`, `pub fn score_answer(Vec<f64>) -> Answer`

- [ ] **Step 1: Write the failing tests**

Create `src/eval.rs` containing only the test module below (the implementation follows in Step 3; putting tests first makes the compile failure the "red" state):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn builders_produce_expected_variants() {
        let q = Question::yes_no("Is it urgent?").yes("time-sensitive").no("no rush");
        match &q {
            Question::YesNo { instructions, criteria } => {
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
        let q = Question::score("How angry?").levels(["Calm", "Angry"]).level("Furious");
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
        qs.insert("b".into(), Question::choice("pick").option_bare("x").option_bare("y"));
        qs.insert("c".into(), Question::score("rate").levels(["lo", "hi"]));
        validate_questions(&qs).unwrap();
    }

    #[test]
    fn validation_rejects_empty_question_map() {
        let err = validate_questions(&Questions::new()).unwrap_err().to_string();
        assert!(err.contains("at least one question"), "{err}");
    }

    #[test]
    fn validation_rejects_blank_instructions() {
        let err = validate_questions(&one("q1", Question::yes_no("  ")))
            .unwrap_err()
            .to_string();
        assert!(err.contains("'q1'") && err.contains("instructions"), "{err}");
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
        let err = validate_questions(&one("team", Question::choice("pick").option_bare("").option_bare("b")))
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
```

- [ ] **Step 2: Add the module to `src/lib.rs` and run the tests to verify they fail**

In `src/lib.rs` add `pub mod eval;` after `pub mod error;`.

Run: `cargo test --lib eval::tests`
Expected: FAIL to compile (`cannot find type Question`, `cannot find function validate_questions`, ...).

- [ ] **Step 3: Write the implementation**

Insert above the test module in `src/eval.rs`:

```rust
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
    Score { instructions: Value, levels: Vec<Value> },
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
                    bail!("choice question '{id}' has an empty option key; give every option a name");
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
        .map(|p| if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 })
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
    let choice = probabilities
        .keys()
        .nth(peak)
        .cloned()
        .unwrap_or_default();
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
```

Note: the `normalize_handles_garbage` test expects `[-1.0, 2.0, NaN]` to become `[0.0, 1.0, 0.0]`: clamping gives `[0, 1, 0]`, total 1, so division leaves it unchanged.

- [ ] **Step 4: Re-export from `src/lib.rs`**

Add below the existing `pub use error::ClientError;` line:

```rust
pub use eval::{Answer, Calibration, EvalResponse, Question, Questions, YesNoCriteria};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib eval::tests`
Expected: all 11 tests PASS.

- [ ] **Step 6: Full gate and commit**

Run: `cargo fmt --all && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib`
Expected: all green.

```bash
git add src/eval.rs src/lib.rs
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(eval): question/answer types, validation and confidence math

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 2: `Capability::Eval`, `Task::Evaluation`, non-exhaustive enums and eval routing

**Files:**
- Modify: `src/config.rs` (ProviderKind at lines 14-106, Capability 112-162, constants 505-524, `default_node_for` ~826)
- Modify: `src/types.rs` (`Task` at ~1146)
- Modify: `src/config_tui.rs` (status loop at ~148-180)
- Modify: `src/tui/ui.rs` (`CAPABILITY_COLUMNS` at 20-26)
- Test: `src/config.rs` tests module, `src/types.rs` tests module

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `Capability::Eval` (config key `"eval"`, label `"Evaluation"`), `Task::Evaluation` (config key `"eval"`, `to_capability() == Some(Capability::Eval)`)
  - `Capability`, `ProviderKind`, `Task` are `#[non_exhaustive]`
  - `ProviderKind::supports_task("eval")` true for OpenAi, Anthropic, AzureOpenAi, MicrosoftFoundry, VertexAi, Ollama; false for LocalAgent
  - `Config::default_eval_node(&self) -> anyhow::Result<(&str, &AiNode)>`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` in `src/config.rs`:

```rust
    #[test]
    fn eval_capability_round_trips() {
        assert_eq!(Capability::Eval.config_key(), "eval");
        assert_eq!(Capability::Eval.label(), "Evaluation");
        assert_eq!("eval".parse::<Capability>().unwrap(), Capability::Eval);
        assert!(ALL_CAPABILITIES.iter().any(|(k, _)| *k == "eval"));
        assert!(ALL_CAPABILITY_KEYS.contains(&"eval"));
        let err = "nope".parse::<Capability>().unwrap_err();
        assert!(err.contains("eval"), "{err}");
    }

    #[test]
    fn eval_supported_by_chat_providers_but_not_local_agents() {
        for kind in [
            ProviderKind::OpenAi,
            ProviderKind::Anthropic,
            ProviderKind::AzureOpenAi,
            ProviderKind::MicrosoftFoundry,
            ProviderKind::VertexAi,
            ProviderKind::Ollama,
        ] {
            assert!(kind.supports_capability(&Capability::Eval), "{kind}");
            assert!(kind.supported_capabilities().contains(&Capability::Eval), "{kind}");
        }
        assert!(!ProviderKind::LocalAgent.supports_capability(&Capability::Eval));
    }

    fn config_with(nodes: &[(&str, ProviderKind)], defaults: &[(&str, &str)]) -> Config {
        let mut config = Config::default();
        for (id, kind) in nodes {
            config.nodes.insert(id.to_string(), AiNode::new(kind.clone()));
        }
        for (cap, id) in defaults {
            config.defaults.insert(cap.to_string(), id.to_string());
        }
        config
    }

    #[test]
    fn default_eval_node_prefers_eval_default() {
        let config = config_with(
            &[("openai/a", ProviderKind::OpenAi), ("ollama/b", ProviderKind::Ollama)],
            &[("chat", "openai/a"), ("eval", "ollama/b")],
        );
        assert_eq!(config.default_eval_node().unwrap().0, "ollama/b");
    }

    #[test]
    fn default_eval_node_falls_back_to_chat() {
        let config = config_with(&[("openai/a", ProviderKind::OpenAi)], &[("chat", "openai/a")]);
        assert_eq!(config.default_eval_node().unwrap().0, "openai/a");
    }

    #[test]
    fn default_eval_node_without_eval_or_chat_errors() {
        let config = config_with(&[], &[]);
        let err = config.default_eval_node().unwrap_err().to_string();
        assert!(err.contains("eval") && err.contains("ailloy ai config"), "{err}");
    }

    #[test]
    fn default_eval_node_dangling_reference_errors() {
        let config = config_with(&[("openai/a", ProviderKind::OpenAi)], &[
            ("chat", "openai/a"),
            ("eval", "typesafe/gone"),
        ]);
        let err = config.default_eval_node().unwrap_err().to_string();
        assert!(err.contains("typesafe/gone"), "{err}");
    }
```

Check whether `Config` implements `Default`: run `grep -n "impl Default for Config\|derive(.*Default" src/config.rs`. If it does not, build it in `config_with` the same way existing tests in this module build an empty `Config` (search the tests module for `Config {` and copy that literal).

Append to the tests module in `src/types.rs`:

```rust
    #[test]
    fn evaluation_task_maps_to_eval_capability() {
        assert_eq!(Task::Evaluation.config_key(), "eval");
        assert_eq!(
            Task::Evaluation.to_capability(),
            Some(crate::config::Capability::Eval)
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib eval_ -- --nocapture`
Expected: FAIL to compile (`no variant named Eval`, `no method default_eval_node`).

- [ ] **Step 3: Implement in `src/config.rs`**

1. Add `#[non_exhaustive]` on the line after the `#[derive(...)]` of `pub enum ProviderKind` and of `pub enum Capability`.
2. Add `Eval` to `Capability`, and extend its impls:

```rust
pub enum Capability {
    Chat,
    Image,
    Embedding,
    Video,
    Eval,
}
```

In `config_key`: `Self::Eval => "eval",`. In `label`: `Self::Eval => "Evaluation",`. In `FromStr`: add `"eval" => Ok(Self::Eval),` and change the error to `"Unknown capability '{}'. Valid: chat, image, embedding, video, eval"`.

3. Replace `supports_task` and `supported_capabilities`:

```rust
    /// Returns whether this provider kind supports a given task.
    pub fn supports_task(&self, task: &str) -> bool {
        matches!(
            (self, task),
            (_, "chat")
                | (
                    Self::OpenAi | Self::AzureOpenAi | Self::VertexAi | Self::MicrosoftFoundry,
                    "image"
                )
                | (Self::AzureOpenAi | Self::MicrosoftFoundry, "video")
                | (
                    Self::OpenAi
                        | Self::AzureOpenAi
                        | Self::Ollama
                        | Self::VertexAi
                        | Self::MicrosoftFoundry,
                    "embedding"
                )
                | (
                    Self::OpenAi
                        | Self::Anthropic
                        | Self::AzureOpenAi
                        | Self::MicrosoftFoundry
                        | Self::VertexAi
                        | Self::Ollama,
                    "eval"
                )
        )
    }

    /// Returns the capabilities this provider kind can potentially support.
    pub fn supported_capabilities(&self) -> Vec<Capability> {
        [
            Capability::Chat,
            Capability::Image,
            Capability::Video,
            Capability::Embedding,
            Capability::Eval,
        ]
        .into_iter()
        .filter(|cap| self.supports_capability(cap))
        .collect()
    }
```

(Task 4 narrows `(_, "chat")` when `ProviderKind::TypeSafe` arrives.)

4. Constants:

```rust
pub const ALL_CAPABILITIES: &[(&str, &str)] = &[
    ("chat", "Chat"),
    ("image", "Image Generation"),
    ("embedding", "Embedding"),
    ("video", "Video Generation"),
    ("eval", "Evaluation"),
];
```

and `pub const ALL_CAPABILITY_KEYS: &[&str] = &["chat", "image", "video", "embedding", "eval"];` (update its doc comment to list eval).

5. Add after `default_chat_node`:

```rust
    /// The node that serves `eval`: `defaults.eval` when set, otherwise the
    /// default chat node (any chat model can emulate evaluation).
    pub fn default_eval_node(&self) -> Result<(&str, &AiNode)> {
        if self.defaults.contains_key("eval") {
            return self.default_node_for("eval");
        }
        self.default_node_for("chat").map_err(|_| {
            anyhow::anyhow!(
                "No node configured for 'eval' and no default chat node to fall back to. \
                 Run `ailloy ai config` to add a TypeSafe or chat node."
            )
        })
    }
```

- [ ] **Step 4: Implement in `src/types.rs`**

Add `#[non_exhaustive]` to `pub enum Task`, add the `Evaluation` variant after `Embedding`, and extend both match blocks: `Self::Evaluation => "eval",` in `config_key`, `Self::Evaluation => Some(crate::config::Capability::Eval),` in `to_capability`.

- [ ] **Step 5: Fix the compile fallout**

Run: `cargo build`. Expected errors are non-exhaustive matches; fix exactly these:
- `src/tui/ui.rs` `CAPABILITY_COLUMNS`: append `Capability::Eval,` and change the doc comment to "The five capability columns".
- Any `match` over `Capability`/`Task` in the binary crate (`src/main.rs`, `src/commands/*.rs`) reported by the compiler: add the missing `Capability::Eval`/`Task::Evaluation` arm with the same behavior as the closest existing arm, or `_ =>` when the match is a filter.

If the TUI node table has a header row built from `CAPABILITY_COLUMNS` with fixed column widths, add a header label `"Eval"` the same way the existing ones are produced (search `src/tui/ui.rs` for `"Video"`).

- [ ] **Step 6: Status line for the eval fallback in `src/config_tui.rs`**

In the status loop (`for &cap_key in capabilities`), the `None` branch of `match config.defaults.get(cap_key)` currently prints "not configured" style output. Before that match, add:

```rust
        if cap_key == "eval"
            && !config.defaults.contains_key("eval")
            && let Ok((chat_id, _)) = config.default_chat_node()
        {
            println!(
                "  {} {}: {} {}",
                "✓".green().bold(),
                label,
                chat_id.bold(),
                "(default chat node; set defaults.eval to use a TypeSafe node)".dimmed()
            );
            continue;
        }
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS, including the 7 new tests and the existing `ALL_CAPABILITY_KEYS` drift test. If an existing test asserts an exact `supported_capabilities()` vector (config.rs ~1490-1531), update its expected vector to include `Capability::Eval` at the end for every provider except `LocalAgent`.

- [ ] **Step 8: Gate and commit**

Run: `cargo fmt --all && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib`

```bash
git add -A src
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(eval): eval capability, evaluation task, non-exhaustive enums

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 3: Chat emulation and `Client::eval`

**Files:**
- Create: `src/eval_chat.rs`
- Modify: `src/lib.rs` (`mod eval_chat;`, private, after `mod mai_images;`)
- Modify: `src/client.rs` (trait at lines 17-156, `Client::for_capability` ~386, new methods after `embed_one` ~643)
- Modify: `src/local_agent.rs` (Provider impl ~97)
- Modify: `src/blocking.rs`

**Interfaces:**
- Consumes: from Task 1 `Question`, `Questions`, `Answer`, `EvalResponse`, `Calibration`, `validate_questions`, `normalize_probabilities`, `choice_answer`, `score_answer`; `ChatOptions::builder().json_schema(name, schema)`; `Message::system`, `Message::user`; `Provider::chat`.
- Produces:
  - `Provider::evaluate(&self, state: &serde_json::Value, questions: &Questions) -> anyhow::Result<EvalResponse>` (default = chat emulation)
  - `Client::eval(&self, state: impl Into<serde_json::Value>, questions: &Questions) -> Result<EvalResponse>`
  - `Client::eval_one(&self, state: impl Into<serde_json::Value>, question: Question) -> Result<Answer>`
  - `blocking::Client::eval` / `eval_one` with the same signatures, synchronous
  - `pub(crate) fn eval_chat::evaluate_via_chat<P: Provider + ?Sized>(provider: &P, state: &Value, questions: &Questions) -> Result<EvalResponse>` (async)
  - `pub(crate) const eval_chat::MAX_IN_FLIGHT: usize = 4;`

- [ ] **Step 1: Write the failing tests for the pure helpers**

Create `src/eval_chat.rs` with only this test module:

```rust
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
        assert_eq!(s["properties"]["probabilities"]["required"], json!(["a", "b"]));
        assert_eq!(s["properties"]["probabilities"]["additionalProperties"], json!(false));

        let s = question_schema(&Question::score("rate").levels(["lo", "mid", "hi"]));
        assert_eq!(s["properties"]["probabilities"]["required"], json!(["0", "1", "2"]));
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
        assert!(p.contains("\"context\": \"y\""), "structured state is pretty JSON: {p}");
        assert!(p.contains("0: Calm") && p.contains("1: Angry"));

        let p = question_prompt(&json!("s"), &Question::choice("Team?").option("billing", "payments").option_bare("other"));
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
        assert!((a.as_score().unwrap() - 0.5).abs() < 1e-9, "all zero -> uniform");
    }

    #[test]
    fn missing_or_extra_keys_name_the_question() {
        let q = Question::choice("pick").option_bare("a").option_bare("b");
        let err = parse_chat_answer("team", &q, r#"{"probabilities": {"a": 1}, "rationale": ""}"#)
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
```

- [ ] **Step 2: Run to verify failure**

Add `mod eval_chat;` to `src/lib.rs` (after `mod mai_images;`).
Run: `cargo test --lib eval_chat::tests`
Expected: FAIL to compile (`question_schema` not found, ...).

- [ ] **Step 3: Implement the helpers and `evaluate_via_chat`**

Insert above the test module:

```rust
//! Eval emulation over chat: one structured-output chat call per question.
//!
//! Chat models answer batched questions with cross-question influence (order
//! effects and flattened distributions, measured 2026-10-06), so each question
//! gets its own call. Calls run concurrently, at most [`MAX_IN_FLIGHT`] at once.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::client::Provider;
use crate::eval::{
    Answer, Calibration, EvalResponse, Question, Questions, choice_answer,
    normalize_probabilities, score_answer, validate_questions,
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
            out.push_str(&format!("QUESTION (choose one): {}\nOptions:\n", render(instructions)));
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

fn probability_map(
    root: &Value,
    id: &str,
    expected: &[String],
) -> Result<Vec<f64>> {
    let map = root
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("question '{id}': the model's answer has no 'probabilities' object"))?;
    if let Some(extra) = map.keys().find(|k| !expected.contains(k)) {
        bail!("question '{id}': the model answered with unknown key '{extra}'");
    }
    expected
        .iter()
        .map(|key| {
            number(map.get(key), id, key).map_err(|_| {
                anyhow!("question '{id}': the model's answer is missing key '{key}'")
            })
        })
        .collect()
}

/// Parse one chat reply into an answer and its rationale.
pub(crate) fn parse_chat_answer(id: &str, question: &Question, raw: &str) -> Result<(Answer, String)> {
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
            let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.5 };
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

/// Answer every question with its own chat call, at most [`MAX_IN_FLIGHT`]
/// at once. Any failing question fails the whole evaluation.
pub(crate) async fn evaluate_via_chat<P: Provider + ?Sized>(
    provider: &P,
    state: &Value,
    questions: &Questions,
) -> Result<EvalResponse> {
    validate_questions(questions)?;
    let results: Vec<(String, Result<(Answer, String, String, Option<Usage>)>)> =
        futures_util::stream::iter(questions.iter())
            .map(|(id, question)| async move {
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
                        .with_context(|| {
                            format!("evaluating question '{id}' on {} failed", provider.name())
                        })?;
                    let (answer, rationale) = parse_chat_answer(id, question, &response.content)?;
                    Ok((answer, rationale, response.model, response.usage))
                }
                .await;
                (id.clone(), outcome)
            })
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
```

Note: `outcome?` returns the first failing question in ID order; its message names the question.

- [ ] **Step 4: Run helper tests to verify they pass**

Run: `cargo test --lib eval_chat::tests`
Expected: 6 tests PASS.

- [ ] **Step 5: Write failing tests for the trait default and `Client::eval`**

Append to the tests module in `src/client.rs`:

```rust
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Answers every eval question from canned JSON chosen by question type,
    /// recording prompts and tracking peak concurrency.
    struct MockJudge {
        prompts: Mutex<Vec<String>>,
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        fail_on: Option<&'static str>,
    }

    impl MockJudge {
        fn new(fail_on: Option<&'static str>) -> Self {
            Self {
                prompts: Mutex::new(Vec::new()),
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                fail_on,
            }
        }
    }

    #[async_trait]
    impl Provider for MockJudge {
        fn name(&self) -> &str {
            "mock-judge"
        }

        async fn chat(
            &self,
            messages: &[Message],
            _options: Option<&ChatOptions>,
        ) -> Result<ChatResponse> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::task::yield_now().await;
            let prompt = messages.last().unwrap().content.text().to_string();
            self.prompts.lock().unwrap().push(prompt.clone());
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if let Some(marker) = self.fail_on
                && prompt.contains(marker)
            {
                anyhow::bail!("boom");
            }
            let content = if prompt.contains("(yes/no)") {
                r#"{"probability": 0.9, "rationale": "yes-ish"}"#
            } else if prompt.contains("(choose one)") {
                r#"{"probabilities": {"a": 0.7, "b": 0.3}, "rationale": "a"}"#
            } else {
                r#"{"probabilities": {"0": 0.0, "1": 1.0}, "rationale": "high"}"#
            };
            Ok(ChatResponse {
                content: content.to_string(),
                model: "mock-model".to_string(),
                usage: Some(crate::types::Usage {
                    prompt_tokens: 10,
                    completion_tokens: 2,
                    total_tokens: 12,
                }),
            })
        }
    }

    fn mixed_questions(n_yes_no: usize) -> crate::eval::Questions {
        let mut qs = crate::eval::Questions::new();
        for i in 0..n_yes_no {
            qs.insert(format!("yn{i}"), crate::eval::Question::yes_no(format!("question {i}?")));
        }
        qs.insert(
            "pick".into(),
            crate::eval::Question::choice("pick one").option_bare("a").option_bare("b"),
        );
        qs.insert(
            "rate".into(),
            crate::eval::Question::score("rate it").levels(["lo", "hi"]),
        );
        qs
    }

    #[tokio::test]
    async fn chat_emulation_asks_one_call_per_question() {
        let client = Client::from_provider(Box::new(MockJudge::new(None)));
        let resp = client.eval("some state", &mixed_questions(1)).await.unwrap();
        assert_eq!(resp.answers.len(), 3);
        assert_eq!(resp.calibration, crate::eval::Calibration::SelfReported);
        assert_eq!(resp.answers["yn0"].as_yes_no(), Some(0.9));
        assert_eq!(resp.answers["pick"].as_choice(), Some("a"));
        assert_eq!(resp.answers["rate"].as_score(), Some(1.0));
        assert_eq!(resp.rationale["pick"], "a");
        assert_eq!(resp.model, "mock-model");
        let usage = resp.usage.unwrap();
        assert_eq!((usage.prompt_tokens, usage.total_tokens), (30, 36));
    }

    #[tokio::test]
    async fn chat_emulation_runs_concurrently_but_capped() {
        let judge = std::sync::Arc::new(MockJudge::new(None));
        struct Shared(std::sync::Arc<MockJudge>);
        #[async_trait]
        impl Provider for Shared {
            fn name(&self) -> &str {
                "shared"
            }
            async fn chat(
                &self,
                messages: &[Message],
                options: Option<&ChatOptions>,
            ) -> Result<ChatResponse> {
                self.0.chat(messages, options).await
            }
        }
        let client = Client::from_provider(Box::new(Shared(judge.clone())));
        client.eval("s", &mixed_questions(8)).await.unwrap();
        assert_eq!(judge.prompts.lock().unwrap().len(), 10);
        let peak = judge.peak.load(Ordering::SeqCst);
        assert!(peak > 1 && peak <= crate::eval_chat::MAX_IN_FLIGHT, "peak {peak}");
    }

    #[tokio::test]
    async fn chat_emulation_failure_names_the_question() {
        let client = Client::from_provider(Box::new(MockJudge::new(Some("rate it"))));
        let err = format!("{:#}", client.eval("s", &mixed_questions(1)).await.unwrap_err());
        assert!(err.contains("'rate'") && err.contains("boom"), "{err}");
    }

    #[tokio::test]
    async fn eval_one_returns_the_single_answer() {
        let client = Client::from_provider(Box::new(MockJudge::new(None)));
        let answer = client
            .eval_one("s", crate::eval::Question::yes_no("ok?"))
            .await
            .unwrap();
        assert_eq!(answer.as_yes_no(), Some(0.9));
    }

    #[tokio::test]
    async fn eval_rejects_invalid_questions_before_calling() {
        let judge = MockJudge::new(None);
        let client = Client::from_provider(Box::new(judge));
        let err = client
            .eval("s", &crate::eval::Questions::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least one question"), "{err}");
    }
```

Append to the tests module in `src/local_agent.rs`:

```rust
    #[tokio::test]
    async fn local_agent_does_not_support_eval() {
        let client = LocalAgentClient::new("claude");
        let mut qs = crate::eval::Questions::new();
        qs.insert("q".into(), crate::eval::Question::yes_no("ok?"));
        let err = crate::client::Provider::evaluate(&client, &serde_json::json!("s"), &qs)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("eval"), "{err}");
    }
```

- [ ] **Step 6: Run to verify failure**

Run: `cargo test --lib eval`
Expected: FAIL to compile (`no method named eval`, `no function evaluate`).

- [ ] **Step 7: Implement the trait method, client methods and local-agent override**

In `src/client.rs`, add to the imports: `use crate::eval::{Answer, EvalResponse, Question, Questions};`. Add to the `Provider` trait, after `embed`:

```rust
    /// Answer typed questions about a state.
    ///
    /// Default implementation emulates evaluation over [`Provider::chat`]
    /// with one structured-output call per question (self-reported
    /// probabilities). Judgment providers such as TypeSafe override it.
    async fn evaluate(
        &self,
        state: &serde_json::Value,
        questions: &Questions,
    ) -> Result<EvalResponse> {
        crate::eval_chat::evaluate_via_chat(self, state, questions).await
    }
```

In `impl Client`, after `embed_one`:

```rust
    /// Evaluate typed questions about `state` (a string or structured JSON).
    ///
    /// Routes to the node's provider: TypeSafe nodes answer natively in one
    /// request; chat nodes answer one question per call.
    pub async fn eval(
        &self,
        state: impl Into<serde_json::Value>,
        questions: &Questions,
    ) -> Result<EvalResponse> {
        let state = state.into();
        self.provider.evaluate(&state, questions).await
    }

    /// Evaluate a single question and return its answer.
    pub async fn eval_one(
        &self,
        state: impl Into<serde_json::Value>,
        question: Question,
    ) -> Result<Answer> {
        let mut questions = Questions::new();
        questions.insert("q".to_string(), question);
        let mut response = self.eval(state, &questions).await?;
        response
            .answers
            .remove("q")
            .context("the evaluation returned no answer for the question")
    }
```

In `Client::for_capability`, replace `let (id, node) = config.default_node_for(cap)?;` with:

```rust
        let (id, node) = if cap == "eval" {
            config.default_eval_node()?
        } else {
            config.default_node_for(cap)?
        };
```

In `src/local_agent.rs`, inside `impl Provider for LocalAgentClient`, add:

```rust
    async fn evaluate(
        &self,
        _state: &serde_json::Value,
        _questions: &crate::eval::Questions,
    ) -> Result<crate::eval::EvalResponse> {
        Err(crate::error::ClientError::Unsupported(format!(
            "eval on local agent '{}': CLI agents cannot follow a JSON schema; \
             use a TypeSafe or API chat node (run `ailloy ai config`)",
            self.binary
        ))
        .into())
    }
```

(Check the field name holding the binary in `LocalAgentClient`; the `binary()` accessor at line 52 returns it, so `self.binary()` works if the field differs.)

In `src/blocking.rs`, add after the blocking `embed_one` (or after `chat` if there is no embed mirror; follow the existing method style):

```rust
    /// Evaluate typed questions about `state` (blocking).
    pub fn eval(
        &self,
        state: impl Into<serde_json::Value>,
        questions: &crate::eval::Questions,
    ) -> Result<crate::eval::EvalResponse> {
        self.runtime.block_on(self.inner.eval(state, questions))
    }

    /// Evaluate a single question (blocking).
    pub fn eval_one(
        &self,
        state: impl Into<serde_json::Value>,
        question: crate::eval::Question,
    ) -> Result<crate::eval::Answer> {
        self.runtime.block_on(self.inner.eval_one(state, question))
    }
```

- [ ] **Step 8: Run tests**

Run: `cargo test --lib eval`
Expected: PASS (chat emulation, concurrency cap, failure naming, eval_one, validation, local agent).

- [ ] **Step 9: Gate and commit**

Run: `cargo fmt --all && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib`

```bash
git add -A src
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(eval): Provider::evaluate with per-question chat emulation, Client::eval

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 4: TypeSafe provider (`src/typesafe.rs`)

**Files:**
- Create: `src/typesafe.rs`
- Modify: `src/lib.rs` (`pub mod typesafe;` after `pub mod terminal;`)
- Modify: `src/config.rs` (`ProviderKind::TypeSafe` variant, serde, `FromStr`, `Display`, `supports_task`)
- Modify: `src/client.rs` (`create_provider_from_node` ~1010, `ClientBuilder::build` ~865 plus a `typesafe()` setter, `resolve_auth_api_key` unchanged)
- Modify: `src/discover.rs` (`discover_env_keys`)

**Interfaces:**
- Consumes: Task 1 types and `validate_questions`, `choice_answer`; `ClientError::Unsupported`; `resolve_auth_api_key(&Auth, &str)`.
- Produces:
  - `ProviderKind::TypeSafe` (`"typesafe"`), `supports_task` true only for `"eval"`
  - `pub struct TypeSafeClient` with `pub fn new(api_key: impl Into<String>, model: impl Into<String>, endpoint: Option<String>) -> Self` and `pub fn with_node_id(self, id: impl Into<String>) -> Self`
  - `pub const DEFAULT_MODEL: &str = "jev-latest"; pub const DEFAULT_ENDPOINT: &str = "https://api.typesafe.ai"; pub const ENV_VAR: &str = "TYPESAFE_API_KEY";`
  - `ClientBuilder::typesafe(self) -> Self`

- [ ] **Step 1: Write the failing tests**

Create `src/typesafe.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{Question, Questions};
    use serde_json::json;

    fn questions() -> Questions {
        let mut qs = Questions::new();
        qs.insert(
            "urgent".into(),
            Question::yes_no("Urgent?").yes("time-sensitive").no("no rush"),
        );
        qs.insert(
            "team".into(),
            Question::choice("Which team?").option("billing", "payments").option_bare("other"),
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
        assert_eq!((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens), (462, 71, 533));
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
        let msg = format_error(401, Some("req_1"), body, Some("typesafe/jev-latest"), "jev-latest");
        assert!(msg.contains("TYPESAFE_API_KEY"), "{msg}");
        assert!(msg.contains("ailloy ai config set-key typesafe/jev-latest"), "{msg}");
        assert!(msg.contains("req_1"), "{msg}");
    }

    #[test]
    fn formats_unknown_model() {
        let body = r#"{"detail":{"error_type":"api_usage_error","message":"Unknown model: no-such-model"}}"#;
        let msg = format_error(400, None, body, None, "no-such-model");
        assert!(msg.contains("no-such-model") && msg.contains("jev-latest"), "{msg}");
    }

    #[test]
    fn formats_validation_list_with_question_id() {
        let body = r#"{"detail":[{"type":"missing","loc":["body","questions","q","choice","criteria"],"msg":"Field required"}]}"#;
        let msg = format_error(422, None, body, None, "jev-latest");
        assert!(msg.contains("'q'") && msg.contains("Field required"), "{msg}");
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
        assert_eq!(retry_delay(0, Some("garbage")), std::time::Duration::from_millis(500));
        assert_eq!(retry_delay(0, Some("600")), std::time::Duration::from_secs(30), "capped");
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
        let err = crate::client::Provider::chat(&client, &[crate::types::Message::user("hi")], None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("chat node"), "{err}");
    }
}
```

Note: the spec's "state with attachments is rejected" bullet needs no code. The eval API takes `serde_json::Value` state, which cannot carry binary attachments (those only exist on chat `Message`s), so there is nothing to reject. Task 9 records this in the spec.

- [ ] **Step 2: Run to verify failure**

Add `pub mod typesafe;` to `src/lib.rs`.
Run: `cargo test --lib typesafe::tests`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

Insert above the tests:

```rust
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
        (_, Some(Value::String(s))) => format!("TypeSafe rejected the request (HTTP {status}): {s}"),
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

    async fn chat(&self, _messages: &[Message], _options: Option<&ChatOptions>) -> Result<ChatResponse> {
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
        let body = request_body(&self.model, state, questions);
        let raw = with_retries(MAX_ATTEMPTS, Duration::ZERO, || {
            let request = self
                .client
                .post(&url)
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
```

Borrow note: the closure passed to `with_retries` captures `url` and `body` by reference and builds a fresh `RequestBuilder` each attempt; the inner `async move` moves only the per-attempt `request` and a `url` reference. If the borrow checker complains about `url` inside `async move`, bind `let url = url.as_str();` before the closure and use that.

- [ ] **Step 4: Run TypeSafe unit tests**

Run: `cargo test --lib typesafe::tests`
Expected: PASS.

- [ ] **Step 5: Wire `ProviderKind::TypeSafe` (write failing tests first)**

Append to the tests module in `src/config.rs`:

```rust
    #[test]
    fn typesafe_provider_kind() {
        assert_eq!("typesafe".parse::<ProviderKind>().unwrap(), ProviderKind::TypeSafe);
        assert_eq!(ProviderKind::TypeSafe.to_string(), "typesafe");
        assert_eq!(ProviderKind::TypeSafe.supported_capabilities(), vec![Capability::Eval]);
        assert!(!ProviderKind::TypeSafe.supports_task("chat"));
        let yaml = "provider: typesafe\nmodel: jev-latest\n";
        let node: AiNode = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(node.provider, ProviderKind::TypeSafe);
    }
```

Append to the tests module in `src/client.rs`:

```rust
    #[test]
    fn typesafe_node_builds_a_provider() {
        let mut node = AiNode::new(ProviderKind::TypeSafe);
        node.auth = Some(Auth::ApiKey("k".into()));
        let provider = create_provider_from_node("typesafe/jev-latest", &node).unwrap();
        assert_eq!(provider.name(), "typesafe");
    }

    #[test]
    fn typesafe_node_without_key_has_actionable_error() {
        unsafe { std::env::remove_var("TYPESAFE_API_KEY") };
        let node = AiNode::new(ProviderKind::TypeSafe);
        let err = match create_provider_from_node("typesafe/jev-latest", &node) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("TYPESAFE_API_KEY"), "{err}");
    }
```

Run: `cargo test --lib typesafe_`
Expected: FAIL to compile (`no variant TypeSafe`).

- [ ] **Step 6: Implement the wiring**

`src/config.rs`:
- Add variant `#[serde(rename = "typesafe")] TypeSafe,` after `LocalAgent`.
- `FromStr`: `"typesafe" => Ok(Self::TypeSafe),` and append `, typesafe` to the valid list in the error.
- `Display`: `Self::TypeSafe => write!(f, "typesafe"),`.
- `supports_task`: change the first arm from `(_, "chat")` to
  ```rust
  (
      Self::OpenAi
          | Self::Anthropic
          | Self::AzureOpenAi
          | Self::MicrosoftFoundry
          | Self::VertexAi
          | Self::Ollama
          | Self::LocalAgent,
      "chat"
  )
  ```
  and add `| Self::TypeSafe` to the `"eval"` arm.

`src/client.rs` `create_provider_from_node`, add an arm:

```rust
        ProviderKind::TypeSafe => {
            let api_key = match &node.auth {
                Some(auth) => resolve_auth_api_key(auth, node_id)?,
                None => std::env::var(crate::typesafe::ENV_VAR).with_context(|| {
                    format!(
                        "No auth configured for TypeSafe node '{node_id}'. Set TYPESAFE_API_KEY \
                         or run `ailloy ai config set-key {node_id}`."
                    )
                })?,
            };
            let model = node
                .model
                .clone()
                .unwrap_or_else(|| crate::typesafe::DEFAULT_MODEL.to_string());
            Ok(Box::new(
                crate::typesafe::TypeSafeClient::new(api_key, model, node.endpoint.clone())
                    .with_node_id(node_id),
            ))
        }
```

`ClientBuilder`: add a setter next to the existing `openai()`/`anthropic()` setters (copy their exact shape, which sets `self.kind`):

```rust
    pub fn typesafe(mut self) -> Self {
        self.kind = Some(ProviderKind::TypeSafe);
        self
    }
```

and an arm in `build`:

```rust
            ProviderKind::TypeSafe => {
                let api_key = self
                    .api_key
                    .or_else(|| std::env::var(crate::typesafe::ENV_VAR).ok())
                    .context("API key required for TypeSafe (set TYPESAFE_API_KEY)")?;
                let model = self
                    .model
                    .unwrap_or_else(|| crate::typesafe::DEFAULT_MODEL.to_string());
                Box::new(crate::typesafe::TypeSafeClient::new(api_key, model, self.endpoint))
            }
```

`src/discover.rs` `discover_env_keys`, after the Anthropic block:

```rust
    if std::env::var(crate::typesafe::ENV_VAR).is_ok() {
        let mut node = AiNode::new(ProviderKind::TypeSafe);
        node.capabilities = vec![Capability::Eval];
        node.auth = Some(Auth::Env(crate::typesafe::ENV_VAR.to_string()));
        node.model = Some(crate::typesafe::DEFAULT_MODEL.to_string());
        results.push(DiscoveredNode {
            suggested_id: format!("typesafe/{}", crate::typesafe::DEFAULT_MODEL),
            node,
            description: "TYPESAFE_API_KEY is set".to_string(),
        });
    }
```

Then run `cargo build`. Adding the variant makes the TUI's exhaustive `match provider` blocks fail to compile. This task must end green, so apply **Task 5 Step 2** (the `src/tui/forms.rs` and `src/tui/mod.rs` arms) now, exactly as written there. Task 5 then adds the tests for those arms plus the connectivity-test and `ai test --all` behavior.

- [ ] **Step 7: Run tests and gate**

Run: `cargo fmt --all && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add -A src
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(typesafe): TypeSafe provider with native eval, retries and actionable errors

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 5: Config TUI, connectivity tests and `ai test --all`

**Files:**
- Modify: `src/tui/forms.rs` (`PROVIDER_ORDER` 55-64, `auth_options` 166-177, `default_auth_index` 180-188, `env_var_for` 191-196, `to_node` provider match ~330, `build_fields` match ~510, initial capabilities ~611-620)
- Modify: `src/tui/mod.rs` (`run_test` 187-221)
- Modify: `src/commands/ai.rs` (`run_test_all` 189-240)

**Interfaces:**
- Consumes: `ProviderKind::TypeSafe`, `Capability::Eval`, `Client::eval_one`, `Question::yes_no`, `typesafe::DEFAULT_MODEL`, `typesafe::ENV_VAR`.
- Produces: TypeSafe selectable in the add-node form; `fn connectivity_probe(node: &AiNode) -> Probe` shared shape is NOT introduced (keep the two call sites independent, matching current code).

- [ ] **Step 1: Write failing tests in `src/tui/forms.rs` tests module**

```rust
    #[test]
    fn typesafe_form_builds_node() {
        let mut form = NodeForm::new();
        form.provider = ProviderKind::TypeSafe;
        form.fields = build_fields(&ProviderKind::TypeSafe, false, None);
        let (id, node) = form.to_node().unwrap();
        assert_eq!(id, "typesafe/jev-latest");
        assert_eq!(node.provider, ProviderKind::TypeSafe);
        assert_eq!(node.capabilities, vec![Capability::Eval]);
        assert_eq!(node.auth, Some(Auth::Env("TYPESAFE_API_KEY".into())));
        assert_eq!(node.model.as_deref(), Some("jev-latest"));
        assert_eq!(node.endpoint, None, "default endpoint is not stored");
    }

    #[test]
    fn typesafe_is_offered_in_provider_order() {
        assert!(PROVIDER_ORDER.contains(&ProviderKind::TypeSafe));
    }
```

Check how existing tests in this module switch provider (see the tests at ~743-832, e.g. `form.provider = ProviderKind::Anthropic;`); if they call a method such as `form.set_provider(...)` instead of assigning `fields`, use that method in the test above instead of the two assignment lines.

Run: `cargo test --lib typesafe_form`
Expected: FAIL (non-exhaustive match or assertion failure).

- [ ] **Step 2: Implement in `src/tui/forms.rs`** (already done if Task 4 applied it; verify and skip)

- `PROVIDER_ORDER`: change type to `[ProviderKind; 8]`, append `ProviderKind::TypeSafe`, update the doc comment to "The eight configurable provider kinds".
- `auth_options`: add `ProviderKind::TypeSafe` to the `OpenAi | Anthropic` arm (env, api_key, keychain).
- `default_auth_index`: add `ProviderKind::TypeSafe` to the env-default arm (`=> 0`).
- `env_var_for`: add `ProviderKind::TypeSafe => "TYPESAFE_API_KEY",` above the `_` arm.
- `build_fields` match, add:
  ```rust
        ProviderKind::TypeSafe => {
            fields.push(FormField::text(
                FieldKey::Model,
                "model",
                if model.is_empty() {
                    crate::typesafe::DEFAULT_MODEL.to_string()
                } else {
                    model.clone()
                },
            ));
            fields.push(FormField::text(
                FieldKey::Endpoint,
                "endpoint (optional, blank = https://api.typesafe.ai)",
                endpoint.unwrap_or_default(),
            ));
        }
  ```
- `to_node` match, add:
  ```rust
            ProviderKind::TypeSafe => {
                let model = self.require(FieldKey::Model, "model (e.g. jev-latest)")?;
                node.model = Some(model.clone());
                let endpoint = self.text_of(FieldKey::Endpoint);
                node.endpoint = (!endpoint.is_empty()
                    && endpoint.trim_end_matches('/') != crate::typesafe::DEFAULT_ENDPOINT)
                    .then_some(endpoint);
                format!("typesafe/{model}")
            }
  ```
- Initial capabilities (~611): before the `match prefill`, the `None` branch should pick `vec![Capability::Eval]` when `provider == &ProviderKind::TypeSafe`:
  ```rust
        None => {
            if *provider == ProviderKind::TypeSafe {
                vec![Capability::Eval]
            } else if model.is_empty() {
                vec![Capability::Chat]
            } else {
                capabilities_for_deployment(&model)
            }
        }
  ```
- Any other `match provider` in `src/tui/` that the compiler flags (e.g. `src/tui/mod.rs:271` discovery dispatch): add `ProviderKind::TypeSafe` to the arm that means "no discovery available" (the same arm `OpenAi` uses).

- [ ] **Step 3: TypeSafe-aware connectivity test in `src/tui/mod.rs`**

In `run_test`, replace the `Ok(client) => { ... }` body with:

```rust
            Ok(client) => {
                if node.provider == ProviderKind::TypeSafe {
                    let fut = client.eval_one(
                        "The sky is blue.",
                        crate::eval::Question::yes_no("Does the text mention a color?"),
                    );
                    match tokio::time::timeout(TEST_TIMEOUT, fut).await {
                        Ok(Ok(answer)) => Ok(format!(
                            "eval works (p(yes) = {:.2})",
                            answer.as_yes_no().unwrap_or_default()
                        )),
                        Ok(Err(e)) => Err(format!("{e:#}")),
                        Err(_) => Err(format!("timed out after {}s", TEST_TIMEOUT.as_secs())),
                    }
                } else {
                    let messages = [Message::user("Say hello in one sentence.")];
                    let fut = client.chat(&messages);
                    match tokio::time::timeout(TEST_TIMEOUT, fut).await {
                        Ok(Ok(resp)) => Ok(resp.content),
                        Ok(Err(e)) => Err(format!("{e:#}")),
                        Err(_) => Err(format!("timed out after {}s", TEST_TIMEOUT.as_secs())),
                    }
                }
            }
```

(Add `ProviderKind` to the file's `use crate::config::{...}` if it is not imported.)

- [ ] **Step 4: `ai test --all` in `src/commands/ai.rs`**

In `run_test_all`, add a branch before the `else if node.capabilities.contains(&Capability::Embedding)`:

```rust
        } else if node.capabilities.contains(&Capability::Eval) {
            match ailloy::Client::with_node(id) {
                Ok(client) => client
                    .eval_one(
                        "The sky is blue.",
                        ailloy::Question::yes_no("Does the text mention a color?"),
                    )
                    .await
                    .map(|_| "eval".to_string()),
                Err(e) => Err(e),
            }
```

Update the function's doc comment: "Ping every configured node: 1-token chat for chat-capable nodes, a one-question eval for eval-only nodes (TypeSafe), a tiny embed for embedding-capable ones."

- [ ] **Step 5: Run tests and gate**

Run: `cargo fmt --all && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib`
Expected: PASS.

- [ ] **Step 6: Manual TUI smoke check**

Run: `cargo run -- ai config`, press `a`, cycle the provider selector to TypeSafe, confirm the fields read `model` (prefilled `jev-latest`), `endpoint (optional...)`, auth `env`, capability `eval` checked. Press Esc without saving. Report what you saw in the task summary.

- [ ] **Step 7: Commit**

```bash
git add -A src
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(typesafe): config dashboard, connectivity test and ai test --all support

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 6: `ailloy eval` CLI arguments

**Files:**
- Modify: `src/cli.rs` (`EvalArgs` ~205-256 and the `Eval` doc comment at 57-61)
- Modify: `src/main.rs` (`Commands::Eval` at 76-89)

**Interfaces:**
- Produces (in `src/cli.rs`):

```rust
pub struct EvalArgs {
    pub input: Option<String>,
    pub file: Option<String>,
    pub context: Option<String>,
    pub node: Option<String>,
    pub json: bool,
    pub yes_no: Option<String>,
    pub yes_means: Option<String>,
    pub no_means: Option<String>,
    pub threshold: Option<f64>,
    pub choice: Option<String>,
    pub option: Vec<String>,
    pub expect: Vec<String>,
    pub score: Option<String>,
    pub level: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub questions: Option<String>,
    pub min_confidence: Option<f64>,
}
```

Deviation from the spec, deliberate: the spec's `--true`/`--false` flags and `true:`/`false:` YAML keys become `--yes-means`/`--no-means` and `yes_means:`/`no_means:`. In YAML 1.2 (serde_yaml 0.9) an unquoted `true:` key is a boolean, not a string, so it cannot deserialize into a named field; the flags are renamed to match. Task 9 updates the spec text.

- [ ] **Step 1: Write the failing CLI parsing tests**

Append to the tests module in `src/cli.rs` (create `#[cfg(test)] mod tests { use super::*; use clap::Parser; }` at the end of the file if none exists; the top-level parser is `Cli` with a non-optional `command: Commands` field):

```rust
    fn eval_args(argv: &[&str]) -> Result<EvalArgs, clap::Error> {
        let mut full = vec!["ailloy", "eval"];
        full.extend_from_slice(argv);
        match Cli::try_parse_from(full)?.command {
            Commands::Eval(args) => Ok(args),
            _ => panic!("not an eval command"),
        }
    }

    #[test]
    fn eval_requires_exactly_one_mode() {
        assert!(eval_args(&["text"]).is_err());
        assert!(eval_args(&["text", "--yes-no", "a?", "--score", "b?"]).is_err());
        let a = eval_args(&["text", "--yes-no", "ok?", "--threshold", "0.8"]).unwrap();
        assert_eq!(a.yes_no.as_deref(), Some("ok?"));
        assert_eq!(a.threshold, Some(0.8));
    }

    #[test]
    fn eval_choice_and_score_flags_repeat() {
        let a = eval_args(&[
            "t", "--choice", "team?", "--option", "billing=payments", "--option", "other",
            "--expect", "billing",
        ])
        .unwrap();
        assert_eq!(a.option, vec!["billing=payments", "other"]);
        assert_eq!(a.expect, vec!["billing"]);
        let a = eval_args(&["t", "--score", "mood?", "--level", "Calm", "--level", "Angry", "--max", "1.0"]).unwrap();
        assert_eq!(a.level.len(), 2);
        assert_eq!(a.max, Some(1.0));
    }

    #[test]
    fn eval_criteria_flag_is_gone() {
        assert!(eval_args(&["t", "-c", "x"]).is_err());
    }
```

Do not run these yet: the binary crate cannot compile until Task 7 rewrites `commands::eval::run` to accept `cli::EvalArgs`. They run in Task 7 Step 4.

- [ ] **Step 2: Replace `EvalArgs` in `src/cli.rs`**

Replace the `Eval(EvalArgs)` doc comment with:

```rust
    /// Ask typed questions about input with an AI judge (exit 0 pass, 1 fail, 4 unsure)
    ///
    /// Built for scripts and integration tests:
    ///   my-tool run | ailloy eval --yes-no "Does the output mention the order id?"
    Eval(EvalArgs),
```

Replace the `#[command(after_help = ...)]` examples and the struct (keep the existing `after_help` attribute style used on the struct; new examples text below):

```rust
#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("mode")
        .required(true)
        .args(["yes_no", "choice", "score", "questions"])
))]
#[command(after_help = "Examples:
  # Yes/no (exit 0 when p(yes) >= threshold, default 0.5)
  my-tool run | ailloy eval --yes-no \"Does the output mention the order id?\"
  ailloy eval \"$out\" --yes-no \"Is the tone polite?\" --threshold 0.8

  # Choice (exit 1 unless the answer is one of --expect)
  ailloy eval -f ticket.txt --choice \"Which team should handle this?\" \\
    --option billing=\"payments, refunds\" --option technical=\"bugs, outages\" --expect billing

  # Score (exit 1 when outside --min/--max)
  ailloy eval \"$reply\" --score \"How frustrated is the customer?\" \\
    --level Calm --level Frustrated --level \"Very angry\" --max 1.0

  # Many questions over one input, JSON out
  ailloy eval -f ticket.txt --questions checks.yaml --json

Exit codes: 0 pass, 1 a gate failed, 2 usage/config error, 3 provider error,
4 gates passed but an answer is below --min-confidence")]
pub struct EvalArgs {
    /// The input to evaluate (or pipe via stdin / use --file)
    pub input: Option<String>,

    /// Read the input to evaluate from a file
    #[arg(short, long)]
    pub file: Option<String>,

    /// Extra context for the judge (what produced the input, expectations)
    #[arg(long)]
    pub context: Option<String>,

    /// Judge node (defaults to defaults.eval, then the default chat node)
    #[arg(short, long, add = clap_complete::engine::ArgValueCandidates::new(complete_node_ids))]
    pub node: Option<String>,

    /// Print the answers as JSON
    #[arg(long)]
    pub json: bool,

    /// Ask a yes/no question
    #[arg(long, value_name = "QUESTION")]
    pub yes_no: Option<String>,

    /// What a yes means (with --yes-no)
    #[arg(long, requires = "yes_no", value_name = "TEXT")]
    pub yes_means: Option<String>,

    /// What a no means (with --yes-no)
    #[arg(long, requires = "yes_no", value_name = "TEXT")]
    pub no_means: Option<String>,

    /// Pass when p(yes) >= threshold, 0.0-1.0 (with --yes-no; default 0.5)
    #[arg(short, long, requires = "yes_no")]
    pub threshold: Option<f64>,

    /// Ask a choice question
    #[arg(long, value_name = "QUESTION")]
    pub choice: Option<String>,

    /// A choice option, `key` or `key=description` (repeatable, with --choice)
    #[arg(long, requires = "choice", value_name = "KEY[=DESC]")]
    pub option: Vec<String>,

    /// Pass only when the chosen option is one of these (repeatable, with --choice)
    #[arg(long, requires = "choice", value_name = "KEY")]
    pub expect: Vec<String>,

    /// Ask a score question
    #[arg(long, value_name = "QUESTION")]
    pub score: Option<String>,

    /// A score level, lowest first (repeatable, 2-10, with --score)
    #[arg(long, requires = "score", value_name = "TEXT")]
    pub level: Vec<String>,

    /// Fail when the score is below this (with --score)
    #[arg(long, requires = "score")]
    pub min: Option<f64>,

    /// Fail when the score is above this (with --score)
    #[arg(long, requires = "score")]
    pub max: Option<f64>,

    /// Read several questions from a YAML or JSON file
    #[arg(long, value_name = "FILE")]
    pub questions: Option<String>,

    /// Exit 4 when an answer's confidence is below this (0.0-1.0)
    #[arg(long, value_name = "F")]
    pub min_confidence: Option<f64>,
}
```

If `EvalArgs` currently derives something other than `Args` (check the existing derive line), keep that derive.

- [ ] **Step 3: Update `src/main.rs`**

Replace the `Commands::Eval(args)` arm with:

```rust
        Commands::Eval(args) => {
            let code = commands::eval::run(args).await;
            std::process::exit(code as i32);
        }
```

Task 7 changes `commands::eval::run` to take `crate::cli::EvalArgs` directly. Tasks 6 and 7 are implemented back to back and verified together in Task 7 Steps 4-5; they are committed as two commits in Task 7 Step 7. Task 6 stays a separate review unit (its diff is `src/cli.rs` + `src/main.rs`).

---

### Task 7: `ailloy eval` command logic (`src/commands/eval.rs`)

**Files:**
- Rewrite: `src/commands/eval.rs`

**Interfaces:**
- Consumes: `crate::cli::EvalArgs` (Task 6), `ailloy::{Client, Question, Questions, Answer, EvalResponse, Calibration}`.
- Produces (pub within the binary crate, for tests):
  - `pub const EXIT_PASS: u8 = 0; EXIT_FAIL = 1; EXIT_USAGE = 2; EXIT_PROVIDER = 3; EXIT_UNSURE = 4;`
  - `pub enum Gate { YesNo { threshold: f64 }, Choice { expect: Vec<String> }, Score { min: Option<f64>, max: Option<f64> } }`
  - `pub struct Item { pub id: String, pub question: Question, pub gate: Gate, pub min_confidence: Option<f64> }`
  - `pub enum Outcome { Pass, Fail, Unsure }`
  - `pub fn parse_option(raw: &str) -> (String, Option<String>)`
  - `pub fn items_from_flags(args: &EvalArgs) -> Result<Vec<Item>, String>`
  - `pub fn parse_questions_file(text: &str, json: bool, global_min_confidence: Option<f64>) -> Result<Vec<Item>, String>`
  - `pub fn judge(item: &Item, answer: &Answer) -> Outcome`
  - `pub fn exit_code(outcomes: &[Outcome]) -> u8`
  - `pub fn build_state(input: &str, context: Option<&str>) -> serde_json::Value`
  - `pub fn render_json(resp: &EvalResponse, items: &[Item], outcomes: &[Outcome]) -> serde_json::Value`
  - `pub fn render_text(resp: &EvalResponse, items: &[Item], outcomes: &[Outcome], batch: bool) -> String`
  - `pub async fn run(args: EvalArgs) -> u8`

- [ ] **Step 1: Write the failing tests**

Replace the whole of `src/commands/eval.rs` with the test module first (implementation goes above it in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn args() -> EvalArgs {
        EvalArgs {
            input: None,
            file: None,
            context: None,
            node: None,
            json: false,
            yes_no: None,
            yes_means: None,
            no_means: None,
            threshold: None,
            choice: None,
            option: vec![],
            expect: vec![],
            score: None,
            level: vec![],
            min: None,
            max: None,
            questions: None,
            min_confidence: None,
        }
    }

    #[test]
    fn option_parsing() {
        assert_eq!(parse_option("billing=payments, refunds"), ("billing".into(), Some("payments, refunds".into())));
        assert_eq!(parse_option("other"), ("other".into(), None));
        assert_eq!(parse_option(" a = b "), ("a".into(), Some("b".into())));
    }

    #[test]
    fn yes_no_flags_build_one_item() {
        let mut a = args();
        a.yes_no = Some("ok?".into());
        a.yes_means = Some("clearly".into());
        a.threshold = Some(0.8);
        a.min_confidence = Some(0.5);
        let items = items_from_flags(&a).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "yes_no");
        assert!(matches!(items[0].gate, Gate::YesNo { threshold } if threshold == 0.8));
        assert_eq!(items[0].min_confidence, Some(0.5));
    }

    #[test]
    fn threshold_out_of_range_is_usage_error() {
        let mut a = args();
        a.yes_no = Some("ok?".into());
        a.threshold = Some(1.5);
        assert!(items_from_flags(&a).unwrap_err().contains("--threshold"));
    }

    #[test]
    fn choice_expect_must_be_an_option() {
        let mut a = args();
        a.choice = Some("team?".into());
        a.option = vec!["billing".into(), "technical".into()];
        a.expect = vec!["sales".into()];
        let err = items_from_flags(&a).unwrap_err();
        assert!(err.contains("sales") && err.contains("billing"), "{err}");
    }

    #[test]
    fn choice_needs_two_options() {
        let mut a = args();
        a.choice = Some("team?".into());
        a.option = vec!["billing".into()];
        assert!(items_from_flags(&a).unwrap_err().contains("--option"));
    }

    #[test]
    fn score_needs_two_levels() {
        let mut a = args();
        a.score = Some("mood?".into());
        a.level = vec!["Calm".into()];
        assert!(items_from_flags(&a).unwrap_err().contains("--level"));
    }

    const FILE: &str = r#"
questions:
  mentions_order:
    yes_no: "Does the output mention the order id?"
    yes_means: "The order id appears verbatim"
    threshold: 0.8
  team:
    choice: "Which team should handle this?"
    options: { billing: "payments, refunds", technical: "bugs, outages", other: null }
    expect: [billing]
  frustration:
    score: "How frustrated is the customer?"
    levels: [Calm, Frustrated, Very angry]
    max: 1.0
    min_confidence: 0.6
"#;

    #[test]
    fn questions_file_yaml() {
        let items = parse_questions_file(FILE, false, Some(0.3)).unwrap();
        let by_id: BTreeMap<_, _> = items.iter().map(|i| (i.id.as_str(), i)).collect();
        assert!(matches!(by_id["mentions_order"].gate, Gate::YesNo { threshold } if threshold == 0.8));
        assert!(matches!(&by_id["team"].gate, Gate::Choice { expect } if expect == &vec!["billing".to_string()]));
        assert_eq!(by_id["team"].min_confidence, Some(0.3), "global floor applies");
        assert_eq!(by_id["frustration"].min_confidence, Some(0.6), "per-question floor wins");
    }

    #[test]
    fn questions_file_json() {
        let json = r#"{"questions": {"q": {"yes_no": "ok?"}}}"#;
        let items = parse_questions_file(json, true, None).unwrap();
        assert!(matches!(items[0].gate, Gate::YesNo { threshold } if threshold == 0.5));
    }

    #[test]
    fn questions_file_rejects_mixed_entry() {
        let bad = "questions:\n  q:\n    yes_no: \"ok?\"\n    choice: \"pick\"\n";
        let err = parse_questions_file(bad, false, None).unwrap_err();
        assert!(err.contains("'q'") && err.contains("exactly one"), "{err}");
        let none = "questions:\n  q:\n    threshold: 0.5\n";
        assert!(parse_questions_file(none, false, None).unwrap_err().contains("'q'"));
        let stray = "questions:\n  q:\n    yes_no: \"ok?\"\n    options: {a: null, b: null}\n";
        assert!(parse_questions_file(stray, false, None).unwrap_err().contains("'q'"));
    }

    #[test]
    fn questions_file_rejects_unknown_expect() {
        let bad = "questions:\n  t:\n    choice: \"pick\"\n    options: {a: null, b: null}\n    expect: [c]\n";
        let err = parse_questions_file(bad, false, None).unwrap_err();
        assert!(err.contains("'t'") && err.contains("'c'"), "{err}");
    }

    #[test]
    fn questions_file_rejects_unknown_fields_and_empty() {
        assert!(parse_questions_file("questions:\n  q:\n    yes_no: x\n    bogus: 1\n", false, None).is_err());
        assert!(parse_questions_file("questions: {}\n", false, None).unwrap_err().contains("no questions"));
    }

    fn item(gate: Gate, min_confidence: Option<f64>) -> Item {
        Item { id: "q".into(), question: Question::yes_no("x"), gate, min_confidence }
    }

    #[test]
    fn gates() {
        let yes = Answer::YesNo { probability: 0.9 };
        assert_eq!(judge(&item(Gate::YesNo { threshold: 0.5 }, None), &yes), Outcome::Pass);
        assert_eq!(judge(&item(Gate::YesNo { threshold: 0.95 }, None), &yes), Outcome::Fail);
        // confidence of p=0.9 is 0.8
        assert_eq!(judge(&item(Gate::YesNo { threshold: 0.5 }, Some(0.85)), &yes), Outcome::Unsure);

        let mut probs = BTreeMap::new();
        probs.insert("billing".to_string(), 0.8);
        probs.insert("other".to_string(), 0.2);
        let choice = ailloy::eval::choice_answer(probs);
        assert_eq!(judge(&item(Gate::Choice { expect: vec![] }, None), &choice), Outcome::Pass);
        assert_eq!(judge(&item(Gate::Choice { expect: vec!["other".into()] }, None), &choice), Outcome::Fail);

        let score = ailloy::eval::score_answer(vec![0.0, 0.5, 0.5]);
        assert_eq!(judge(&item(Gate::Score { min: None, max: Some(1.0) }, None), &score), Outcome::Fail);
        assert_eq!(judge(&item(Gate::Score { min: Some(1.0), max: None }, None), &score), Outcome::Pass);
    }

    #[test]
    fn a_failed_gate_wins_over_low_confidence() {
        let yes = Answer::YesNo { probability: 0.55 };
        assert_eq!(judge(&item(Gate::YesNo { threshold: 0.9 }, Some(0.9)), &yes), Outcome::Fail);
    }

    #[test]
    fn exit_code_precedence() {
        assert_eq!(exit_code(&[Outcome::Pass, Outcome::Pass]), EXIT_PASS);
        assert_eq!(exit_code(&[Outcome::Pass, Outcome::Unsure]), EXIT_UNSURE);
        assert_eq!(exit_code(&[Outcome::Unsure, Outcome::Fail]), EXIT_FAIL);
    }

    #[test]
    fn state_shape() {
        assert_eq!(build_state("x", None), serde_json::json!("x"));
        assert_eq!(
            build_state("x", Some("ctx")),
            serde_json::json!({"context": "ctx", "input": "x"})
        );
    }

    fn response() -> EvalResponse {
        let mut answers = BTreeMap::new();
        answers.insert("q".to_string(), Answer::YesNo { probability: 0.9 });
        let mut rationale = BTreeMap::new();
        rationale.insert("q".to_string(), "clear".to_string());
        EvalResponse {
            answers,
            model: "gpt-x".into(),
            usage: None,
            calibration: ailloy::Calibration::SelfReported,
            rationale,
        }
    }

    #[test]
    fn json_output_shape() {
        let items = vec![item(Gate::YesNo { threshold: 0.5 }, None)];
        let v = render_json(&response(), &items, &[Outcome::Pass]);
        assert_eq!(v["model"], "gpt-x");
        assert_eq!(v["calibration"], "self_reported");
        assert_eq!(v["pass"], true);
        assert_eq!(v["answers"]["q"]["type"], "yes_no");
        assert_eq!(v["answers"]["q"]["probability"], 0.9);
        assert_eq!(v["answers"]["q"]["pass"], true);
        assert_eq!(v["answers"]["q"]["outcome"], "pass");
        assert_eq!(v["answers"]["q"]["rationale"], "clear");
        assert!(v.get("usage").is_some());
    }

    #[test]
    fn text_output_marks_self_reported_and_rationale() {
        colored::control::set_override(false);
        let items = vec![item(Gate::YesNo { threshold: 0.5 }, None)];
        let text = render_text(&response(), &items, &[Outcome::Pass], false);
        assert!(text.contains("PASS") && text.contains("p=0.90"), "{text}");
        assert!(text.contains("(gpt-x, self-reported)"), "{text}");
        assert!(text.contains("clear"), "{text}");
    }
}
```

Run: `cargo test --bin ailloy eval`
Expected: FAIL to compile.

- [ ] **Step 2: Write the implementation**

Insert above the tests:

```rust
//! `ailloy eval`: typed questions about input, with script-friendly exit codes.
//!
//! ```bash
//! my-tool run | ailloy eval --yes-no "Does the output mention the order id?"
//! ```

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};

use anyhow::{Context, Result};
use colored::Colorize;
use serde::Deserialize;
use serde_json::{Value, json};

use ailloy::{Answer, Calibration, Client, EvalResponse, Question, Questions};

use crate::cli::EvalArgs;

/// Exit codes: 0 pass, 1 fail, 2 usage/config error, 3 provider error, 4 unsure.
pub const EXIT_PASS: u8 = 0;
pub const EXIT_FAIL: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_PROVIDER: u8 = 3;
pub const EXIT_UNSURE: u8 = 4;

const DEFAULT_THRESHOLD: f64 = 0.5;

/// What decides pass or fail for one question.
#[derive(Debug, Clone, PartialEq)]
pub enum Gate {
    YesNo { threshold: f64 },
    Choice { expect: Vec<String> },
    Score { min: Option<f64>, max: Option<f64> },
}

/// One question plus its gate.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub question: Question,
    pub gate: Gate,
    pub min_confidence: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail,
    Unsure,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unsure => "unsure",
        }
    }
}

/// `key=description` or bare `key`.
pub fn parse_option(raw: &str) -> (String, Option<String>) {
    match raw.split_once('=') {
        Some((key, desc)) => (key.trim().to_string(), Some(desc.trim().to_string())),
        None => (raw.trim().to_string(), None),
    }
}

fn check_unit(name: &str, value: Option<f64>) -> Result<(), String> {
    match value {
        Some(v) if !(0.0..=1.0).contains(&v) => {
            Err(format!("{name} must be between 0.0 and 1.0 (got {v})"))
        }
        _ => Ok(()),
    }
}

fn choice_item(
    id: &str,
    text: &str,
    options: Vec<(String, Option<String>)>,
    expect: Vec<String>,
    min_confidence: Option<f64>,
    option_hint: &str,
) -> Result<Item, String> {
    if options.len() < 2 {
        return Err(format!(
            "choice '{id}' needs at least 2 options (use {option_hint})"
        ));
    }
    let keys: Vec<&str> = options.iter().map(|(k, _)| k.as_str()).collect();
    if let Some(bad) = expect.iter().find(|e| !keys.contains(&e.as_str())) {
        return Err(format!(
            "choice '{id}': expected option '{bad}' is not one of the options ({})",
            keys.join(", ")
        ));
    }
    let mut question = Question::choice(text);
    for (key, desc) in options {
        question = match desc {
            Some(d) => question.option(key, d),
            None => question.option_bare(key),
        };
    }
    Ok(Item {
        id: id.to_string(),
        question,
        gate: Gate::Choice { expect },
        min_confidence,
    })
}

fn score_item(
    id: &str,
    text: &str,
    levels: Vec<String>,
    min: Option<f64>,
    max: Option<f64>,
    min_confidence: Option<f64>,
    level_hint: &str,
) -> Result<Item, String> {
    if !(2..=10).contains(&levels.len()) {
        return Err(format!(
            "score '{id}' needs 2 to 10 levels, lowest first (use {level_hint})"
        ));
    }
    Ok(Item {
        id: id.to_string(),
        question: Question::score(text).levels(levels),
        gate: Gate::Score { min, max },
        min_confidence,
    })
}

fn yes_no_item(
    id: &str,
    text: &str,
    yes: Option<String>,
    no: Option<String>,
    threshold: Option<f64>,
    min_confidence: Option<f64>,
) -> Result<Item, String> {
    check_unit("--threshold / threshold", threshold)?;
    let mut question = Question::yes_no(text);
    if let Some(y) = yes {
        question = question.yes(y);
    }
    if let Some(n) = no {
        question = question.no(n);
    }
    Ok(Item {
        id: id.to_string(),
        question,
        gate: Gate::YesNo {
            threshold: threshold.unwrap_or(DEFAULT_THRESHOLD),
        },
        min_confidence,
    })
}

/// Build the single question from inline flags (`--yes-no`, `--choice`, `--score`).
pub fn items_from_flags(args: &EvalArgs) -> Result<Vec<Item>, String> {
    check_unit("--min-confidence", args.min_confidence)?;
    let item = if let Some(text) = &args.yes_no {
        yes_no_item(
            "yes_no",
            text,
            args.yes_means.clone(),
            args.no_means.clone(),
            args.threshold,
            args.min_confidence,
        )?
    } else if let Some(text) = &args.choice {
        choice_item(
            "choice",
            text,
            args.option.iter().map(|o| parse_option(o)).collect(),
            args.expect.clone(),
            args.min_confidence,
            "--option key=description, repeated",
        )?
    } else if let Some(text) = &args.score {
        score_item(
            "score",
            text,
            args.level.clone(),
            args.min,
            args.max,
            args.min_confidence,
            "--level TEXT, repeated",
        )?
    } else {
        return Err("pass one of --yes-no, --choice, --score or --questions".to_string());
    };
    Ok(vec![item])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionsFile {
    questions: BTreeMap<String, FileQuestion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileQuestion {
    yes_no: Option<String>,
    yes_means: Option<String>,
    no_means: Option<String>,
    threshold: Option<f64>,
    choice: Option<String>,
    options: Option<BTreeMap<String, Option<String>>>,
    expect: Option<Vec<String>>,
    score: Option<String>,
    levels: Option<Vec<String>>,
    min: Option<f64>,
    max: Option<f64>,
    min_confidence: Option<f64>,
}

/// Parse a `--questions` file (YAML, or JSON when `json` is true).
pub fn parse_questions_file(
    text: &str,
    json: bool,
    global_min_confidence: Option<f64>,
) -> Result<Vec<Item>, String> {
    let file: QuestionsFile = if json {
        serde_json::from_str(text).map_err(|e| format!("invalid questions file: {e}"))?
    } else {
        serde_yaml::from_str(text).map_err(|e| format!("invalid questions file: {e}"))?
    };
    if file.questions.is_empty() {
        return Err("the questions file has no questions under `questions:`".to_string());
    }
    let mut items = Vec::new();
    for (id, q) in file.questions {
        let kinds = [q.yes_no.is_some(), q.choice.is_some(), q.score.is_some()]
            .iter()
            .filter(|b| **b)
            .count();
        if kinds != 1 {
            return Err(format!(
                "question '{id}' must set exactly one of yes_no, choice, score"
            ));
        }
        check_unit(&format!("question '{id}' min_confidence"), q.min_confidence)?;
        let floor = q.min_confidence.or(global_min_confidence);
        let item = if let Some(text) = q.yes_no {
            if q.options.is_some() || q.expect.is_some() || q.levels.is_some() || q.min.is_some() || q.max.is_some() {
                return Err(format!(
                    "question '{id}' is yes_no but sets choice/score fields (options, expect, levels, min, max)"
                ));
            }
            yes_no_item(&id, &text, q.yes_means, q.no_means, q.threshold, floor)
                .map_err(|e| format!("question '{id}': {e}"))?
        } else if let Some(text) = q.choice {
            if q.threshold.is_some() || q.yes_means.is_some() || q.no_means.is_some() || q.levels.is_some() || q.min.is_some() || q.max.is_some() {
                return Err(format!(
                    "question '{id}' is a choice but sets yes_no/score fields"
                ));
            }
            choice_item(
                &id,
                &text,
                q.options.unwrap_or_default().into_iter().collect(),
                q.expect.unwrap_or_default(),
                floor,
                "options: {key: description}",
            )?
        } else {
            if q.threshold.is_some() || q.yes_means.is_some() || q.no_means.is_some() || q.options.is_some() || q.expect.is_some() {
                return Err(format!(
                    "question '{id}' is a score but sets yes_no/choice fields"
                ));
            }
            score_item(
                &id,
                q.score.as_deref().unwrap_or_default(),
                q.levels.unwrap_or_default(),
                q.min,
                q.max,
                floor,
                "levels: [lowest, ..., highest]",
            )?
        };
        items.push(item);
    }
    Ok(items)
}

/// Gate one answer. A failed gate wins over low confidence.
pub fn judge(item: &Item, answer: &Answer) -> Outcome {
    let passed = match (&item.gate, answer) {
        (Gate::YesNo { threshold }, Answer::YesNo { probability }) => probability >= threshold,
        (Gate::Choice { expect }, Answer::Choice { choice, .. }) => {
            expect.is_empty() || expect.contains(choice)
        }
        (Gate::Score { min, max }, Answer::Score { score, .. }) => {
            min.is_none_or(|m| *score >= m) && max.is_none_or(|m| *score <= m)
        }
        _ => false,
    };
    if !passed {
        return Outcome::Fail;
    }
    match item.min_confidence {
        Some(floor) if answer.confidence() < floor => Outcome::Unsure,
        _ => Outcome::Pass,
    }
}

/// 1 if any gate failed, else 4 if any answer is unsure, else 0.
pub fn exit_code(outcomes: &[Outcome]) -> u8 {
    if outcomes.contains(&Outcome::Fail) {
        EXIT_FAIL
    } else if outcomes.contains(&Outcome::Unsure) {
        EXIT_UNSURE
    } else {
        EXIT_PASS
    }
}

/// The state sent to the judge: the input, or `{context, input}`.
pub fn build_state(input: &str, context: Option<&str>) -> Value {
    match context {
        Some(ctx) => json!({"context": ctx, "input": input}),
        None => Value::String(input.to_string()),
    }
}

/// The `--json` document.
pub fn render_json(resp: &EvalResponse, items: &[Item], outcomes: &[Outcome]) -> Value {
    let mut answers = serde_json::Map::new();
    for (item, outcome) in items.iter().zip(outcomes) {
        let Some(answer) = resp.answers.get(&item.id) else { continue };
        let mut entry = serde_json::to_value(answer).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut entry {
            map.insert("confidence".into(), json!(answer.confidence()));
            if let Some(n) = answer.normalized_score() {
                map.insert("normalized_score".into(), json!(n));
            }
            map.insert("pass".into(), json!(*outcome != Outcome::Fail));
            map.insert("outcome".into(), json!(outcome.label()));
            if let Some(r) = resp.rationale.get(&item.id) {
                map.insert("rationale".into(), json!(r));
            }
        }
        answers.insert(item.id.clone(), entry);
    }
    let usage = resp.usage.as_ref().map(|u| {
        json!({"input_tokens": u.prompt_tokens, "output_tokens": u.completion_tokens})
    });
    json!({
        "model": resp.model,
        "calibration": resp.calibration,
        "pass": !outcomes.contains(&Outcome::Fail),
        "answers": answers,
        "usage": usage,
    })
}

fn model_label(resp: &EvalResponse) -> String {
    match resp.calibration {
        Calibration::Measured => format!("({})", resp.model),
        Calibration::SelfReported => format!("({}, self-reported)", resp.model),
    }
}

fn describe(item: &Item, answer: &Answer) -> String {
    match answer {
        Answer::YesNo { probability } => format!("p={probability:.2}"),
        Answer::Choice {
            choice,
            probabilities,
            ..
        } => format!(
            "{choice}  p={:.2}",
            probabilities.get(choice).copied().unwrap_or_default()
        ),
        Answer::Score {
            score,
            probabilities,
            ..
        } => {
            let top = probabilities.len().saturating_sub(1);
            let nearest = (score.round() as usize).min(top);
            let label = match &item.question {
                Question::Score { levels, .. } => levels
                    .get(nearest)
                    .map(|l| l.as_str().map(str::to_string).unwrap_or_else(|| l.to_string()))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            format!("{score:.2} of 0..{top} ({label})")
        }
        _ => String::new(),
    }
}

/// Human-readable output, one line per question.
pub fn render_text(resp: &EvalResponse, items: &[Item], outcomes: &[Outcome], batch: bool) -> String {
    let mut out = String::new();
    let model = model_label(resp);
    for (item, outcome) in items.iter().zip(outcomes) {
        let Some(answer) = resp.answers.get(&item.id) else { continue };
        let status = match outcome {
            Outcome::Pass => "PASS".green().bold(),
            Outcome::Fail => "FAIL".red().bold(),
            Outcome::Unsure => "UNSURE".yellow().bold(),
        };
        let name = if batch { item.id.clone() } else { item.question.kind().replace('_', "-") };
        out.push_str(&format!(
            "{status}  {name}  {}  confidence {:.2}  {model}\n",
            describe(item, answer),
            answer.confidence()
        ));
        if let Answer::Choice { probabilities, .. } = answer {
            let dist: Vec<String> = probabilities.iter().map(|(k, p)| format!("{k} {p:.2}")).collect();
            out.push_str(&format!("        {}\n", dist.join(" · ")));
        }
        if let Some(r) = resp.rationale.get(&item.id) {
            out.push_str(&format!("        - {r}\n"));
        }
    }
    out
}

fn usage_error(message: &str) -> u8 {
    eprintln!("{} {message}", "error:".red().bold());
    EXIT_USAGE
}

fn read_input(args: &EvalArgs) -> Result<Option<String>> {
    if let Some(input) = &args.input {
        return Ok(Some(input.clone()));
    }
    if let Some(f) = &args.file {
        return std::fs::read_to_string(f)
            .with_context(|| format!("cannot read input file {f}"))
            .map(Some);
    }
    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read stdin")?;
        return Ok(Some(buf));
    }
    Ok(None)
}

pub async fn run(args: EvalArgs) -> u8 {
    let items = match &args.questions {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Err(e) = check_unit("--min-confidence", args.min_confidence) {
                    return usage_error(&e);
                }
                parse_questions_file(&text, path.ends_with(".json"), args.min_confidence)
            }
            Err(e) => Err(format!("cannot read questions file {path}: {e}")),
        },
        None => items_from_flags(&args),
    };
    let items = match items {
        Ok(items) => items,
        Err(e) => return usage_error(&e),
    };
    let input = match read_input(&args) {
        Ok(Some(input)) if !input.trim().is_empty() => input,
        Ok(Some(_)) => return usage_error("input is empty"),
        Ok(None) => {
            return usage_error("nothing to evaluate: pass an argument, --file, or pipe stdin");
        }
        Err(e) => return usage_error(&format!("{e:#}")),
    };
    match evaluate(&args, &items, &input).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e:#}", "error:".red().bold());
            EXIT_PROVIDER
        }
    }
}

async fn evaluate(args: &EvalArgs, items: &[Item], input: &str) -> Result<u8> {
    let client = match &args.node {
        Some(node) => Client::with_node(node)?,
        None => Client::for_capability("eval")?,
    };
    let questions: Questions = items
        .iter()
        .map(|i| (i.id.clone(), i.question.clone()))
        .collect();
    let state = build_state(input, args.context.as_deref());
    let response = client.eval(state, &questions).await?;
    let outcomes: Vec<Outcome> = items
        .iter()
        .map(|item| match response.answers.get(&item.id) {
            Some(answer) => judge(item, answer),
            None => Outcome::Fail,
        })
        .collect();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&render_json(&response, items, &outcomes))?);
    } else {
        print!("{}", render_text(&response, items, &outcomes, args.questions.is_some()));
    }
    Ok(exit_code(&outcomes))
}
```

Notes for the implementer:
- `Option::is_none_or` is stable since Rust 1.82 (MSRV 1.88 is fine).
- `ailloy::eval::choice_answer`/`score_answer` are used by the tests; they are `pub` from Task 1.
- `colored::control::set_override(false)` in the test makes color codes predictable.
- Long `if` conditions above will be reflowed by `cargo fmt`; that is fine.

- [ ] **Step 3: Note on Step 1's red state**

Step 1 fails to compile (no `Item`, `Gate`, ...); Step 2 resolves it. Move on.

- [ ] **Step 4: Run all eval CLI tests (Tasks 6 and 7)**

Run: `cargo test --bin ailloy eval`
Expected: PASS (CLI parsing tests from Task 6 and logic tests from Task 7).

- [ ] **Step 5: Gate**

Run: `cargo fmt --all && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib`
Expected: PASS.

- [ ] **Step 6: Live smoke test (manual, small cost)**

```bash
cargo build --release
echo "Customer #4417: payouts failed for 3 days, furious. Order ORD-99812." > /tmp/ailloy-eval-ticket.txt
./target/release/ailloy eval -f /tmp/ailloy-eval-ticket.txt --yes-no "Does the text mention an order id?"; echo "exit $?"
./target/release/ailloy eval -f /tmp/ailloy-eval-ticket.txt --choice "Which team?" --option billing="payments, payouts" --option technical="bugs" --expect billing; echo "exit $?"
./target/release/ailloy eval "hello" --score "How angry?" --level Calm --level Angry; echo "exit $?"
./target/release/ailloy eval "x" --choice "pick" --option a; echo "exit $? (expect 2)"
```

Expected: the first three print one result line each with `(gpt-5.6-luna, self-reported)` (the default chat node) and exit 0; the last exits 2 with an actionable message. TypeSafe checks come in Task 10. Paste the output in the task summary.

- [ ] **Step 7: Commit Tasks 6 and 7 as two commits**

```bash
git add src/cli.rs src/main.rs
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(cli)!: ailloy eval takes --yes-no/--choice/--score/--questions

BREAKING CHANGE: -c/--criteria and --criteria-file are removed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
git add src/commands/eval.rs
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "feat(cli)!: rebuild ailloy eval on the eval capability with gates and exit code 4

BREAKING CHANGE: --threshold gates the yes/no probability and the JSON output shape changed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

(The first commit alone does not compile; that is acceptable inside this pair. If your reviewer requires every commit to build, squash them into one.)

---

### Task 8: Examples

**Files:**
- Create: `examples/eval.rs`
- Rewrite: `examples/eval.sh`

**Interfaces:**
- Consumes: public library API from Tasks 1-4.

- [ ] **Step 1: Write `examples/eval.rs`**

```rust
//! Typed evaluation with ailloy: one batch with all three question types,
//! gated on confidence.
//!
//! Uses `defaults.eval` from your ailloy config (a TypeSafe node gives
//! calibrated probabilities), falling back to your default chat node.
//!
//! Run: cargo run --example eval

use ailloy::{Client, Question, Questions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ticket = "Hi, my payouts have failed for three days and our finance team is \
                  furious. Order ORD-99812. Please fix this today.";

    let mut questions = Questions::new();
    questions.insert(
        "urgent".into(),
        Question::yes_no("Does the customer need a response today?"),
    );
    questions.insert(
        "team".into(),
        Question::choice("Which team should own this ticket?")
            .option("billing", "payments, payouts, invoices")
            .option("technical", "bugs and outages")
            .option_bare("other"),
    );
    questions.insert(
        "frustration".into(),
        Question::score("How frustrated is the customer?")
            .levels(["Calm", "Annoyed", "Frustrated", "Furious"]),
    );

    let client = Client::for_capability("eval")?;
    let response = client.eval(ticket, &questions).await?;

    println!("model: {} ({:?})", response.model, response.calibration);
    for (id, answer) in &response.answers {
        let confident = answer.confidence() >= 0.6;
        println!(
            "{id:<12} {answer:?}  confidence {:.2}{}",
            answer.confidence(),
            if confident { "" } else { "  -> route to a human" }
        );
    }
    Ok(())
}
```

Check `Cargo.toml` for an `[[example]]` section with `required-features`; if other examples declare one (e.g. `chat` needing `cli` for `tokio/macros`), add the same block for `eval`. If none exists and `cargo build --examples` fails on `#[tokio::main]`, add:

```toml
[[example]]
name = "eval"
required-features = ["cli"]
```

- [ ] **Step 2: Rewrite `examples/eval.sh`**

```bash
#!/usr/bin/env bash
# The integration-test pattern: judge a non-deterministic AI output.
#
# `ailloy eval` exits 0 on pass, 1 on fail, 2 on usage error, 3 on provider
# error, and 4 when the gates pass but an answer is below --min-confidence,
# so it slots straight into any test script or CI job.
set -euo pipefail

# Imagine this is your tool producing a non-deterministic answer:
answer="$(my-tool ask 'Summarize the incident report')"

# One yes/no check, with context for the judge:
echo "$answer" | ailloy eval \
  --yes-no "Does the summary give the outage start time, the root cause, and at least one follow-up action?" \
  --context "input is a summary of incident INC-4711; the report is about a database failover"

# Stricter: require p(yes) >= 0.8 and route uncertain answers to review (exit 4):
set +e
echo "$answer" | ailloy eval --yes-no "Is it written in professional English?" \
  --threshold 0.8 --min-confidence 0.6
code=$?
set -e
if [ "$code" -eq 4 ]; then echo "judge unsure, flag for human review"; elif [ "$code" -ne 0 ]; then exit "$code"; fi

# Many checks in one request (free batching on a TypeSafe node):
cat > /tmp/checks.yaml <<'YAML'
questions:
  root_cause:
    yes_no: "Does the summary name a root cause?"
  tone:
    choice: "What is the tone of the summary?"
    options: { neutral: "factual", alarmist: "exaggerated urgency", dismissive: "downplays impact" }
    expect: [neutral]
  completeness:
    score: "How complete is the summary?"
    levels: [Missing key facts, Partial, Complete]
    min: 1.5
YAML
echo "$answer" | ailloy eval --questions /tmp/checks.yaml --json
```

- [ ] **Step 3: Build examples and commit**

Run: `cargo build --examples && bash -n examples/eval.sh`
Expected: success.

```bash
git add examples Cargo.toml
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "docs(examples): eval library example and rewritten eval.sh

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 9: Documentation, version 3.0.0 and changelog

**Files:**
- Modify: `Cargo.toml` (`version = "2.2.1"` -> `"3.0.0"`), then `cargo build` to refresh `Cargo.lock`
- Modify: `CHANGELOG.md`, `README.md`, `src/doc/ai-reference.md`, `src/commands/skill.rs`, `CLAUDE.md`, `INSTALL.md` (review only), `src/lib.rs` crate docs
- Modify: `docs/superpowers/specs/2026-10-06-eval-capability-design.md` (flag rename note)

- [ ] **Step 1: Version bump**

Set `version = "3.0.0"` in `Cargo.toml`; run `cargo build` so `Cargo.lock` updates.

- [ ] **Step 2: CHANGELOG**

Add at the top (below the title/intro, above the 2.2.1 entry), using today's date:

```markdown
## [3.0.0] - 2026-10-06

### Breaking changes

#### Library

| Change | Migration |
|---|---|
| `Capability::Eval` added | add an arm or `_ =>` to exhaustive matches |
| `ProviderKind::TypeSafe` added | same |
| `Task::Evaluation` added | same |
| `Capability`, `ProviderKind` and `Task` are now `#[non_exhaustive]` | add `_ =>`; future additions are non-breaking |
| `ProviderKind::supported_capabilities()` no longer always contains `Chat` (TypeSafe is eval-only) | check capabilities before calling `chat` |
| `ALL_CAPABILITIES`, `ALL_TASKS` and `ALL_CAPABILITY_KEYS` include `eval` | none, or filter it out |
| `Provider` gains `evaluate()` with a default (chat emulation) | override it to opt a custom provider out |

#### CLI

- `ailloy eval`: `-c/--criteria` and `--criteria-file` are removed; use `--yes-no "<question>"` or `--questions <file>`.
- `--threshold` now gates the yes/no probability (default 0.5) instead of the judge's self-reported score; the judge's own pass/fail verdict is gone.
- `--json` output changed: `{model, calibration, pass, answers: {<id>: {type, ..., confidence, pass, outcome, rationale?}}, usage}`.
- New exit code 4: gates passed but an answer is below `--min-confidence`.

### Added

- `eval` capability: typed `YesNo`, `Choice` and `Score` questions over a state (`Client::eval`, `Client::eval_one`, blocking mirrors), with probabilities and confidence on every answer.
- `typesafe` provider (TypeSafe Jev): calibrated probabilities, all questions in one request, `TYPESAFE_API_KEY` discovery, config dashboard support.
- Chat nodes serve `eval` too (one structured-output call per question, at most 4 at once; answers marked self-reported).
- `ailloy eval --choice/--option/--expect`, `--score/--level/--min/--max`, `--questions <yaml|json>`, `--min-confidence`, `--yes-means/--no-means`.
- `defaults.eval` routing, falling back to the default chat node.
- `examples/eval.rs`.
```

Check the existing CHANGELOG link-reference style at the bottom of the file (e.g. `[2.2.1]: https://github.com/...compare/...`) and add a matching `[3.0.0]` line.

- [ ] **Step 3: README**

- Replace the eval section (around line 320-338) with the new command forms: the yes/no example, a choice example with `--expect`, a score example with `--max`, the `--questions` file example (same YAML as Task 8), the exit-code list (0/1/2/3/4), and a "Threshold guidance" paragraph: "Answers vary slightly between runs. Jev is mostly identical run to run (TypeSafe's cookbook measured a std dev of 0 to about 0.008); chat models varied by 0.01 to 0.12 in our measurement. Keep gates away from typical values. Batching is free on TypeSafe nodes; on chat nodes each question is its own call, so large documents with many questions belong on a TypeSafe node."
- Add TypeSafe to the providers list/table, with setup: `export TYPESAFE_API_KEY=...` and `ailloy ai config` (or the YAML node `provider: typesafe`, `model: jev-latest`, `auth: {env: TYPESAFE_API_KEY}`, `capabilities: [eval]`, plus `defaults: {eval: typesafe/jev-latest}`).
- Update the command table row (line ~442) to `ailloy eval <input> --yes-no <question>`.
- Update every `version = "2.0"` dependency snippet to `"3.0"`.
- Add a short library example (the `Client::for_capability("eval")` + `Questions` snippet from `examples/eval.rs`).

- [ ] **Step 4: `src/doc/ai-reference.md`**

Rewrite the "Eval" section (from line ~121) to cover: usage line `ailloy eval [INPUT] (--yes-no Q | --choice Q | --score Q | --questions FILE) [OPTIONS]`, a table of every flag from Task 6 with its description, the questions file format (full YAML example and the field list: `yes_no|choice|score`, `yes_means`, `no_means`, `threshold`, `options`, `expect`, `levels`, `min`, `max`, `min_confidence`), output formats (text and JSON examples from the spec section 3), exit codes, node selection (`--node`, `defaults.eval`, default chat node), TypeSafe node setup, and the threshold guidance paragraph from Step 3. Update the quick-reference lines at ~371-372 to the new flags.

- [ ] **Step 5: Skill text in `src/commands/skill.rs`**

- Description (line 23): replace "and LLM-as-judge evaluation with script-friendly exit codes" with "and typed evaluation (yes/no, choice, score; LLM or TypeSafe judge) with script-friendly exit codes", and add TypeSafe to the provider list in parentheses.
- Line 40: "`ailloy eval` gives deterministic pass/fail exit codes and JSON answers."
- Line 91: replace with two lines:
  ```
  - `cmd | ailloy eval --yes-no "question"`: judge; exit 0 pass, 1 fail, 4 unsure (`--threshold`, `--min-confidence`, `--json`)
  - `ailloy eval -f in.txt --choice "q" --option a --option b --expect a` / `--score "q" --level lo --level hi --max 1` / `--questions checks.yaml`
  ```
  (The surrounding lines in this file use em-dashes; the new lines use a colon on purpose.)
- Status line (~97): "(chat, image, video, embedding, eval)"; set-default line: `--task chat|image|video|embedding|eval`.
- If the skill tests assert specific substrings (search `src/commands/skill.rs` tests for `eval`), update them.

- [ ] **Step 6: `CLAUDE.md`**

- Architecture map: add `eval.rs` ("Public eval types: Question (YesNo/Choice/Score), Answer, Calibration, EvalResponse, validation, TypeSafe-compatible confidence math"), `eval_chat.rs` ("Chat emulation of eval (private): one strict-JSON-schema chat call per question, max 4 in flight, code computes winner/score/confidence"), `typesafe.rs` ("TypeSafe System One client (Jev): POST /v1/systemone, YesNo maps to noul, 120 s timeout, 429/529 retries, three error-body shapes, request id in errors"); update `config.rs` line to mention `Capability::Eval`, `ProviderKind::TypeSafe`, `default_eval_node`; update `commands/eval.rs` description ("typed eval with gates, questions file, exit codes 0/1/2/3/4").
- Key Patterns: add an **Eval capability** bullet: question types, `Provider::evaluate` default = chat emulation (per question, because batching changes chat answers, measured 2026-10-06), TypeSafe batches everything, `defaults.eval` then default chat node, `#[non_exhaustive]` on `Capability`/`ProviderKind`/`Task`/`Question`/`Answer`.
- Update the Provider trait bullet to list `evaluate()`.
- Feature Flags dependency snippets: `version = "2.0"` -> `"3.0"`.
- Environment variable bullet: add `TYPESAFE_API_KEY`.

- [ ] **Step 7: `src/lib.rs` crate docs and `INSTALL.md`**

- In the `lib.rs` header, add TypeSafe to "Supported options include ...".
- In `src/cli.rs`, the top-level `about` string lists "chat, images, video, and embeddings"; add "evaluation" to that list (keep the rest of the string as is).
- Review `INSTALL.md`; change only if it lists providers or the eval command.

- [ ] **Step 8: Spec note**

In the spec, section 3, replace `--true "<meaning>"`, `--false "<meaning>"` with `--yes-means "<meaning>"`, `--no-means "<meaning>"`, and the YAML `true:` example key with `yes_means:`. Add one sentence: "Renamed from `--true`/`--false` during planning: an unquoted `true:` YAML key parses as a boolean." In section 2, replace the bullet "State with attachments is rejected: Jev accepts text only." with "Eval state is a JSON value (text or structured data), so binary attachments cannot reach TypeSafe; no extra check is needed."

- [ ] **Step 9: Verify and commit**

```bash
cargo fmt --all -- --check && cargo clippy -- -D warnings && cargo test && cargo build --no-default-features --lib
cargo run -q -- eval --help
cargo run -q -- ai skill | grep -n "eval"
```

```bash
git add -A
git diff --cached -U0 | grep '^+' | grep -P '\x{2014}'   # must print nothing
git commit -m "docs: eval capability and TypeSafe provider, 3.0.0 changelog and version

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01KrAcYXk575873KEAeYSvSA"
```

---

### Task 10: Live verification against TypeSafe and Foundry

**Files:** none changed unless a defect is found (then fix it in the owning task's file with a regression test).

- [ ] **Step 1: Add a TypeSafe node to a throwaway local config**

```bash
mkdir -p /tmp/ailloy-live && cd /tmp/ailloy-live
cat > .ailloy.yaml <<'YAML'
extends: global
nodes:
  typesafe/jev-latest:
    provider: typesafe
    model: jev-latest
    auth: { env: TYPESAFE_API_KEY }
    capabilities: [eval]
defaults:
  eval: typesafe/jev-latest
YAML
```

Check the exact local-config schema with `ailloy ai config --help` or `src/config.rs` (`extends` handling) and adjust if needed.

- [ ] **Step 2: Same batch on both backends**

```bash
cd /tmp/ailloy-live
cat > checks.yaml <<'YAML'
questions:
  mentions_order:
    yes_no: "Does the input mention an order id?"
  team:
    choice: "Which team should handle this?"
    options: { billing: "payments, refunds, payouts", technical: "bugs, outages", other: null }
    expect: [billing]
  frustration:
    score: "How frustrated is the customer?"
    levels: [Calm, Frustrated, Very angry]
YAML
echo "Customer #4417 says payouts failed for 3 days and they are furious. Order id ORD-99812." > ticket.txt
AILLOY=/Users/kristofer/repos/mklab-se/ailloy/target/release/ailloy
$AILLOY eval -f ticket.txt --questions checks.yaml; echo "exit $?"
$AILLOY eval -f ticket.txt --questions checks.yaml --node microsoft-foundry/gpt-5.6-luna; echo "exit $?"
$AILLOY eval -f ticket.txt --questions checks.yaml --json | head -40
$AILLOY ai test --all
```

Expected: TypeSafe run labelled `(jev-1.13.0)` with no rationale lines and exit 0; Foundry run labelled `(gpt-5.6-luna, self-reported)` with rationale lines and exit 0; JSON has `"calibration": "measured"`; `ai test --all` shows the TypeSafe node as `✓ ... (eval, N ms)`.

- [ ] **Step 3: Error paths**

```bash
TYPESAFE_API_KEY=invalid $AILLOY eval -f ticket.txt --yes-no "ok?"; echo "exit $? (expect 3, message names TYPESAFE_API_KEY and a request id)"
```

- [ ] **Step 4: Clean up and report**

```bash
rm -rf /tmp/ailloy-live /tmp/ailloy-eval-ticket.txt /tmp/checks.yaml
```

Report the outputs from Steps 2-3 in the task summary. No commit unless a fix was needed.
