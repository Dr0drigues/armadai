use std::path::PathBuf;

use crate::audit::{
    deep::{DeepOutcome, available_cli, run_deep},
    import_surfaces,
    proposal::generate_proposal,
    rules::{AuditSettings, Severity},
    run_audit,
};
use crate::core::agent::{Agent, AgentMetadata};
use crate::providers::factory::create_provider;
use crate::providers::traits::{ChatMessage, CompletionRequest};

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

/// Build the in-memory auditor agent for the given detected CLI.
///
/// Only `claude` and `gemini` are supported (see `deep::DEEP_CLIS`), both
/// through the standard unified-tool resolution (`provider = cli`, no
/// explicit `command`), which invokes them non-interactively via `-p`.
fn build_deep_auditor(cli: &str) -> Agent {
    Agent {
        name: "deep-auditor".to_string(),
        source: PathBuf::from("<in-memory>"),
        metadata: AgentMetadata {
            provider: cli.to_string(),
            model: Some("latest:pro".to_string()),
            command: None,
            args: None,
            temperature: 0.2,
            max_tokens: None,
            timeout: None,
            tags: vec![],
            stacks: vec![],
            scope: vec![],
            model_fallback: vec![],
            cost_limit: None,
            rate_limit: None,
            context_window: None,
            mode: None,
            orchestration: None,
            triggers: None,
            ring_config: None,
        },
        system_prompt: String::new(),
        instructions: None,
        output_format: None,
        pipeline: None,
        context: None,
    }
}

/// Call the deep-pass auditor synchronously, bridging into the async
/// provider API. `execute` runs on the tokio multi-thread runtime installed
/// by `#[tokio::main]`, so `block_in_place` + `Handle::current().block_on`
/// is safe here (it is only reached once a real CLI has been detected —
/// the `deep_without_cli_errors_explicitly` test bails out before this
/// point and never exercises it on the single-threaded test runtime).
fn call_deep_auditor(agent: &Agent, prompt: &str) -> anyhow::Result<String> {
    let provider = create_provider(agent)?;
    let request = CompletionRequest {
        model: agent.metadata.model.clone().unwrap_or_default(),
        system_prompt: String::new(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
        temperature: 0.2,
        max_tokens: None,
    };
    let response = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(provider.complete(request))
    })?;
    Ok(response.content)
}

/// Run the `--deep` LLM analysis pass and merge its outcome into `audit`.
///
/// `cli` is the already-detected CLI name (or `None` if no supported LLM CLI
/// was found in `PATH`); it is passed in rather than re-detected here so
/// callers (and tests) can inject the detection result instead of mutating
/// the process environment.
async fn apply_deep_pass(
    audit: &mut crate::audit::report::AuditReport,
    root: &std::path::Path,
    settings: &AuditSettings,
    cli: Option<&str>,
) -> anyhow::Result<()> {
    let Some(cli) = cli else {
        anyhow::bail!("--deep requires an LLM CLI (claude, gemini); none found in PATH");
    };
    let (_, config) = import_surfaces(root);
    let agent = build_deep_auditor(cli);
    let run = |prompt: &str| call_deep_auditor(&agent, prompt);
    eprintln!("  Note: --deep sends (secret-redacted) prompt excerpts to the '{cli}' CLI.");
    match run_deep(
        &config,
        &audit.findings,
        settings.deep_prompt_truncation,
        run,
    )? {
        DeepOutcome::Findings(v) => {
            audit.findings.extend(v);
            audit
                .findings
                .sort_by(|a, b| (a.severity, &a.file, a.rule).cmp(&(b.severity, &b.file, b.rule)));
        }
        DeepOutcome::Raw(s) => {
            audit.deep_raw = Some(s);
        }
    }
    Ok(())
}

pub async fn execute(
    path: Option<PathBuf>,
    report: Option<PathBuf>,
    min_severity: String,
    quiet: bool,
    propose: bool,
    deep: bool,
) -> anyhow::Result<()> {
    let root = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    if !root.is_dir() {
        anyhow::bail!("not a directory: {}", root.display());
    }
    let settings = AuditSettings::from_project(&root);
    let mut audit = run_audit(&root, &settings);
    if audit.detected.is_empty() {
        println!(
            "No native agentic configuration detected in {}.",
            root.display()
        );
        return Ok(());
    }
    if deep {
        apply_deep_pass(&mut audit, &root, &settings, available_cli()).await?;
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
            false,
        )
        .await;
        assert!(result.is_ok());
        assert!(dir.path().join(".armadai-proposal/pack.yaml").is_file());
        assert!(dir.path().join(".armadai-proposal/agents/ok.md").is_file());
    }

    #[tokio::test]
    async fn deep_pass_without_cli_errors_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("a.md"), "---\nname: a\ndescription: d\n---\nP.").unwrap();
        let settings = AuditSettings::from_project(dir.path());
        let mut audit = run_audit(dir.path(), &settings);
        let err = apply_deep_pass(&mut audit, dir.path(), &settings, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--deep requires an LLM CLI"));
    }
}
