use std::path::PathBuf;

use crate::audit::{
    AuditInput, AuditScope, GlobalLayout,
    deep::{DeepOutcome, available_cli, run_deep},
    proposal::generate_proposal,
    reverse::ImportedConfig,
    rules::{AuditSettings, Severity},
};
use armadai_core::agent::{Agent, AgentMetadata};
use armadai_core::provider::{ChatMessage, CompletionRequest};
use armadai_providers::factory::create_provider;

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

/// What `--deep` is about to send, and — in global scope — whose material it
/// is.
///
/// Returned as a `String` rather than printed inline so the wording is
/// assertable: the two scopes cross a different privacy boundary, and a single
/// shared sentence silently understated the wider one. In project scope the
/// excerpts come from a repository's shared config; in global scope they are
/// the user's own `~/.claude/CLAUDE.md` and the prompts of their global
/// agents. That is the same boundary that kept `R03` out of the rule set, so
/// the warning has to name it.
fn deep_privacy_note(scope: AuditScope, cli: &str) -> String {
    let mut note = format!(
        "Note: --deep sends (secret-redacted) prompt excerpts, and any observed-usage \
         finding message (U01-U04), to the '{cli}' CLI."
    );
    if scope == AuditScope::Global {
        note.push_str(
            " In global scope those excerpts are your own material: \
             ~/.claude/CLAUDE.md and the prompts of your global agents, not a \
             repository's shared config.",
        );
    }
    note
}

