use std::path::PathBuf;

use crate::audit::{
    import_surfaces,
    proposal::generate_proposal,
    rules::{AuditSettings, Severity},
    run_audit,
};

pub(crate) fn min_severity_from(flag: &str, quiet: bool) -> Severity {
    if quiet {
        return Severity::Warning;
    }
    match flag {
        "crit" => Severity::Critical,
        "warn" => Severity::Warning,
        _ => Severity::Info,
    }
}

pub async fn execute(
    path: Option<PathBuf>,
    report: Option<PathBuf>,
    min_severity: String,
    quiet: bool,
    propose: bool,
) -> anyhow::Result<()> {
    let root = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    if !root.is_dir() {
        anyhow::bail!("not a directory: {}", root.display());
    }
    let settings = AuditSettings::from_project(&root);
    let audit = run_audit(&root, &settings);
    if audit.detected.is_empty() {
        println!(
            "No native agentic configuration detected in {}.",
            root.display()
        );
        return Ok(());
    }
    audit.print_terminal(min_severity_from(&min_severity, quiet));
    if let Some(out) = report {
        let is_html = out
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
            .unwrap_or(false);
        if is_html {
            std::fs::write(&out, audit.to_html())?;
            println!("\n  HTML report written to {}", out.display());
        } else {
            std::fs::write(&out, audit.to_markdown())?;
            println!("\n  Markdown report written to {}", out.display());
        }
    }
    if propose {
        let (_, config) = import_surfaces(&root);
        let summary = generate_proposal(&root, &config)?;
        println!();
        println!("  Proposal written to {}/", summary.out_dir.display());
        println!(
            "    {} agent(s), {} shared prompt(s), {} skill(s) ({} fixed)",
            summary.agents, summary.prompts, summary.skills, summary.skill_fixes
        );
        if !summary.skipped_agents.is_empty() {
            println!(
                "    {} agent(s) skipped (unreadable): {}",
                summary.skipped_agents.len(),
                summary.skipped_agents.join(", ")
            );
        }
        println!("  Install it with:");
        println!(
            "    armadai init --pack {} --project",
            summary.out_dir.display()
        );
    }
    if audit.critical_count() > 0 {
        anyhow::bail!("{} critical finding(s)", audit.critical_count());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_severity_mapping() {
        use crate::audit::rules::Severity;
        assert_eq!(min_severity_from("info", false), Severity::Info);
        assert_eq!(min_severity_from("warn", false), Severity::Warning);
        assert_eq!(min_severity_from("crit", false), Severity::Critical);
        assert_eq!(min_severity_from("info", true), Severity::Warning); // --quiet wins
    }

    #[tokio::test]
    async fn execute_fails_on_missing_path() {
        let result = execute(
            Some(PathBuf::from("/nonexistent/xyz")),
            None,
            "info".to_string(),
            false,
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_fails_on_critical_finding() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("bad.md"), "---\nname: [broken\n---\nBody").unwrap();
        let result = execute(
            Some(dir.path().to_path_buf()),
            None,
            "info".to_string(),
            false,
            false,
        )
        .await;
        assert!(result.is_err()); // A01 critical -> non-zero exit
    }

    #[tokio::test]
    async fn execute_succeeds_on_clean_repo_and_writes_report() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ok.md"),
            "---\nname: ok\ndescription: Fine agent\nmodel: latest:pro\ntools: Read\n---\nShort prompt.",
        )
        .unwrap();
        let report_path = dir.path().join("audit.md");
        let result = execute(
            Some(dir.path().to_path_buf()),
            Some(report_path.clone()),
            "info".to_string(),
            false,
            false,
        )
        .await;
        assert!(result.is_ok());
        let md = std::fs::read_to_string(report_path).unwrap();
        assert!(md.contains("# armadai audit"));
    }

    #[tokio::test]
    async fn execute_writes_html_when_extension_is_html() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ok.md"),
            "---\nname: ok\ndescription: Fine agent\nmodel: latest:pro\ntools: Read\n---\nShort prompt.",
        )
        .unwrap();
        let report_path = dir.path().join("audit.html");
        let result = execute(
            Some(dir.path().to_path_buf()),
            Some(report_path.clone()),
            "info".to_string(),
            false,
            false,
        )
        .await;
        assert!(result.is_ok());
        let html = std::fs::read_to_string(report_path).unwrap();
        assert!(html.starts_with("<!doctype html>"));
    }

    #[tokio::test]
    async fn execute_with_propose_writes_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ok.md"),
            "---\nname: ok\ndescription: Fine\nmodel: latest:pro\ntools: Read\n---\nShort prompt.",
        )
        .unwrap();
        let result = execute(
            Some(dir.path().to_path_buf()),
            None,
            "info".to_string(),
            false,
            true,
        )
        .await;
        assert!(result.is_ok());
        assert!(dir.path().join(".armadai-proposal/pack.yaml").is_file());
        assert!(dir.path().join(".armadai-proposal/agents/ok.md").is_file());
    }
}
