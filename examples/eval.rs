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
        Question::score("How frustrated is the customer?").levels([
            "Calm",
            "Annoyed",
            "Frustrated",
            "Furious",
        ]),
    );

    let client = Client::for_capability("eval")?;
    let response = client.eval(ticket, &questions).await?;

    println!("model: {} ({:?})", response.model, response.calibration);
    for (id, answer) in &response.answers {
        let confident = answer.confidence() >= 0.6;
        println!(
            "{id:<12} {answer:?}  confidence {:.2}{}",
            answer.confidence(),
            if confident {
                ""
            } else {
                "  -> route to a human"
            }
        );
    }
    Ok(())
}
