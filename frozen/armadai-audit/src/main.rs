//! FROZEN: the `armadai audit` engine, isolated from the workspace until it is
//! rebuilt on the declarative model (post-v1). See `README.md`.
//!
//! The command line is the `audit` subcommand of the `armadai` binary at commit
//! c486959, unchanged, now run as `armadai-audit [PATH] [OPTIONS]`.

mod audit;
mod cli_audit;
mod vendored;

use clap::Parser;

/// Audit native agentic configs (Claude Code) and report issues
#[derive(Parser)]
#[command(
    name = "armadai-audit",
    version,
    long_about = "Audit native agentic configs and report issues.\n\n\
        Scans .claude/agents/, .claude/skills/ and CLAUDE.md (no ArmadAI setup \
        required), runs static rules (deprecated models, oversized prompts, \
        duplicated blocks, broken references, plaintext secrets...) and prints \
        an actionable report. It also reads this project's Claude Code transcripts \
        under ~/.claude/projects/ to measure observed usage (rules U01-U04) — that \
        data never leaves this machine; pass --no-usage or set `audit.usage: false` \
        in the project config to skip it. Exits non-zero if critical findings exist.\n\n\
        With --global, audits what you carry into every session instead: \
        ~/.claude/agents/, ~/.claude/skills/, ~/.claude/CLAUDE.md and \
        ~/.config/armadai/skills/. Every rule family applies except U01-U04, which \
        correlate one project's transcripts. The synced catalogue \
        (~/.config/armadai/registry) is never read, in either scope.\n\n\
        FROZEN BUILD: --deep is parsed but unavailable (no execution provider).",
    after_help = "Examples:\n  \
        armadai-audit\n  \
        armadai-audit --global\n  \
        armadai-audit --report report.html"
)]
struct Args {
    /// Project directory to audit (defaults to current directory)
    path: Option<std::path::PathBuf>,
    /// Audit the user's global library instead of a project
    #[arg(long, conflicts_with = "path")]
    global: bool,
    /// Write a report to this file (markdown, or HTML if the extension is .html)
    #[arg(long)]
    report: Option<std::path::PathBuf>,
    /// Only display findings at or above this severity (exit code still counts everything)
    #[arg(long, value_parser = ["crit", "warn", "info"], default_value = "info")]
    min_severity: String,
    /// Shortcut for --min-severity warn
    #[arg(long, conflicts_with = "min_severity")]
    quiet: bool,
    /// Generate an installable ArmadAI pack from the audited config (.armadai-proposal/)
    #[arg(long)]
    propose: bool,
    /// Run an optional LLM pass — unavailable in this frozen build, fails explicitly
    #[arg(long)]
    deep: bool,
    /// Skip scanning Claude Code transcripts for observed usage (overrides `audit.usage` in project config)
    #[arg(long)]
    no_usage: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    cli_audit::execute(
        a.path,
        a.global,
        a.report,
        a.min_severity,
        a.quiet,
        a.propose,
        a.deep,
        a.no_usage,
    )
    .await
}
