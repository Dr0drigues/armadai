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
}
