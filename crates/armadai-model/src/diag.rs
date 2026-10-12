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