/// Run the `--deep` LLM analysis pass and merge its outcome into `audit`.
///
/// `cli` is the already-detected CLI name (or `None` if no supported LLM CLI
/// was found in `PATH`); it is passed in rather than re-detected here so
/// callers (and tests) can inject the detection result instead of mutating
/// the process environment.
///
/// `config` is the surfaces the caller already imported, rather than a root
/// to import again: the global scope has no single root to re-derive them
/// from, and re-reading them was a third full pass over the same files.
async fn apply_deep_pass(
    audit: &mut crate::audit::report::AuditReport,
    config: &ImportedConfig,
    settings: &AuditSettings,
    cli: Option<&str>,
) -> anyhow::Result<()> {
    let Some(cli) = cli else {
        anyhow::bail!("--deep requires an LLM CLI (claude, gemini); none found in PATH");
    };
    let agent = build_deep_auditor(cli);
    let run = |prompt: &str| call_deep_auditor(&agent, prompt);
    let w = crate::cli::style::warn();
    let note = deep_privacy_note(audit.scope, cli);
    anstream::eprintln!("{w}  {note}{w:#}");
    match run_deep(
        config,
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

// One parameter per CLI flag, as every other command in this module does
// (`run`, `link`, `unlink`): the dispatcher in `cli/mod.rs` destructures the
// clap variant and forwards it, so an args struct here would only move the
// arity one level up.
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    path: Option<PathBuf>,
    global: bool,
    report: Option<PathBuf>,
    min_severity: String,
    quiet: bool,
    propose: bool,
    deep: bool,
    no_usage: bool,
) -> anyhow::Result<()> {
    // The working root. In global scope it is not what gets audited, and no
    // longer what tunes the thresholds either (see `settings` below) — it is
    // only where `--propose` writes its pack, i.e. wherever the user stands.
    let root = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    if !root.is_dir() {
        anyhow::bail!("not a directory: {}", root.display());
    }
    // Thresholds follow the audited surface, not the working directory: a
    // global audit read from `<cwd>/armadai.yaml` gave a different verdict on
    // the same library depending on which folder it was launched from
    // (measured: 2 `R01` warnings from a directory carrying
    // `skill_token_threshold: 5`, 0 from a neutral one). The global library's
    // own settings live with it, in `~/.config/armadai/config.yaml`.
    let settings = if global {
        AuditSettings::from_global()
    } else {
        AuditSettings::from_project(&root)
    };
    // Import first: on the "nothing here" path, there is nothing to audit
    // and nothing to propose, so the (potentially hundreds-of-megabytes)
    // transcript scan below must never run for it. The imported surfaces are
    // kept around so `--propose` and `--deep` reuse them instead of reading
    // the same files a second and third time.
    let input = if global {
        // No `$HOME`, no `~`, nothing global to audit. Falling back to `.`
        // reported the current repository's `.claude/` as the user's library,
        // labelled `~/.claude` — a wrong answer indistinguishable from a right
        // one, which is worse than a refusal.
        let Some(layout) = GlobalLayout::from_env() else {
            anyhow::bail!(
                "--global audits what lives under ~, and $HOME is not set; \
                 set it, or audit a path instead"
            );
        };
        AuditInput::for_global(&layout)
    } else {
        AuditInput::for_project(&root)
    };
    if input.detected().is_empty() {
        let o = crate::cli::style::ok();
        let m = crate::cli::style::muted();
        let where_ = match input.scope() {
            AuditScope::Global => "your global library".to_string(),
            AuditScope::Project => format!("{}", root.display()),
        };
        anstream::println!("{o}No native agentic configuration detected in{o:#} {m}{where_}.{m:#}");
        return Ok(());
    }
    // The flag always wins over the config key (`audit.usage`); the config
    // key only takes effect when the flag is absent. Global scope never
    // scans: U01-U04 correlate *one project's* transcripts, and the global
    // library belongs to no project — so the (potentially very large) scan is
    // skipped outright rather than run and then ignored.
    let usage_enabled = !global && !no_usage && settings.usage;
    // Scanned at most once here (only when something was detected and usage
    // measurement is enabled) and bound for the rest of the command —
    // transcripts can run into the hundreds of megabytes.
    let usage = usage_enabled.then(|| crate::audit::usage::scan(&root));
    let usage = usage.filter(|o| !o.is_empty());
    let mut audit = input.analyse(&settings, usage.as_ref());
    if deep {
        apply_deep_pass(&mut audit, input.config(), &settings, available_cli()).await?;
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
            let o = crate::cli::style::ok();
            let m = crate::cli::style::muted();
            anstream::println!(
                "\n  {o}HTML report written to{o:#} {m}{}{m:#}",
                out.display()
            );
        } else {
            std::fs::write(&out, audit.to_markdown())?;
            let o = crate::cli::style::ok();
            let m = crate::cli::style::muted();
            anstream::println!(
                "\n  {o}Markdown report written to{o:#} {m}{}{m:#}",
                out.display()
            );
        }
    }
    if propose {
        let summary = generate_proposal(&root, input.config())?;
        anstream::println!();
        let o = crate::cli::style::ok();
        let m = crate::cli::style::muted();
        anstream::println!(
            "  {o}Proposal written to{o:#} {m}{}/{m:#}",
            summary.out_dir.display()
        );
        anstream::println!(
            "{m}    {} agent(s), {} shared prompt(s), {} skill(s) ({} fixed){m:#}",
            summary.agents,
            summary.prompts,
            summary.skills,
            summary.skill_fixes
        );
        if !summary.skipped_agents.is_empty() {
            let w = crate::cli::style::warn();
            anstream::println!(
                "{w}    {} agent(s) skipped (unreadable): {}{w:#}",
                summary.skipped_agents.len(),
                summary.skipped_agents.join(", ")
            );
        }
        anstream::println!("{m}  Install it with:{m:#}");
        anstream::println!(
            "{m}    armadai init --pack {} --project{m:#}",
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

    /// Points `ARMADAI_CLAUDE_PROJECTS_DIR` at a fresh, empty tempdir so
    /// `execute()`'s unconditional `usage::scan(&root)` call never reads the
    /// real machine's `~/.claude/projects` — non-deterministic across
    /// machines, and on a machine with a real transcript corpus, actively
    /// wrong for tests that assert on a clean, controlled fixture. Mirrors
    /// `discovery`'s own `ProjectsDirGuard`.
    ///
    /// The `MutexGuard` is a struct field, not a bare local binding, which is
    /// what keeps clippy's `await_holding_lock` from firing when this guard
    /// is held across the `.await` in an `execute(...)` test below — the same
    /// shape as `TempStorageGuard` in `cli/run.rs` and the guards in
    /// `cli/watch.rs`.
    struct ProjectsDirGuard {
        _dir: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl ProjectsDirGuard {
        fn empty() -> Self {
            let lock = armadai_core::test_support::env_lock();
            let dir = tempfile::tempdir().unwrap();
            // SAFETY: modifies the global environment; serialised via `env_lock()`.
            unsafe { std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", dir.path()) }
            Self {
                _dir: dir,
                _lock: lock,
            }
        }

        /// Same guard, pointed at a caller-supplied directory already
        /// populated with transcripts (see `usage_scenario`) rather than a
        /// fresh empty one.
        fn at(dir: tempfile::TempDir) -> Self {
            let lock = armadai_core::test_support::env_lock();
            // SAFETY: modifies the global environment; serialised via `env_lock()`.
            unsafe { std::env::set_var("ARMADAI_CLAUDE_PROJECTS_DIR", dir.path()) }
            Self {
                _dir: dir,
                _lock: lock,
            }
        }
    }

    impl Drop for ProjectsDirGuard {
        fn drop(&mut self) {
            // SAFETY: restoring env state at end of test scope.
            unsafe { std::env::remove_var("ARMADAI_CLAUDE_PROJECTS_DIR") }
        }
    }

    /// Builds a project declaring one agent that never ran, plus a
    /// transcript in which Claude Code's built-in `general-purpose` did the
    /// work — the minimal shape that makes both the "Observed usage" section
    /// and a U0x finding (U01 on `ghost`, U02 on `general-purpose`) appear
    /// by default, so the `--no-usage`/`audit.usage` tests below have
    /// something real to suppress.
    fn usage_scenario() -> (tempfile::TempDir, tempfile::TempDir) {
        let project = tempfile::tempdir().unwrap();
        let agents = project.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ghost.md"),
            "---\nname: ghost\ndescription: never invoked\n---\nBody",
        )
        .unwrap();
        let projects = tempfile::tempdir().unwrap();
        let session_dir = projects.path().join("session");
        std::fs::create_dir_all(&session_dir).unwrap();
        let cwd = project.path().to_string_lossy().to_string();
        std::fs::write(
            session_dir.join("s1.jsonl"),
            format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"2026-08-01T00:00:00Z\",\
                 \"isSidechain\":false,\"uuid\":\"u1\",\"cwd\":\"{cwd}\",\"message\":{{\
                 \"model\":\"m\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"t1\",\
                 \"name\":\"Agent\",\"input\":{{\"subagent_type\":\"general-purpose\",\
                 \"description\":\"work\"}}}}],\"usage\":{{\"input_tokens\":1,\
                 \"output_tokens\":1}}}}}}\n"
            ),
        )
        .unwrap();
        (project, projects)
    }

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
            false,
            None,
            "info".to_string(),
            false,
            false,
            false,
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_fails_on_critical_finding() {
        let _env = ProjectsDirGuard::empty();
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("bad.md"), "---\nname: [broken\n---\nBody").unwrap();
        let result = execute(
            Some(dir.path().to_path_buf()),
            false,
            None,
            "info".to_string(),
            false,
            false,
            false,
            false,
        )
        .await;
        assert!(result.is_err()); // A01 critical -> non-zero exit
    }

    #[tokio::test]
    async fn execute_succeeds_on_clean_repo_and_writes_report() {
        let _env = ProjectsDirGuard::empty();
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
            false,
            Some(report_path.clone()),
            "info".to_string(),
            false,
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
        let _env = ProjectsDirGuard::empty();
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
            false,
            Some(report_path.clone()),
            "info".to_string(),
            false,
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
        let _env = ProjectsDirGuard::empty();
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
            false,
            None,
            "info".to_string(),
            false,
            true,
            false,
            false,
        )
        .await;
        assert!(result.is_ok());
        assert!(dir.path().join(".armadai-proposal/pack.yaml").is_file());
        assert!(dir.path().join(".armadai-proposal/agents/ok.md").is_file());
    }

    /// Baseline: with real transcripts and no opt-out, the usage section and
    /// its findings appear. Establishes that the fixture in `usage_scenario`
    /// actually exercises what the suppression tests below claim to
    /// suppress.
    #[tokio::test]
    async fn usage_section_and_findings_appear_by_default() {
        let (project, projects) = usage_scenario();
        let _env = ProjectsDirGuard::at(projects);
        let report_path = project.path().join("audit.md");
        let result = execute(
            Some(project.path().to_path_buf()),
            false,
            Some(report_path.clone()),
            "info".to_string(),
            false,
            false,
            false,
            false, // no_usage
        )
        .await;
        assert!(result.is_ok());
        let md = std::fs::read_to_string(report_path).unwrap();
        assert!(md.contains("Observed usage"), "{md}");
        assert!(md.contains("U01") && md.contains("U02"), "{md}");
    }

    /// `--no-usage` must skip the scan entirely: no section, no U0x finding.
    #[tokio::test]
    async fn no_usage_flag_suppresses_the_section_and_findings() {
        let (project, projects) = usage_scenario();
        let _env = ProjectsDirGuard::at(projects);
        let report_path = project.path().join("audit.md");
        let result = execute(
            Some(project.path().to_path_buf()),
            false,
            Some(report_path.clone()),
            "info".to_string(),
            false,
            false,
            false,
            true, // no_usage
        )
        .await;
        assert!(result.is_ok());
        let md = std::fs::read_to_string(report_path).unwrap();
        assert!(!md.contains("Observed usage"), "{md}");
        assert!(!md.contains("U01") && !md.contains("U02"), "{md}");
    }

    /// `audit.usage: false` in project config must have the same effect as
    /// the flag, when the flag itself is absent.
    #[tokio::test]
    async fn usage_config_key_suppresses_when_the_flag_is_absent() {
        let (project, projects) = usage_scenario();
        let _env = ProjectsDirGuard::at(projects);
        std::fs::write(
            project.path().join("armadai.yaml"),
            "audit:\n  usage: false\n",
        )
        .unwrap();
        let report_path = project.path().join("audit.md");
        let result = execute(
            Some(project.path().to_path_buf()),
            false,
            Some(report_path.clone()),
            "info".to_string(),
            false,
            false,
            false,
            false, // no_usage: flag absent
        )
        .await;
        assert!(result.is_ok());
        let md = std::fs::read_to_string(report_path).unwrap();
        assert!(!md.contains("Observed usage"), "{md}");
        assert!(!md.contains("U01") && !md.contains("U02"), "{md}");
    }

    /// Precedence: `--no-usage` wins even when the config explicitly enables
    /// usage measurement.
    #[tokio::test]
    async fn no_usage_flag_overrides_a_config_that_enables_usage() {
        let (project, projects) = usage_scenario();
        let _env = ProjectsDirGuard::at(projects);
        std::fs::write(
            project.path().join("armadai.yaml"),
            "audit:\n  usage: true\n",
        )
        .unwrap();
        let report_path = project.path().join("audit.md");
        let result = execute(
            Some(project.path().to_path_buf()),
            false,
            Some(report_path.clone()),
            "info".to_string(),
            false,
            false,
            false,
            true, // no_usage: the flag must win
        )
        .await;
        assert!(result.is_ok());
        let md = std::fs::read_to_string(report_path).unwrap();
        assert!(!md.contains("Observed usage"), "{md}");
        assert!(!md.contains("U01") && !md.contains("U02"), "{md}");
    }

    /// The two scopes cross a different privacy boundary, and the warning has
    /// to say which one. A single shared sentence understated the global case:
    /// it named "prompt excerpts" while what actually leaves the machine is
    /// the user's own instructions file and their personal agents' prompts.
    #[test]
    fn the_deep_warning_names_whose_material_leaves_the_machine() {
        let project = deep_privacy_note(AuditScope::Project, "claude");
        assert!(
            project.contains("'claude' CLI"),
            "the target CLI must be named: {project}"
        );
        assert!(
            !project.contains("global"),
            "a project run must not claim to send the user's own library: {project}"
        );

        let global = deep_privacy_note(AuditScope::Global, "claude");
        assert!(
            global.contains("~/.claude/CLAUDE.md"),
            "a global run must name the user's own instructions file: {global}"
        );
        assert!(
            global.contains("global agents"),
            "and the prompts of their global agents: {global}"
        );
    }

    #[tokio::test]
    async fn deep_pass_without_cli_errors_explicitly() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("a.md"), "---\nname: a\ndescription: d\n---\nP.").unwrap();
        let settings = AuditSettings::from_project(dir.path());
        let input = AuditInput::for_project(dir.path());
        let mut audit = input.analyse(&settings, None);
        let err = apply_deep_pass(&mut audit, input.config(), &settings, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--deep requires an LLM CLI"));
    }
}
