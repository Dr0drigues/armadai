//! Conformance report: for every declared element and target, what became of it.

use serde::Serialize;

/// Why `flow recording` is not compiled for Codex, Copilot and OpenCode.
pub const RECORDING_CLAUDE_ONLY: &str = "Claude Code only for now";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Compiled,
    Approximated,
    NotCompiled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Compiled => "compiled",
            Status::Approximated => "approximated",
            Status::NotCompiled => "not-compiled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportLine {
    pub target: String,
    pub element: String,
    pub status: Status,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub lines: Vec<ReportLine>,
}

impl Report {
    pub fn push(
        &mut self,
        target: &str,
        element: impl Into<String>,
        status: Status,
        reason: Option<&str>,
    ) {
        self.lines.push(ReportLine {
            target: target.to_string(),
            element: element.into(),
            status,
            reason: reason.map(str::to_string),
        });
    }

    fn sorted(&self) -> Vec<&ReportLine> {
        let mut v: Vec<&ReportLine> = self.lines.iter().collect();
        // stable: lines on the same element keep their insertion order
        v.sort_by(|a, b| (&a.target, &a.element).cmp(&(&b.target, &b.element)));
        v
    }

    /// `target: element: status (reason)`, one line each, sorted by target then element.
    pub fn render_text(&self) -> String {
        self.sorted()
            .into_iter()
            .map(|l| {
                let mut s = format!("{}: {}: {}", l.target, l.element, l.status.as_str());
                if let Some(r) = &l.reason {
                    s.push_str(&format!(" ({r})"));
                }
                s.push('\n');
                s
            })
            .collect()
    }

    /// `[{target, element, status, reason}]`, in the order of [`Self::render_text`].
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self.sorted()).expect("report lines serialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        let mut r = Report::default();
        r.push("codex", "agent dev", Status::NotCompiled, Some("step 4"));
        r.push(
            "claude",
            "agent dev capability shell=ask",
            Status::Approximated,
            Some("per-agent ask unavailable"),
        );
        r.push("claude", "agent dev", Status::Compiled, None);
        r
    }

    #[test]
    fn text_is_sorted_by_target_then_element_and_keeps_the_reason() {
        assert_eq!(
            sample().render_text(),
            "claude: agent dev: compiled\n\
             claude: agent dev capability shell=ask: approximated (per-agent ask unavailable)\n\
             codex: agent dev: not-compiled (step 4)\n"
        );
    }

    #[test]
    fn json_has_stable_fields() {
        assert_eq!(
            sample().to_json(),
            serde_json::json!([
                {"target": "claude", "element": "agent dev", "status": "compiled", "reason": null},
                {"target": "claude", "element": "agent dev capability shell=ask",
                 "status": "approximated", "reason": "per-agent ask unavailable"},
                {"target": "codex", "element": "agent dev", "status": "not-compiled", "reason": "step 4"},
            ])
        );
    }
}
