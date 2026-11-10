//! Cost estimation for AI calls.
//!
//! Providers bill per token (chat, images, embeddings) or per second (video),
//! but their APIs never say what a call cost. This module turns the token
//! counts a call reports ([`Usage`]) into an estimated USD amount.
//!
//! Prices come from two places, most specific first:
//!
//! 1. **Node overrides**: `defaults:` keys on a node in the config
//!    (`chat.price_input`, `chat.price_output`, `image.price_input`,
//!    `image.price_output`, `embedding.price_input`, `video.price_second`).
//!    Token prices are USD per 1M tokens; `video.price_second` is USD per
//!    second of generated video. Use these for models the built-in table
//!    doesn't know, negotiated rates, or a different deployment type.
//! 2. **Built-in list prices** ([`BUILTIN_PRICES`]): a static, prefix-matched
//!    table of public list prices. Update it when providers change prices.
//!
//! The result is an *estimate*: list prices only (no discounts, cached-input
//! rates, long-context surcharges or taxes), and image edits price their
//! input image tokens at the text-input rate because the usage block ailloy
//! keeps does not split them out. Treat it as an order-of-magnitude guide,
//! not an invoice.

use crate::config::{AiNode, Capability, ProviderKind};
use crate::types::Usage;

/// Where a [`Price`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PriceSource {
    /// The built-in list-price table.
    BuiltIn,
    /// The node's own `*.price_*` defaults (at least one field).
    Node,
    /// A local model (Ollama): nothing is billed.
    Local,
}

/// Prices for one model and capability.
///
/// Token prices are USD per 1M tokens; `per_second` is USD per second of
/// generated video. Any field may be unknown.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Price {
    /// USD per 1M input (prompt) tokens.
    pub input: Option<f64>,
    /// USD per 1M output (completion) tokens.
    pub output: Option<f64>,
    /// USD per second of generated video.
    pub per_second: Option<f64>,
    /// Where these prices came from.
    pub source: PriceSource,
}

impl Price {
    /// A token price (USD per 1M input and output tokens).
    pub fn tokens(input: f64, output: f64) -> Self {
        Self {
            input: Some(input),
            output: Some(output),
            per_second: None,
            source: PriceSource::Node,
        }
    }

    /// A per-second video price (USD per second).
    pub fn per_second(usd: f64) -> Self {
        Self {
            input: None,
            output: None,
            per_second: Some(usd),
            source: PriceSource::Node,
        }
    }

    /// Estimated USD cost of a call that reported `usage`.
    ///
    /// `None` when a price needed for the reported tokens is unknown (for
    /// example an output price is missing but the call produced output).
    pub fn cost_of(&self, usage: &Usage) -> Option<f64> {
        let part = |tokens: u32, price: Option<f64>| -> Option<f64> {
            if tokens == 0 {
                Some(0.0)
            } else {
                price.map(|p| f64::from(tokens) * p / 1_000_000.0)
            }
        };
        Some(part(usage.prompt_tokens, self.input)? + part(usage.completion_tokens, self.output)?)
    }

    /// Estimated USD cost of `seconds` of generated video.
    pub fn cost_of_seconds(&self, seconds: f64) -> Option<f64> {
        self.per_second.map(|p| p * seconds)
    }
}

/// One row of the built-in price table.
struct PriceRow {
    capability: Capability,
    prefix: &'static str,
    input: Option<f64>,
    output: Option<f64>,
    per_second: Option<f64>,
}

const fn tok(capability: Capability, prefix: &'static str, input: f64, output: f64) -> PriceRow {
    PriceRow {
        capability,
        prefix,
        input: Some(input),
        output: Some(output),
        per_second: None,
    }
}

const fn sec(prefix: &'static str, usd: f64) -> PriceRow {
    PriceRow {
        capability: Capability::Video,
        prefix,
        input: None,
        output: None,
        per_second: Some(usd),
    }
}

use Capability::{Chat, Embedding, Image};

