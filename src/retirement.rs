//! Model retirement awareness.
//!
//! Providers retire models on published schedules; a configured node pointing
//! at a retired model fails at request time with a confusing provider error.
//! This static table lets `ailloy ai status` warn ahead of time. Update it
//! when providers announce new retirement dates (sources: Azure OpenAI model
//! retirement schedule, Anthropic model deprecations).

/// (model prefix, retirement date `YYYY-MM-DD`, suggested replacement)
const RETIREMENTS: &[(&str, &str, &str)] = &[
    // OpenAI / Azure OpenAI (Azure retirement schedule, 2026)
    ("gpt-4o-mini", "2026-10-01", "gpt-5.4-mini"),
    ("gpt-4o", "2026-10-01", "gpt-5.4"),
    ("gpt-4.1-nano", "2026-10-14", "gpt-5.4-mini"),
    ("gpt-4.1-mini", "2026-10-14", "gpt-5.4-mini"),
    ("gpt-4.1", "2026-10-14", "gpt-5.4"),
    ("o1", "2026-07-15", "gpt-5.5"),
    ("o3-mini", "2026-08-02", "gpt-5.4-mini"),
    ("o4-mini", "2026-10-16", "gpt-5.4-mini"),
    // Anthropic (first-party and on Foundry)
    ("claude-opus-4-1", "2026-08-05", "claude-opus-4-8"),
    ("claude-3-7-sonnet", "2026-02-19", "claude-sonnet-5"),
    ("claude-3-5-haiku", "2026-02-19", "claude-haiku-4-5"),
    // Google
    ("gemini-3-pro", "2026-06-30", "gemini-3.1-pro"),
];

/// A warning when `model` matches a scheduled (or past) retirement.
///
/// Matching is by prefix so dated variants (`gpt-4o-2024-08-06`) and
/// deployment-style names are caught. Longest prefix wins.
pub fn retirement_warning(model: &str) -> Option<String> {
    let normalized = model.to_ascii_lowercase();
    RETIREMENTS
        .iter()
        .filter(|(prefix, _, _)| normalized.starts_with(prefix))
        .max_by_key(|(prefix, _, _)| prefix.len())
        .map(|(_, date, replacement)| {
            format!("model '{model}' retires {date} — consider switching to '{replacement}'")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_for_retiring_models_incl_dated_variants() {
        assert!(retirement_warning("gpt-4o").unwrap().contains("2026-10-01"));
        assert!(retirement_warning("gpt-4o-2024-08-06").is_some());
        assert!(
            retirement_warning("gpt-4o-mini")
                .unwrap()
                .contains("gpt-5.4-mini"),
            "longest prefix wins"
        );
        assert!(retirement_warning("claude-opus-4-1").is_some());
        assert!(retirement_warning("o3-mini").is_some());
    }

    #[test]
    fn silent_for_current_models() {
        assert!(retirement_warning("gpt-5.4-mini").is_none());
        assert!(retirement_warning("claude-sonnet-5").is_none());
        assert!(retirement_warning("gemini-3.1-pro").is_none());
        assert!(retirement_warning("o1-preview-but-not-really-no-wait-yes").is_some());
        assert!(retirement_warning("phi-4").is_none());
    }
}
