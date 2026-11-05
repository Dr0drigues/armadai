//! `armadai flow status`: where each flow of a Claude Code session stands,
//! from the logs the recording hooks write.
//!
//! The project root is resolved exactly as the hooks resolve it (from a
//! subdirectory or a git worktree), so the logs read are the ones written.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::Project;

use crate::flow_status::line::Piece;
use crate::flow_status::record::{FlowEvent, GROUP_SEP, log_path};
use crate::flow_status::render;
use crate::flow_status::runs::{Mark, Run, runs};

const NO_ACTIVITY: &str =
    "no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)";

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(clap::Subcommand)]
pub enum FlowCommand {
    /// Show where each flow of a Claude Code session stands
    #[command(
        long_about = "Show where each flow of a Claude Code session stands, from the events \
            the recording hooks wrote under .armadai/state/claude/ (installed by `armadai link` \
            for the claude target when the project has a flow).\n\n\
            Each run of a flow lists its steps (✓ done with its state, ● running, ○ pending, \
            ? unsure with the reason, ✗ a state the step does not declare, ~ held by the \
            coordinator, which is not recorded, - not run in this session: before the step a \
            run opened at, when the session started mid-flow), the links taken more than once, \
            and the timed transitions (UTC). A run is attributed to the flow whose id is the command you typed (when the recording hook saw it) until \
            it finishes, then the sub-agents are inferred again; it is shown `interrupted` when a \
            later flow command closed it before it finished. Without --session, the latest \
            session.\n\n\
            --json prints a snapshot for other tools; --line prints one line for \
            a status bar (Claude Code's statusLine, or a custom command in your own bar).",
        after_help = "Examples:\n  \
            armadai flow status\n  \
            armadai flow status deliver --watch\n  \
            armadai flow status --list\n  \
            armadai flow status --session 3f2a9c\n  \
            armadai flow status --line\n  \
            armadai flow status --json"
    )]
    Status {
        /// Only the runs of this flow (id under .armadai/flows/)
        flow: Option<String>,
        /// Session id (see --list); the latest one when omitted
        #[arg(long)]
        session: Option<String>,
        /// Redraw on every new event (checked each second) until Ctrl-C
        #[arg(long, conflicts_with = "list")]
        watch: bool,
        /// List the recorded sessions instead
        #[arg(long, conflicts_with_all = ["flow", "session"])]
        list: bool,
        /// Print the session as JSON (schema armadai.flow-status/1) on stdout, and nothing else
        #[arg(long, conflicts_with_all = ["watch", "list"])]
        json: bool,
        /// Print one line for a status bar: the flow, its current step and its progress; reads session_id and cwd from a JSON on stdin when piped; never fails
        #[arg(long, conflicts_with_all = ["watch", "list", "json"])]
        line: bool,
        /// When to colour the output: auto (a terminal; NO_COLOR and CLICOLOR_FORCE honoured), always, never
        #[arg(long, value_enum, default_value = "auto")]
        color: ColorArg,
    },
}