/// Built-in list prices, matched by model-name prefix (longest wins,
/// case-insensitive).
///
/// Source: Azure Retail Prices API (`prices.azure.com`), Global Standard,
/// short context, USD, retrieved 2026-11-09. These match OpenAI's own list
/// prices for the same models. gpt-6.1-terra's output price was not yet on
/// the Azure price sheet and comes from OpenAI's published rate. Image input
/// is the text-input rate; image output is the image-output-token rate.
const BUILTIN_PRICES: &[PriceRow] = &[
    // Chat (USD per 1M tokens: input, output)
    tok(Chat, "gpt-6.1-sol", 2.0, 10.0),
    tok(Chat, "gpt-6.1-terra", 1.0, 5.0),
    tok(Chat, "gpt-6-astra", 10.0, 50.0),
    tok(Chat, "gpt-6-sol", 2.0, 10.0),
    tok(Chat, "gpt-6-luna", 0.1, 0.5),
    tok(Chat, "gpt-5.6-sol", 4.0, 20.0),
    tok(Chat, "gpt-5.6-terra", 2.0, 12.0),
    tok(Chat, "gpt-5.6-luna", 0.2, 1.2),
    // TypeSafe System One (USD per 1M input tokens; output tokens are free).
    // Source: docs.typesafe.ai/models, retrieved 2026-11-10. Eval is priced
    // through the chat rows.
    tok(Chat, "jev", 0.042, 0.0),
    // Images (USD per 1M tokens: text input, image output)
    tok(Image, "gpt-image-2.5-flare", 5.0, 30.0),
    tok(Image, "gpt-image-2.5-sunburst", 5.0, 30.0),
    tok(Image, "gpt-image-2", 5.0, 30.0),
    tok(Image, "gpt-image-1", 5.0, 40.0),
    tok(Image, "mai-image-2.6-flash", 5.0, 19.0),
    tok(Image, "mai-image-2.6", 5.0, 38.0),
    tok(Image, "mai-image-2.5-flash", 1.75, 19.5),
    tok(Image, "mai-image-2.5-pro", 5.0, 106.0),
    tok(Image, "mai-image-2.5", 5.0, 47.0),
    // Embeddings (USD per 1M input tokens; no output tokens)
    tok(Embedding, "text-embedding-3-large", 0.13, 0.0),
    tok(Embedding, "text-embedding-3-small", 0.02, 0.0),
    tok(Embedding, "text-embedding-ada-002", 0.10, 0.0),
    // Video (USD per second)
    sec("sora-2-pro", 0.30),
    sec("sora-2", 0.10),
];

/// The built-in list price for `model` used for `capability`, if known.
///
/// Matching is by prefix, so dated variants (`gpt-6.1-sol-2026-09-29`) match
/// their base model. The longest matching prefix wins, so `gpt-image-2.5-*`
/// is not priced as `gpt-image-2`. Eval calls are priced as chat.
pub fn builtin_price(capability: &Capability, model: &str) -> Option<Price> {
    let capability = match capability {
        Capability::Eval => &Capability::Chat,
        other => other,
    };
    let normalized = model.to_ascii_lowercase();
    BUILTIN_PRICES
        .iter()
        .filter(|row| &row.capability == capability && normalized.starts_with(row.prefix))
        .max_by_key(|row| row.prefix.len())
        .map(|row| Price {
            input: row.input,
            output: row.output,
            per_second: row.per_second,
            source: PriceSource::BuiltIn,
        })
}

/// The node-default keys that override prices for `capability`:
/// `(input, output, per_second)`.
fn override_keys(
    capability: &Capability,
) -> (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
) {
    match capability {
        Capability::Chat | Capability::Eval => {
            (Some("chat.price_input"), Some("chat.price_output"), None)
        }
        Capability::Image => (Some("image.price_input"), Some("image.price_output"), None),
        Capability::Embedding => (Some("embedding.price_input"), None, None),
        Capability::Video => (None, None, Some("video.price_second")),
    }
}

/// The price for calls to `node` using `capability`, if any is known.
///
/// Node overrides (`chat.price_input`, ...) take precedence field by field
/// over the built-in list price for the node's model. Ollama nodes are free
/// (local). Local CLI agents bill through their own subscription, so they
/// have no price unless the node sets one.
pub fn price_for_node(node: &AiNode, capability: &Capability) -> Option<Price> {
    let model = node.model.as_deref().or(node.deployment.as_deref());
    let builtin = model.and_then(|m| builtin_price(capability, m));
    let mut price = match (&node.provider, builtin) {
        (ProviderKind::Ollama, _) => Price {
            input: Some(0.0),
            output: Some(0.0),
            per_second: Some(0.0),
            source: PriceSource::Local,
        },
        (_, Some(p)) => p,
        (_, None) => Price {
            input: None,
            output: None,
            per_second: None,
            source: PriceSource::Node,
        },
    };

    let read = |key: Option<&str>| -> Option<f64> {
        key.and_then(|k| node.default_for(k))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    let (in_key, out_key, sec_key) = override_keys(capability);
    let overrides = [
        (read(in_key), &mut price.input),
        (read(out_key), &mut price.output),
        (read(sec_key), &mut price.per_second),
    ];
    let mut overridden = false;
    for (value, slot) in overrides {
        if let Some(v) = value {
            *slot = Some(v);
            overridden = true;
        }
    }
    if overridden {
        price.source = PriceSource::Node;
    }
    // Embeddings produce no output tokens; an input price alone is complete.
    if *capability == Capability::Embedding && price.input.is_some() && price.output.is_none() {
        price.output = Some(0.0);
    }

    (price.input.is_some() || price.output.is_some() || price.per_second.is_some()).then_some(price)
}

/// Format a USD amount for display: `$0`, `<$0.0001`, `$0.0059`, `$0.42`,
/// `$12.30`.
pub fn format_usd(amount: f64) -> String {
    if amount <= 0.0 {
        "$0".to_string()
    } else if amount < 0.0001 {
        "<$0.0001".to_string()
    } else if amount < 0.01 {
        format!("${amount:.4}")
    } else if amount < 1.0 {
        format!("${amount:.3}")
    } else {
        format!("${amount:.2}")
    }
}

/// A running total of estimated cost across several calls (for example an
/// interactive chat session).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostTracker {
    total: f64,
    calls: u32,
    unpriced_calls: u32,
}

