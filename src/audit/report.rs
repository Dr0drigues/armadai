use std::fmt::Write as _;
use std::path::PathBuf;

use super::rules::{Finding, Severity};

/// Assembled result of one audit run.
pub struct AuditReport {
    pub root: PathBuf,
    pub detected: Vec<String>,
    pub agent_count: usize,
    pub skill_count: usize,
    pub findings: Vec<Finding>,
}

impl AuditReport {
    pub fn critical_count(&self) -> usize {
        self.count(Severity::Critical)
    }

    fn count(&self, s: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == s).count()
    }

    fn summary_line(&self) -> String {
        format!(
            "{} critical, {} warning(s), {} info",
            self.count(Severity::Critical),
            self.count(Severity::Warning),
            self.count(Severity::Info)
        )
    }

    /// Funnel block: what adopting ArmadAI would fix automatically.
    fn funnel_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let remappable = self.findings.iter().filter(|f| f.rule == "A03").count();
        if remappable > 0 {
            lines.push(format!(
                "{remappable} deprecated model(s) remapped automatically (model aliases)"
            ));
        }
        let dedupable = self.findings.iter().filter(|f| f.rule == "A06").count();
        if dedupable > 0 {
            lines.push(format!(
                "{dedupable} duplicated block(s) turned into shared prompt fragments"
            ));
        }
        let oversized = self.findings.iter().filter(|f| f.rule == "A05").count();
        if oversized > 0 {
            lines.push(format!(
                "{oversized} oversized prompt(s) split with composable fragments"
            ));
        }
        lines
    }

    /// Plain aligned output on stdout, findings ordered by severity
    /// (findings are already sorted by run_rules). The CLI's final
    /// `anyhow::bail!` on critical findings is what signals errors on
    /// stderr, so this only ever writes to stdout.
    pub fn print_terminal(&self) {
        println!("armadai audit - {}", self.root.display());
        println!(
            "  Detected: {} ({} agent(s), {} skill(s))",
            self.detected.join(", "),
            self.agent_count,
            self.skill_count
        );
        println!();
        for f in &self.findings {
            let line = format!(
                "  {:<5} {:<4} {:<40} {}",
                f.severity.label(),
                f.rule,
                f.file.display(),
                f.message
            );
            println!("{line}");
            if let Some(s) = &f.suggestion {
                println!("        -> {s}");
            }
        }
        println!();
        println!("  Summary: {}", self.summary_line());
        let funnel = self.funnel_lines();
        if !funnel.is_empty() {
            println!();
            println!("  What ArmadAI would give you:");
            for l in funnel {
                println!("    - {l}");
            }
            println!("    Run `armadai audit --propose` (coming soon) to generate the config.");
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        let _ = writeln!(md, "# armadai audit — {}\n", self.root.display());
        let _ = writeln!(
            md,
            "Detected: {} ({} agents, {} skills)\n",
            self.detected.join(", "),
            self.agent_count,
            self.skill_count
        );
        let _ = writeln!(md, "**Summary: {}**\n", self.summary_line());
        let _ = writeln!(md, "| Severity | Rule | File | Message | Suggestion |");
        let _ = writeln!(md, "|---|---|---|---|---|");
        for f in &self.findings {
            let _ = writeln!(
                md,
                "| {} | {} | `{}` | {} | {} |",
                f.severity.label(),
                f.rule,
                f.file.display(),
                f.message,
                f.suggestion.as_deref().unwrap_or("—")
            );
        }
        let funnel = self.funnel_lines();
        if !funnel.is_empty() {
            let _ = writeln!(md, "\n## What ArmadAI would give you\n");
            for l in funnel {
                let _ = writeln!(md, "- {l}");
            }
        }
        md
    }

    /// Fully self-contained HTML report: no external resources, no JavaScript,
    /// inline CSS with light/dark support via `prefers-color-scheme`.
    pub fn to_html(&self) -> String {
        let root = html_escape(&self.root.display().to_string());
        let detected = html_escape(&self.detected.join(", "));
        let mut html = String::new();
        let _ = write!(
            html,
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>armadai audit — {root}</title>
<style>
{css}
</style>
</head>
<body>
<header>
<h1>armadai audit — <code>{root}</code></h1>
<p class="detected">Detected: {detected} ({agent_count} agent(s), {skill_count} skill(s))</p>
</header>
<p class="summary"><strong>Summary: {summary}</strong></p>
<table>
<thead>
<tr><th>Severity</th><th>Rule</th><th>File</th><th>Message</th><th>Suggestion</th></tr>
</thead>
<tbody>
"#,
            css = HTML_CSS,
            agent_count = self.agent_count,
            skill_count = self.skill_count,
            summary = html_escape(&self.summary_line()),
        );

        for f in &self.findings {
            let badge_class = match f.severity {
                Severity::Critical => "badge-crit",
                Severity::Warning => "badge-warn",
                Severity::Info => "badge-info",
            };
            let _ = writeln!(
                html,
                r#"<tr>
<td><span class="badge {badge_class}">{severity}</span></td>
<td>{rule}</td>
<td><code class="path">{file}</code></td>
<td>{message}</td>
<td>{suggestion}</td>
</tr>"#,
                badge_class = badge_class,
                severity = html_escape(f.severity.label()),
                rule = html_escape(f.rule),
                file = html_escape(&f.file.display().to_string()),
                message = html_escape(&f.message),
                suggestion = f
                    .suggestion
                    .as_deref()
                    .map(html_escape)
                    .unwrap_or_else(|| "—".to_string()),
            );
        }

        html.push_str("</tbody>\n</table>\n");

        let funnel = self.funnel_lines();
        if !funnel.is_empty() {
            html.push_str(
                "<section class=\"funnel\">\n<h2>What ArmadAI would give you</h2>\n<ul>\n",
            );
            for l in funnel {
                let _ = writeln!(html, "<li>{}</li>", html_escape(&l));
            }
            html.push_str("</ul>\n</section>\n");
        }

        html.push_str("<footer><p>generated by armadai audit</p></footer>\n</body>\n</html>\n");
        html
    }
}

