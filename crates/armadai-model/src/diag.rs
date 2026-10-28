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

/// Byte offset of the first whole-word `token` at or after `from` (`key` = used as a mapping key).
fn find_idx(text: &str, from: usize, token: &str, key: bool) -> Option<usize> {
    if token.is_empty() {
        return None;
    }
    let tail = text.get(from..)?;
    for (i, _) in tail.match_indices(token) {
        let idx = from + i;
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
        return Some(idx);
    }
    None
}

/// 1-based (line, column) of byte `idx`.
pub(crate) fn line_col(text: &str, idx: usize) -> (Option<usize>, Option<usize>) {
    let before = &text[..idx];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (Some(line), Some(col))
}

fn find_word(text: &str, token: &str, key: bool) -> (Option<usize>, Option<usize>) {
    find_idx(text, 0, token, key).map_or((None, None), |i| line_col(text, i))
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

/// Byte offset of the first whole-word mapping `key` at or after byte `from`.
pub(crate) fn key_start(text: &str, from: usize, key: &str) -> Option<usize> {
    find_idx(text, from, key, true)
}

/// Byte offset just after the first `key:` used as a mapping key.
pub(crate) fn key_end(text: &str, key: &str) -> Option<usize> {
    let i = find_idx(text, 0, key, true)?;
    let after = i + key.len();
    let colon = text[after..].find(':')?;
    Some(after + colon + 1)
}

/// 1-based (line, column) of the first whole-word `token` at or after byte `from`.
pub(crate) fn locate_from(text: &str, from: usize, token: &str) -> (Option<usize>, Option<usize>) {
    find_idx(text, from, token, false).map_or((None, None), |i| line_col(text, i))
}

/// Byte offset of the key of the `n`-th (0-based) `key: value` pair at or after byte `from`,
/// `value` a whole word, bare or quoted, in block or flow style.
fn pair_idx(text: &str, from: usize, key: &str, value: &str, n: usize) -> Option<usize> {
    let mut at = from;
    let mut seen = 0;
    while let Some(k) = find_idx(text, at, key, true) {
        let after = k + key.len();
        at = after;
        let rest = text[after..]
            .trim_start_matches([' ', '\t', '"', '\''])
            .strip_prefix(':')
            .map(|r| r.trim_start_matches([' ', '\t', '"', '\'']));
        let Some(rest) = rest else { continue };
        if rest.starts_with(value) && !is_word(rest[value.len()..].chars().next()) {
            if seen == n {
                return Some(k);
            }
            seen += 1;
        }
    }
    None
}

/// 1-based (line, column) of the key of the `n`-th `key: value` pair at or after byte `from`.
pub(crate) fn locate_pair(
    text: &str,
    from: usize,
    key: &str,
    value: &str,
    n: usize,
) -> (Option<usize>, Option<usize>) {
    pair_idx(text, from, key, value, n).map_or((None, None), |i| line_col(text, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINKS: &str = "links:\n  - { from: a, to: b, when: rejected }\n  - from: b\n    to: c\n    when: \"rejected\"\n";

    #[test]
    fn locate_pair_counts_pairs_in_flow_and_block_style() {
        assert_eq!(
            locate_pair(LINKS, 0, "when", "rejected", 0),
            (Some(2), Some(23))
        );
        assert_eq!(
            locate_pair(LINKS, 0, "when", "rejected", 1),
            (Some(5), Some(5))
        );
        assert_eq!(locate_pair(LINKS, 0, "when", "rejected", 2), (None, None));
    }

    #[test]
    fn locate_pair_needs_the_whole_value() {
        assert_eq!(locate_pair(LINKS, 0, "when", "reject", 0), (None, None));
    }

    #[test]
    fn key_end_and_locate_from_skip_what_precedes() {
        let t = "a: x\nlinks:\n  - to: x\n";
        let end = key_end(t, "links").unwrap();
        assert_eq!(&t[end - 6..end], "links:");
        assert_eq!(locate_from(t, end, "x"), (Some(3), Some(9)));
        assert_eq!(key_end(t, "nope"), None);
    }
}
