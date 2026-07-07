//! `ailloy eval` — LLM-as-judge evaluation with script-friendly exit codes.
//!
//! The integration-test workhorse: pipe a (possibly non-deterministic) output
//! in, state the criteria, get a verdict back — exit 0 on pass, 1 on fail.
//!
//! ```bash
//! my-tool ask "summarize the report" | ailloy eval \
//!   --criteria "mentions the Q3 revenue figure and names at least two risks"
//! ```

use std::io::{IsTerminal, Read};

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use ailloy::Client;
use ailloy::types::{ChatOptions, Message};

/// Exit codes: 0 pass · 1 fail · 2 usage/config error · 3 provider error.
pub const EXIT_PASS: u8 = 0;
pub const EXIT_FAIL: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_PROVIDER: u8 = 3;

pub struct EvalArgs {
    /// What must hold for the input to pass.
    pub criteria: Option<String>,
    /// Read criteria from a file instead.
    pub criteria_file: Option<String>,
    /// Input to evaluate (positional; stdin and --file also work).
    pub input: Option<String>,
    /// Read the input to evaluate from a file.
    pub file: Option<String>,
    /// Extra context for the judge (what produced the input, expectations…).
    pub context: Option<String>,
    /// Judge node (defaults to the default chat node).
    pub node: Option<String>,
    /// Pass when score >= threshold instead of using the judge's own verdict.
    pub threshold: Option<f32>,
    /// Output format: text (default) or json.
    pub json: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Verdict {
    pub pass: bool,
    /// 0.0 (clear fail) .. 1.0 (clear pass).
    pub score: f32,
    pub reasons: Vec<String>,
}

fn verdict_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "pass": {"type": "boolean", "description": "Does the input satisfy the criteria?"},
            "score": {"type": "number", "minimum": 0.0, "maximum": 1.0,
                      "description": "How well the input satisfies the criteria: 0.0 = clearly fails, 1.0 = clearly satisfies. Must be consistent with pass (pass=false implies score < 0.5)."},
            "reasons": {"type": "array", "items": {"type": "string"},
                        "description": "Short, concrete reasons for the verdict"}
        },
        "required": ["pass", "score", "reasons"],
        "additionalProperties": false
    })
}

/// Build the judge messages. Separated for testability.
pub fn judge_messages(criteria: &str, input: &str, context: Option<&str>) -> Vec<Message> {
    let system = "You are a strict, fair evaluator used inside automated tests. \
                  Judge ONLY whether the input satisfies the stated criteria — not style, \
                  not what you would have written. Be deterministic: identical inputs must \
                  get identical verdicts. Respond with JSON only."
        .to_string();
    let mut user = format!("## Criteria\n{criteria}\n\n## Input to evaluate\n{input}");
    if let Some(ctx) = context {
        user = format!("## Context\n{ctx}\n\n{user}");
    }
    vec![Message::system(system), Message::user(user)]
}

/// Parse the judge's response into a Verdict (tolerates code fences).
pub fn parse_verdict(raw: &str) -> Result<Verdict> {
    let text = raw.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .unwrap_or(text)
        .trim_end_matches("```")
        .trim();
    let mut verdict: Verdict = serde_json::from_str(text)
        .with_context(|| format!("judge did not return valid verdict JSON: {raw}"))?;
    verdict.score = verdict.score.clamp(0.0, 1.0);
    Ok(verdict)
}

/// Apply the optional threshold override.
pub fn final_pass(verdict: &Verdict, threshold: Option<f32>) -> bool {
    match threshold {
        Some(t) => verdict.score >= t,
        None => verdict.pass,
    }
}

pub async fn run(args: EvalArgs) -> u8 {
    match run_inner(args).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e:#}", "error:".red().bold());
            EXIT_PROVIDER
        }
    }
}

async fn run_inner(args: EvalArgs) -> Result<u8> {
    // Criteria
    let criteria = match (&args.criteria, &args.criteria_file) {
        (Some(c), _) => c.clone(),
        (None, Some(f)) => {
            std::fs::read_to_string(f).with_context(|| format!("cannot read criteria file {f}"))?
        }
        (None, None) => {
            eprintln!(
                "{} --criteria (or --criteria-file) is required.\n\n  example: my-tool | ailloy eval --criteria \"the answer names the capital of Sweden\"",
                "error:".red().bold()
            );
            return Ok(EXIT_USAGE);
        }
    };

    // Input: positional > --file > stdin
    let input = if let Some(input) = args.input {
        input
    } else if let Some(f) = &args.file {
        std::fs::read_to_string(f).with_context(|| format!("cannot read input file {f}"))?
    } else if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read stdin")?;
        buf
    } else {
        eprintln!(
            "{} nothing to evaluate — pass an argument, --file, or pipe stdin",
            "error:".red().bold()
        );
        return Ok(EXIT_USAGE);
    };
    if input.trim().is_empty() {
        eprintln!("{} input is empty", "error:".red().bold());
        return Ok(EXIT_USAGE);
    }

    // Judge
    let client = match &args.node {
        Some(node) => Client::with_node(node)?,
        None => Client::from_config()?,
    };
    let options = ChatOptions::builder()
        .json_schema("verdict", verdict_schema())
        .build();
    let messages = judge_messages(&criteria, &input, args.context.as_deref());
    let response = client.chat_with(&messages, &options).await?;
    let verdict = parse_verdict(&response.content)?;
    let pass = final_pass(&verdict, args.threshold);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pass": pass,
                "score": verdict.score,
                "reasons": verdict.reasons,
                "judge_model": response.model,
            }))?
        );
    } else {
        let label = if pass {
            "PASS".green().bold()
        } else {
            "FAIL".red().bold()
        };
        println!(
            "{label}  (score {:.2}, judge: {})",
            verdict.score, response.model
        );
        for reason in &verdict.reasons {
            println!("  - {reason}");
        }
    }

    Ok(if pass { EXIT_PASS } else { EXIT_FAIL })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_fenced_verdicts() {
        let v = parse_verdict(r#"{"pass": true, "score": 0.9, "reasons": ["ok"]}"#).unwrap();
        assert!(v.pass);
        let v = parse_verdict("```json\n{\"pass\": false, \"score\": 0.2, \"reasons\": []}\n```")
            .unwrap();
        assert!(!v.pass);
        assert!(parse_verdict("the input looks fine to me").is_err());
    }

    #[test]
    fn score_is_clamped() {
        let v = parse_verdict(r#"{"pass": true, "score": 3.5, "reasons": []}"#).unwrap();
        assert_eq!(v.score, 1.0);
    }

    #[test]
    fn threshold_overrides_judge_verdict() {
        let v = Verdict {
            pass: false,
            score: 0.8,
            reasons: vec![],
        };
        assert!(!final_pass(&v, None));
        assert!(final_pass(&v, Some(0.7)));
        assert!(!final_pass(&v, Some(0.9)));
    }

    #[test]
    fn judge_prompt_contains_all_parts() {
        let msgs = judge_messages("must be swedish", "hej världen", Some("greeting test"));
        assert_eq!(msgs.len(), 2);
        let user = &msgs[1].content;
        assert!(user.contains("must be swedish"));
        assert!(user.contains("hej världen"));
        assert!(user.contains("greeting test"));
    }

    #[test]
    fn verdict_schema_is_closed() {
        let s = verdict_schema();
        assert_eq!(s["additionalProperties"], false);
        assert_eq!(s["required"][0], "pass");
    }
}