impl CostTracker {
    /// An empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one call's estimated cost (`None` when it could not be
    /// priced).
    pub fn record(&mut self, cost: Option<f64>) {
        self.calls += 1;
        match cost {
            Some(c) => self.total += c,
            None => self.unpriced_calls += 1,
        }
    }

    /// The summed estimate of every priced call.
    pub fn total(&self) -> f64 {
        self.total
    }

    /// Number of calls recorded.
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// Number of recorded calls that could not be priced.
    pub fn unpriced_calls(&self) -> u32 {
        self.unpriced_calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Capability::Video;
    use std::collections::BTreeMap;

    fn usage(prompt: u32, completion: u32) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
        }
    }

    fn node(provider: ProviderKind, model: &str, defaults: &[(&str, &str)]) -> AiNode {
        let mut n = AiNode::new(provider);
        n.model = Some(model.to_string());
        if !defaults.is_empty() {
            n.node_defaults = Some(
                defaults
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<BTreeMap<_, _>>(),
            );
        }
        n
    }

    #[test]
    fn builtin_prices_the_new_foundry_models() {
        let terra = builtin_price(&Chat, "gpt-6.1-terra").unwrap();
        assert_eq!((terra.input, terra.output), (Some(1.0), Some(5.0)));
        let sol = builtin_price(&Chat, "gpt-6.1-sol").unwrap();
        assert_eq!((sol.input, sol.output), (Some(2.0), Some(10.0)));
        for m in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
            let p = builtin_price(&Image, m).unwrap();
            assert_eq!((p.input, p.output), (Some(5.0), Some(30.0)), "{m}");
        }
    }

    #[test]
    fn builtin_matching_is_prefix_longest_and_case_insensitive() {
        let dated = builtin_price(&Chat, "GPT-6.1-Sol-2026-09-29").unwrap();
        assert_eq!(dated.output, Some(10.0));
        // MAI-Image-2.6-Flash must not be priced as MAI-Image-2.6
        let flash = builtin_price(&Image, "MAI-Image-2.6-Flash").unwrap();
        assert_eq!(flash.output, Some(19.0));
        let full = builtin_price(&Image, "MAI-Image-2.6").unwrap();
        assert_eq!(full.output, Some(38.0));
        let pro = builtin_price(&Video, "sora-2-pro").unwrap();
        assert_eq!(pro.per_second, Some(0.30));
        assert_eq!(
            builtin_price(&Video, "sora-2").unwrap().per_second,
            Some(0.10)
        );
    }

    #[test]
    fn jev_is_priced_for_eval_with_free_output() {
        let mut n = AiNode::new(ProviderKind::TypeSafe);
        n.model = Some("jev-latest".to_string());
        let p = price_for_node(&n, &Capability::Eval).unwrap();
        assert_eq!((p.input, p.output), (Some(0.042), Some(0.0)));
        let c = p.cost_of(&usage(1_000_000, 50)).unwrap();
        assert!((c - 0.042).abs() < 1e-12, "{c}");
        assert!(builtin_price(&Chat, "jev-1.14.0").is_some());
    }

    #[test]
    fn builtin_is_scoped_to_capability() {
        assert!(builtin_price(&Image, "gpt-6.1-sol").is_none());
        assert!(builtin_price(&Chat, "gpt-image-2").is_none());
        assert!(builtin_price(&Chat, "some-unknown-model").is_none());
        // eval runs on chat models
        assert!(builtin_price(&Capability::Eval, "gpt-6.1-sol").is_some());
    }

    #[test]
    fn cost_of_tokens() {
        let p = builtin_price(&Image, "gpt-image-2.5-flare").unwrap();
        // 13 prompt * $5/M + 196 output * $30/M
        let c = p.cost_of(&usage(13, 196)).unwrap();
        assert!((c - 0.005945).abs() < 1e-9, "{c}");
    }

    #[test]
    fn cost_of_needs_every_used_price() {
        let p = Price {
            input: Some(1.0),
            output: None,
            per_second: None,
            source: PriceSource::Node,
        };
        assert!(p.cost_of(&usage(100, 10)).is_none());
        assert_eq!(p.cost_of(&usage(1_000_000, 0)), Some(1.0));
    }

    #[test]
    fn video_cost_per_second() {
        let p = builtin_price(&Video, "sora-2").unwrap();
        assert!((p.cost_of_seconds(8.0).unwrap() - 0.8).abs() < 1e-9);
        assert!(Price::tokens(1.0, 1.0).cost_of_seconds(4.0).is_none());
    }

    #[test]
    fn node_override_beats_builtin_field_by_field() {
        let n = node(
            ProviderKind::MicrosoftFoundry,
            "gpt-6.1-sol",
            &[("chat.price_output", "8")],
        );
        let p = price_for_node(&n, &Chat).unwrap();
        assert_eq!(p.input, Some(2.0));
        assert_eq!(p.output, Some(8.0));
        assert_eq!(p.source, PriceSource::Node);
    }

    #[test]
    fn node_override_prices_unknown_models() {
        let unknown = node(ProviderKind::Anthropic, "claude-next", &[]);
        assert!(price_for_node(&unknown, &Chat).is_none());
        let priced = node(
            ProviderKind::Anthropic,
            "claude-next",
            &[("chat.price_input", "3"), ("chat.price_output", "15")],
        );
        let p = price_for_node(&priced, &Chat).unwrap();
        assert_eq!(p.cost_of(&usage(1_000_000, 1_000_000)), Some(18.0));
    }

    #[test]
    fn invalid_override_values_are_ignored() {
        let n = node(
            ProviderKind::MicrosoftFoundry,
            "gpt-6.1-sol",
            &[("chat.price_input", "-1"), ("chat.price_output", "abc")],
        );
        let p = price_for_node(&n, &Chat).unwrap();
        assert_eq!((p.input, p.output), (Some(2.0), Some(10.0)));
        assert_eq!(p.source, PriceSource::BuiltIn);
    }

    #[test]
    fn ollama_is_free_and_deployment_name_is_a_fallback() {
        let local = node(ProviderKind::Ollama, "llama3", &[]);
        let p = price_for_node(&local, &Chat).unwrap();
        assert_eq!(p.source, PriceSource::Local);
        assert_eq!(p.cost_of(&usage(500, 500)), Some(0.0));

        let mut azure = AiNode::new(ProviderKind::AzureOpenAi);
        azure.deployment = Some("gpt-6.1-terra".to_string());
        assert_eq!(price_for_node(&azure, &Chat).unwrap().input, Some(1.0));
    }

    #[test]
    fn embedding_input_override_is_complete() {
        let n = node(
            ProviderKind::OpenAi,
            "my-embedder",
            &[("embedding.price_input", "0.5")],
        );
        let p = price_for_node(&n, &Embedding).unwrap();
        assert_eq!(p.cost_of(&usage(2_000_000, 0)), Some(1.0));
    }

    #[test]
    fn video_override() {
        let n = node(
            ProviderKind::MicrosoftFoundry,
            "sora-2",
            &[("video.price_second", "0.2")],
        );
        let p = price_for_node(&n, &Video).unwrap();
        assert_eq!(p.per_second, Some(0.2));
    }

    #[test]
    fn format_usd_ranges() {
        assert_eq!(format_usd(0.0), "$0");
        assert_eq!(format_usd(0.00001), "<$0.0001");
        assert_eq!(format_usd(0.005945), "$0.0059");
        assert_eq!(format_usd(0.4219), "$0.422");
        assert_eq!(format_usd(12.3), "$12.30");
    }

    #[test]
    fn tracker_sums_and_counts_unpriced() {
        let mut t = CostTracker::new();
        t.record(Some(0.01));
        t.record(None);
        t.record(Some(0.02));
        assert!((t.total() - 0.03).abs() < 1e-12);
        assert_eq!(t.calls(), 3);
        assert_eq!(t.unpriced_calls(), 1);
    }
}
