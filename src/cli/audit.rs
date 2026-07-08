use std::path::PathBuf;

use crate::audit::{
    rules::{AuditSettings, Severity},
    run_audit,
};

pub async fn execute(path: Option<PathBuf>, report: Option<PathBuf>) -> anyhow::Result<()> {
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
    audit.print_terminal(Severity::Info);
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
    if audit.critical_count() > 0 {
        anyhow::bail!("{} critical finding(s)", audit.critical_count());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_fails_on_missing_path() {
        let result = execute(Some(PathBuf::from("/nonexistent/xyz")), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_fails_on_critical_finding() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("bad.md"), "---\nname: [broken\n---\nBody").unwrap();
        let result = execute(Some(dir.path().to_path_buf()), None).await;
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
        let result = execute(Some(dir.path().to_path_buf()), Some(report_path.clone())).await;
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
        let result = execute(Some(dir.path().to_path_buf()), Some(report_path.clone())).await;
        assert!(result.is_ok());
        let html = std::fs::read_to_string(report_path).unwrap();
        assert!(html.starts_with("<!doctype html>"));
    }
}
