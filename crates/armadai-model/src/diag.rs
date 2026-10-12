//! Positioned diagnostics.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub file: PathBuf,
    /// YAML path of the offending node (empty when unknown).
    pub path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    /// `file:line:column: error[code]: message` (+ `\n  hint: ...`).
    pub fn render(&self) -> String {
        let pos = match (self.line, self.column) {
            (Some(l), Some(c)) => format!(":{l}:{c}"),
            (Some(l), None) => format!(":{l}"),
            _ => String::new(),
        };
        let mut out = format!(
            "{}{}: {}[{}]: {}",
            self.file.display(),
            pos,
            self.severity.as_str(),
            self.code,
            self.message
        );
        if let Some(h) = &self.hint {
            out.push_str(&format!("\n  hint: {h}"));
        }
        out
    }
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn find_word(text: &str, token: &str, key: bool) -> (Option<usize>, Option<usize>) {
    if token.is_empty() {
        return (None, None);
    }
    for (idx, _) in text.match_indices(token) {
        let rest = &text[idx + token.len()..];
        if is_word(text[..idx].chars().next_back()) || is_word(rest.chars().next()) {
            continue;
        }
        if key
            && !rest
                .trim_start_matches([' ', '\t', '"', '\''])
                .starts_with(':')
        {
            continue;
        }
        let before = &text[..idx];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        return (Some(line), Some(col));
    }
    (None, None)
}

/// 1-based (line, column) of the first whole-word occurrence of `token`.
// shortcut: first whole-word occurrence, not the exact YAML node; upgrade if a position-aware YAML parser is ever adopted.
pub fn locate(text: &str, token: &str) -> (Option<usize>, Option<usize>) {
    find_word(text, token, false)
}

/// Like [`locate`] but only matches `token` used as a mapping key (followed by `:`).
pub fn locate_key(text: &str, key: &str) -> (Option<usize>, Option<usize>) {
    find_word(text, key, true)
}