pub fn execute(cmd: FlowCommand) -> anyhow::Result<()> {
    let FlowCommand::Status {
        flow,
        session,
        watch,
        list,
        json,
        line,
        color,
    } = cmd;
    anstream::ColorChoice::write_global(match color {
        ColorArg::Auto => anstream::ColorChoice::Auto,
        ColorArg::Always => anstream::ColorChoice::Always,
        ColorArg::Never => anstream::ColorChoice::Never,
    });
    if line {
        anstream::println!("{}", line_text(flow, session));
        return Ok(());
    }
    let cwd = std::env::current_dir()?;
    let Some(root) = crate::claude_adapter::policy_gate::resolve_project(&cwd) else {
        // the same advice as every project command (legacy included)
        crate::compile::find_project(&cwd)?;
        bail!("no ArmadAI project found (.armadai/project.yaml)");
    };
    let view = crate::pack::effective::open(&root, crate::pack::effective::Vars::Literal)?;
    let project = match view.load_checked() {
        Ok((p, _)) => p,
        Err(ds) => {
            for d in &ds {
                let s = crate::cli::style::warn();
                anstream::eprintln!("{s}{}{s:#}", d.render());
            }
            bail!("the project is invalid; run `armadai validate`")
        }
    };
    if let Some(id) = &flow
        && !project.flows.contains_key(id)
    {
        if project.flows.is_empty() {
            bail!("unknown flow `{id}`: the project has no flow");
        }
        bail!(
            "unknown flow `{id}`; flows: {}",
            project.flows.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    if list {
        let rows: Vec<(String, u64, usize)> = (render::sessions(&root).into_iter())
            .map(|(id, at)| {
                let n = log_path(&root, &id)
                    .map(|p| runs(&project, &read_log(&p)).len())
                    .unwrap_or(0);
                (id, at, n)
            })
            .collect();
        if rows.is_empty() {
            println!("{NO_ACTIVITY}");
        } else {
            print_painted(&render::render_sessions(&rows));
        }
        return Ok(());
    }
    if let Some(id) = &session {
        let path = log_path(&root, id).with_context(|| {
            format!("`{id}` is not a session id; see `armadai flow status --list`")
        })?;
        if !watch && !json && !path.is_file() {
            bail!("no session `{id}` recorded; see `armadai flow status --list`");
        }
    }
    let view = View {
        root,
        project,
        flow,
        session,
    };
    if json {
        let (session, runs) = view.snapshot();
        let doc = crate::flow_status::json::document(
            session
                .as_ref()
                .map(|(id, ev)| (id.as_str(), ev.as_slice())),
            &runs,
        );
        println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }
    if !watch {
        print_painted(&view.text());
        return Ok(());
    }
    // shortcut: polls each second rather than watching the file; enough for a terminal view.
    let mut last: Option<(Option<String>, Option<u64>)> = None;
    loop {
        let (session, path) = view.log();
        let len = path
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        let redraw = match &last {
            None => true,
            Some((s, old)) => *s != session || render::should_redraw(*old, len),
        };
        if redraw {
            anstream::print!("\x1b[2J\x1b[H");
            print_painted(&view.text());
        }
        last = Some((session, len));
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

struct View {
    root: PathBuf,
    project: Project,
    flow: Option<String>,
    session: Option<String>,
}

impl View {
    /// The session shown (the chosen one, else the latest) and its log.
    fn log(&self) -> (Option<String>, Option<PathBuf>) {
        let session = (self.session.clone()).or_else(|| render::latest_session(&self.root));
        let path = session.as_deref().and_then(|s| log_path(&self.root, s));
        (session, path)
    }

    /// The session shown with the events of its log (none when it has no
    /// file) and its runs, filtered by the flow id; the one reading of the
    /// log `text` and `--json` share.
    fn snapshot(&self) -> (Option<(String, Vec<FlowEvent>)>, Vec<Run>) {
        let (Some(session), path) = self.log() else {
            return (None, Vec::new());
        };
        let events = (path.filter(|p| p.is_file()))
            .map(|p| read_log(&p))
            .unwrap_or_default();
        let mut found = runs(&self.project, &events);
        if let Some(id) = &self.flow {
            found.retain(|r| r.flow.split(GROUP_SEP).any(|f| f == id));
        }
        (Some((session, events)), found)
    }

    fn text(&self) -> String {
        let (session, found) = self.snapshot();
        // no session, or one whose file is not written yet (--watch)
        match (session, self.log().1) {
            (Some((session, _)), Some(path)) if path.is_file() => render::render(&found, &session),
            _ => format!("{NO_ACTIVITY}\n"),
        }
    }
}

/// The events of a log; a line that does not parse (cut by a crash) is skipped.
fn read_log(path: &Path) -> Vec<FlowEvent> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Print the plain text of `render`, coloured line by line as `list` does.
fn print_painted(text: &str) {
    use crate::cli::style;
    for line in text.lines() {
        let s = match line.trim_start().chars().next() {
            _ if line.starts_with("session ") || line.starts_with("SESSION ") => style::header(),
            Some('✓') => style::ok(),
            Some('●') => style::running(),
            Some('○' | '~') => style::muted(),
            Some('?') => style::warn(),
            Some('✗') => style::err(),
            // a run's heading: `<flow>  in progress|interrupted|finished`
            _ if !line.starts_with(' ') && line.contains("  ") => style::accent(),
            _ => anstyle::Style::new(),
        };
        anstream::println!("{s}{line}{s:#}");
    }
}

/// How long `--line` waits for a JSON on stdin before ignoring it.
const STDIN_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

/// The JSON a status bar pipes in; `None` on a terminal, on a stdin that stays
/// silent, or on anything that is not a JSON object.
fn stdin_json() -> Option<serde_json::Value> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    // shortcut: the reader thread is left behind when stdin stays open; the process exits anyway
    std::thread::spawn(move || {
        let mut raw = String::new();
        let _ = std::io::stdin().read_to_string(&mut raw);
        let _ = tx.send(raw);
    });
    let raw = rx.recv_timeout(STDIN_WAIT).ok()?;
    serde_json::from_str(&raw)
        .ok()
        .filter(serde_json::Value::is_object)
}

/// The coloured line of `--line` (empty on any failure: a status bar must
/// never see an error).
fn line_text(flow: Option<String>, session: Option<String>) -> String {
    let Some(runs) = line_runs(flow, session) else {
        return String::new();
    };
    painted(&crate::flow_status::line::line(&runs))
}

/// The pieces joined by single spaces, each wrapped in its style (the
/// printing macros keep or strip the escapes as `--color` says).
fn painted(pieces: &[(Piece, String)]) -> String {
    use crate::cli::style;
    pieces
        .iter()
        .map(|(piece, text)| {
            let s = match piece {
                Piece::Flow => style::accent(),
                Piece::Progress | Piece::Status => style::muted(),
                Piece::Loops => style::warn(),
                Piece::Mark(m) => match m {
                    Mark::Done => style::ok(),
                    Mark::Running => style::running(),
                    Mark::Pending | Mark::Relay => style::muted(),
                    Mark::Unsure => style::warn(),
                    Mark::Unexpected => style::err(),
                    Mark::NotRun => anstyle::Style::new(),
                },
            };
            format!("{s}{text}{s:#}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The runs `--line` shows: the session is `--session`, else the `session_id`
/// of the JSON on stdin, else the latest log of the project (which is the
/// project of the JSON's `cwd`, else the current directory). `None` on any failure.
fn line_runs(flow: Option<String>, session: Option<String>) -> Option<Vec<Run>> {
    let json = stdin_json();
    let field = |k: &str| json.as_ref()?.get(k)?.as_str().map(str::to_string);
    let cwd = field("cwd")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok())?;
    let root = crate::claude_adapter::policy_gate::resolve_project(&cwd)?;
    let view = crate::pack::effective::open(&root, crate::pack::effective::Vars::Literal).ok()?;
    let (project, _) = view.load_checked().ok()?;
    if flow
        .as_ref()
        .is_some_and(|f| !project.flows.contains_key(f))
    {
        return None;
    }
    let session = session.or_else(|| field("session_id"));
    if let Some(id) = &session {
        log_path(&root, id)?;
    }
    let view = View {
        root,
        project,
        flow,
        session,
    };
    Some(view.snapshot().1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::style;

    fn paint(s: anstyle::Style, t: &str) -> String {
        format!("{s}{t}{s:#}")
    }

    #[test]
    fn every_piece_has_its_own_style() {
        let pieces = vec![
            (Piece::Flow, "deliver".to_string()),
            (Piece::Mark(Mark::Running), "●".to_string()),
            (Piece::Mark(Mark::Running), "guard".to_string()),
            (Piece::Progress, "2/4".to_string()),
            (Piece::Loops, "(loops 1)".to_string()),
            (Piece::Status, "finished".to_string()),
        ];
        let want = [
            paint(style::accent(), "deliver"),
            paint(style::running(), "●"),
            paint(style::running(), "guard"),
            paint(style::muted(), "2/4"),
            paint(style::warn(), "(loops 1)"),
            paint(style::muted(), "finished"),
        ]
        .join(" ");
        assert_eq!(painted(&pieces), want);
    }

    #[test]
    fn each_mark_has_its_style_and_a_step_not_run_has_none() {
        let cases = [
            (Mark::Done, style::ok()),
            (Mark::Running, style::running()),
            (Mark::Pending, style::muted()),
            (Mark::Relay, style::muted()),
            (Mark::Unsure, style::warn()),
            (Mark::Unexpected, style::err()),
            (Mark::NotRun, anstyle::Style::new()),
        ];
        for (m, s) in cases {
            let name = m.name();
            assert_eq!(
                painted(&[(Piece::Mark(m), "x".to_string())]),
                paint(s, "x"),
                "{name}"
            );
        }
    }
}
