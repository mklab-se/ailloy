//! Terminal helpers for enhanced output.

use std::io::IsTerminal;

/// Terminals known to support OSC 8 hyperlinks.
const SUPPORTED_TERMINALS: &[&str] = &["ghostty", "iTerm.app", "WezTerm", "vscode"];

/// Returns true if the terminal likely supports OSC 8 hyperlinks.
fn supports_hyperlinks() -> bool {
    if !std::io::stderr().is_terminal() {
        return false;
    }
    if let Ok(term) = std::env::var("TERM_PROGRAM") {
        return SUPPORTED_TERMINALS
            .iter()
            .any(|&t| t.eq_ignore_ascii_case(&term));
    }
    false
}

/// Format text as an OSC 8 clickable hyperlink if the terminal supports it.
/// Falls back to plain text otherwise.
pub fn hyperlink(url: &str, text: &str) -> String {
    if supports_hyperlinks() {
        format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, text)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hyperlink_format() {
        // When not in a supported terminal, should return plain text
        let result = hyperlink("file:///tmp/test.png", "test.png");
        // In CI/test environments stderr is typically not a terminal,
        // so this should be plain text
        assert!(result.contains("test.png"));
    }
}