const HTML_CSS: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #ffffff;
  --fg: #1a1a1a;
  --muted: #5a5a5a;
  --border: #d8d8d8;
  --row-alt: #f5f5f5;
  --crit-bg: #fde2e1;
  --crit-fg: #8b1a1a;
  --warn-bg: #fdf0d5;
  --warn-fg: #8a5a00;
  --info-bg: #dbeafe;
  --info-fg: #1e40af;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #1e1e1e;
    --fg: #e6e6e6;
    --muted: #a0a0a0;
    --border: #3a3a3a;
    --row-alt: #262626;
    --crit-bg: #4a1414;
    --crit-fg: #ff9a9a;
    --warn-bg: #4a3a10;
    --warn-fg: #ffcf7a;
    --info-bg: #12305a;
    --info-fg: #9ec5ff;
  }
}
* { box-sizing: border-box; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  background: var(--bg);
  color: var(--fg);
  margin: 0;
  padding: 1.5rem;
  line-height: 1.5;
}
header h1 { font-size: 1.3rem; margin-bottom: 0.25rem; }
header .detected, .summary { color: var(--muted); }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
table {
  width: 100%;
  border-collapse: collapse;
  margin-top: 1rem;
}
th, td {
  text-align: left;
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border);
  vertical-align: top;
}
tbody tr:nth-child(even) { background: var(--row-alt); }
code.path {
  word-break: break-all;
  overflow-wrap: anywhere;
}
.badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 0.75rem;
  font-size: 0.75rem;
  font-weight: 600;
  white-space: nowrap;
}
.badge-crit { background: var(--crit-bg); color: var(--crit-fg); }
.badge-warn { background: var(--warn-bg); color: var(--warn-fg); }
.badge-info { background: var(--info-bg); color: var(--info-fg); }
.funnel { margin-top: 1.5rem; }
.funnel h2 { font-size: 1.05rem; }
footer {
  margin-top: 2rem;
  color: var(--muted);
  font-size: 0.85rem;
}
"#;

/// Escape the five HTML special characters so untrusted findings text
/// (file paths, messages, suggestions) can never break out of markup.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::{Finding, Severity};

    fn report_with(findings: Vec<Finding>) -> AuditReport {
        AuditReport {
            root: ".".into(),
            detected: vec!["claude".into()],
            agent_count: 2,
            skill_count: 1,
            findings,
        }
    }

    fn finding(rule: &'static str, severity: Severity) -> Finding {
        Finding {
            rule,
            severity,
            file: ".claude/agents/x.md".into(),
            message: "msg".into(),
            suggestion: Some("fix".into()),
        }
    }

    #[test]
    fn critical_count_counts_only_critical() {
        let r = report_with(vec![
            finding("A03", Severity::Critical),
            finding("A05", Severity::Warning),
        ]);
        assert_eq!(r.critical_count(), 1);
    }

    #[test]
    fn markdown_contains_summary_and_funnel_block() {
        let r = report_with(vec![finding("A03", Severity::Critical)]);
        let md = r.to_markdown();
        assert!(md.contains("# armadai audit"));
        assert!(md.contains("| A03 |"));
        assert!(md.contains("1 critical"));
        assert!(md.contains("What ArmadAI would give you"));
        assert!(md.contains("1 deprecated model(s) remapped automatically"));
    }

    #[test]
    fn html_contains_document_structure_and_summary() {
        let r = report_with(vec![finding("A03", Severity::Critical)]);
        let html = r.to_html();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("armadai audit"));
        assert!(html.contains("1 critical"));
        assert!(html.contains("CRIT"));
    }

    #[test]
    fn html_escapes_dynamic_content() {
        let mut f = finding("A03", Severity::Critical);
        f.message = "a <b> & \"c\" | d".into();
        let r = report_with(vec![f]);
        let html = r.to_html();
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;"));
        assert!(!html.contains("<b>"));
    }
}
