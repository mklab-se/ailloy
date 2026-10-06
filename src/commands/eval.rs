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
            if q.options.is_some()
                || q.expect.is_some()
                || q.levels.is_some()
                || q.min.is_some()
                || q.max.is_some()
            {
                return Err(format!(
                    "question '{id}' is yes_no but sets choice/score fields (options, expect, levels, min, max)"
                ));
            }
            yes_no_item(&id, &text, q.yes_means, q.no_means, q.threshold, floor)
                .map_err(|e| format!("question '{id}': {e}"))?
        } else if let Some(text) = q.choice {
            if q.threshold.is_some()
                || q.yes_means.is_some()
                || q.no_means.is_some()
                || q.levels.is_some()
                || q.min.is_some()
                || q.max.is_some()
            {
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
            if q.threshold.is_some()
                || q.yes_means.is_some()
                || q.no_means.is_some()
                || q.options.is_some()
                || q.expect.is_some()
            {
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
        let Some(answer) = resp.answers.get(&item.id) else {
            continue;
        };
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
    let usage = resp
        .usage
        .as_ref()
        .map(|u| json!({"input_tokens": u.prompt_tokens, "output_tokens": u.completion_tokens}));
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
                    .map(|l| {
                        l.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| l.to_string())
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            };
            format!("{score:.2} of 0..{top} ({label})")
        }
        _ => String::new(),
    }
}

/// Human-readable output, one line per question.
pub fn render_text(
    resp: &EvalResponse,
    items: &[Item],
    outcomes: &[Outcome],
    batch: bool,
) -> String {
    let mut out = String::new();
    let model = model_label(resp);
    for (item, outcome) in items.iter().zip(outcomes) {
        let Some(answer) = resp.answers.get(&item.id) else {
            continue;
        };
        let status = match outcome {
            Outcome::Pass => "PASS".green().bold(),
            Outcome::Fail => "FAIL".red().bold(),
            Outcome::Unsure => "UNSURE".yellow().bold(),
        };
        let name = if batch {
            item.id.clone()
        } else {
            item.question.kind().replace('_', "-")
        };
        out.push_str(&format!(
            "{status}  {name}  {}  confidence {:.2}  {model}\n",
            describe(item, answer),
            answer.confidence()
        ));
        if let Answer::Choice { probabilities, .. } = answer {
            let dist: Vec<String> = probabilities
                .iter()
                .map(|(k, p)| format!("{k} {p:.2}"))
                .collect();
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
        println!(
            "{}",
            serde_json::to_string_pretty(&render_json(&response, items, &outcomes))?
        );
    } else {
        print!(
            "{}",
            render_text(&response, items, &outcomes, args.questions.is_some())
        );
    }
    Ok(exit_code(&outcomes))
}

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
        assert_eq!(
            parse_option("billing=payments, refunds"),
            ("billing".into(), Some("payments, refunds".into()))
        );
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
        assert!(
            matches!(by_id["mentions_order"].gate, Gate::YesNo { threshold } if threshold == 0.8)
        );
        assert!(
            matches!(&by_id["team"].gate, Gate::Choice { expect } if expect == &vec!["billing".to_string()])
        );
        assert_eq!(
            by_id["team"].min_confidence,
            Some(0.3),
            "global floor applies"
        );
        assert_eq!(
            by_id["frustration"].min_confidence,
            Some(0.6),
            "per-question floor wins"
        );
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
        assert!(
            parse_questions_file(none, false, None)
                .unwrap_err()
                .contains("'q'")
        );
        let stray = "questions:\n  q:\n    yes_no: \"ok?\"\n    options: {a: null, b: null}\n";
        assert!(
            parse_questions_file(stray, false, None)
                .unwrap_err()
                .contains("'q'")
        );
    }

    #[test]
    fn questions_file_rejects_unknown_expect() {
        let bad = "questions:\n  t:\n    choice: \"pick\"\n    options: {a: null, b: null}\n    expect: [c]\n";
        let err = parse_questions_file(bad, false, None).unwrap_err();
        assert!(err.contains("'t'") && err.contains("'c'"), "{err}");
    }

    #[test]
    fn questions_file_rejects_unknown_fields_and_empty() {
        assert!(
            parse_questions_file(
                "questions:\n  q:\n    yes_no: x\n    bogus: 1\n",
                false,
                None
            )
            .is_err()
        );
        assert!(
            parse_questions_file("questions: {}\n", false, None)
                .unwrap_err()
                .contains("no questions")
        );
    }

    fn item(gate: Gate, min_confidence: Option<f64>) -> Item {
        Item {
            id: "q".into(),
            question: Question::yes_no("x"),
            gate,
            min_confidence,
        }
    }

    #[test]
    fn gates() {
        let yes = Answer::YesNo { probability: 0.9 };
        assert_eq!(
            judge(&item(Gate::YesNo { threshold: 0.5 }, None), &yes),
            Outcome::Pass
        );
        assert_eq!(
            judge(&item(Gate::YesNo { threshold: 0.95 }, None), &yes),
            Outcome::Fail
        );
        // confidence of p=0.9 is 0.8
        assert_eq!(
            judge(&item(Gate::YesNo { threshold: 0.5 }, Some(0.85)), &yes),
            Outcome::Unsure
        );

        let mut probs = BTreeMap::new();
        probs.insert("billing".to_string(), 0.8);
        probs.insert("other".to_string(), 0.2);
        let choice = ailloy::eval::choice_answer(probs);
        assert_eq!(
            judge(&item(Gate::Choice { expect: vec![] }, None), &choice),
            Outcome::Pass
        );
        assert_eq!(
            judge(
                &item(
                    Gate::Choice {
                        expect: vec!["other".into()]
                    },
                    None
                ),
                &choice
            ),
            Outcome::Fail
        );

        let score = ailloy::eval::score_answer(vec![0.0, 0.5, 0.5]);
        assert_eq!(
            judge(
                &item(
                    Gate::Score {
                        min: None,
                        max: Some(1.0)
                    },
                    None
                ),
                &score
            ),
            Outcome::Fail
        );
        assert_eq!(
            judge(
                &item(
                    Gate::Score {
                        min: Some(1.0),
                        max: None
                    },
                    None
                ),
                &score
            ),
            Outcome::Pass
        );
    }

    #[test]
    fn a_failed_gate_wins_over_low_confidence() {
        let yes = Answer::YesNo { probability: 0.55 };
        assert_eq!(
            judge(&item(Gate::YesNo { threshold: 0.9 }, Some(0.9)), &yes),
            Outcome::Fail
        );
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
