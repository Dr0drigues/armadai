//! `armadai flow status`: where each flow of a Claude Code session stands,
//! from the logs the recording hooks write.
//!
//! The project root is resolved exactly as the hooks resolve it (from a
//! subdirectory or a git worktree), so the logs read are the ones written.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::Project;

use crate::cli::term;
use crate::flow_status::line::{Piece, open_run};
use crate::flow_status::record::{FlowEvent, GROUP_SEP, log_path};
use crate::flow_status::render;
use crate::flow_status::runs::{Mark, Run, runs};
use crate::flow_status::watch;

const NO_ACTIVITY: &str =
    "no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)";

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(clap::ValueEnum, Clone, Copy, PartialEq)]
pub enum FormatArg {
    Text,
    Md,
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
            --json and --format md print a snapshot for other tools; --line prints one line for \
            a status bar (Claude Code's statusLine, or a custom command in your own bar).",
        after_help = "Examples:\n  \
            armadai flow status\n  \
            armadai flow status deliver --watch\n  \
            armadai flow status --list\n  \
            armadai flow status --session 3f2a9c\n  \
            armadai flow status --line\n  \
            armadai flow status --json\n  \
            armadai flow status --format md > status.md"
    )]
    Status {
        /// Only the runs of this flow (id under .armadai/flows/)
        flow: Option<String>,
        /// Session id (see --list); the latest one when omitted
        #[arg(long)]
        session: Option<String>,
        /// Follow the session until Ctrl-C: in a terminal, the running flow is drawn in boxes with its marks and a spinner on the running step; elsewhere, the text is printed again on each new event
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
        /// Output format: text, or md (a Markdown snapshot for GitHub, GitLab or VS Code)
        #[arg(long, value_enum, default_value = "text", conflicts_with_all = ["watch", "list", "json", "line"])]
        format: FormatArg,
        /// When to colour the output: auto (a terminal; NO_COLOR and CLICOLOR_FORCE honoured), always, never
        #[arg(long, value_enum, default_value = "auto")]
        color: ColorArg,
    },
    /// Show calls, durations, verdicts and loops of a session, per agent, step or lot
    #[command(
        long_about = "Show how a Claude Code session went, from the events the recording hooks \
            wrote under .armadai/state/claude/: per group, the calls, the resumed ones, the \
            median, maximum and total duration, the states rendered, the share of rejections \
            (`rejected` or `rejected-*`, for a step that declares one) and the loops.\n\n\
            A call is a sub-agent that started. Its duration runs from its start to its first \
            stop: an agent resumed by SendMessage counts once and its later segments are not \
            timed, so durations are lower bounds. --by agent needs no marker; --by step and \
            --by lot count the calls marked `[flow/step#lot]` (see `armadai link`) and set the \
            others aside. Without --session, the latest session.",
        after_help = "Examples:\n  \
            armadai flow stats\n  \
            armadai flow stats deliver --by step\n  \
            armadai flow stats --all --by lot\n  \
            armadai flow stats --json"
    )]
    Stats {
        /// Only the calls and loops of this flow (id under .armadai/flows/)
        flow: Option<String>,
        /// Session id (see `flow status --list`); the latest one when omitted
        #[arg(long, conflicts_with = "all")]
        session: Option<String>,
        /// Every recorded session together
        #[arg(long)]
        all: bool,
        /// What to group by
        #[arg(long, value_enum, default_value = "agent")]
        by: ByArg,
        /// Print JSON (schema armadai.flow-stats/1) on stdout, and nothing else
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum ByArg {
    Agent,
    Step,
    Lot,
}

/// The project root and the project (valid), with `flow` checked against its flows.
fn open_project(flow: &Option<String>) -> anyhow::Result<(PathBuf, Project)> {
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
    if let Some(id) = flow
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
    Ok((root, project))
}

fn stats_command(
    flow: Option<String>,
    session: Option<String>,
    all: bool,
    by: ByArg,
    json: bool,
) -> anyhow::Result<()> {
    use crate::flow_status::stats::{By, stats};
    let (root, project) = open_project(&flow)?;
    let ids: Vec<String> = if all {
        // oldest first, the order the events happened in
        (render::sessions(&root).into_iter().rev())
            .map(|s| s.0)
            .collect()
    } else if let Some(id) = session {
        let path = log_path(&root, &id).with_context(|| {
            format!("`{id}` is not a session id; see `armadai flow status --list`")
        })?;
        if !path.is_file() {
            bail!("no session `{id}` recorded; see `armadai flow status --list`");
        }
        vec![id]
    } else {
        render::latest_session(&root).into_iter().collect()
    };
    // shortcut: the sessions of --all are read as one log, so a run may straddle two sessions (only loops read from runs); split per session when that shows
    let events: Vec<FlowEvent> = (ids.iter())
        .filter_map(|id| log_path(&root, id))
        .flat_map(|p| read_log(&p))
        .collect();
    let by = match by {
        ByArg::Agent => By::Agent,
        ByArg::Step => By::Step,
        ByArg::Lot => By::Lot,
    };
    let found = stats(&project, &events, by, flow.as_deref());
    if json {
        let doc = crate::flow_status::json::stats_document(&ids, &found);
        anstream::println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }
    if let Some(f) = flow.as_deref().filter(|_| {
        // shortcut: --all has no wording of its own yet, it keeps the general line
        !all && !events.is_empty() && found.groups.is_empty() && found.unmarked_calls == 0
    }) {
        anstream::println!("no call of flow {f} in this session");
    } else if found.groups.is_empty() && found.unmarked_calls == 0 {
        anstream::println!("{NO_ACTIVITY}");
    } else if found.groups.is_empty() && !found.any_marked {
        anstream::println!("{}", render::NO_MARKER);
    } else {
        let scope = match ids.as_slice() {
            [one] if !all => format!("session {one}"),
            _ => format!("all sessions ({})", ids.len()),
        };
        anstream::print!("{}", render::render_stats(&found, &scope));
    }
    Ok(())
}

pub fn execute(cmd: FlowCommand) -> anyhow::Result<()> {
    let FlowCommand::Status {
        flow,
        session,
        watch,
        list,
        json,
        line,
        format,
        color,
    } = cmd
    else {
        let FlowCommand::Stats {
            flow,
            session,
            all,
            by,
            json,
        } = cmd
        else {
            unreachable!("two commands")
        };
        return stats_command(flow, session, all, by, json);
    };
    anstream::ColorChoice::write_global(match color {
        ColorArg::Auto => anstream::ColorChoice::Auto,
        ColorArg::Always => anstream::ColorChoice::Always,
        ColorArg::Never => anstream::ColorChoice::Never,
    });
    if line {
        anstream::println!("{}", line_text(flow, session));
        return Ok(());
    }
    let (root, project) = open_project(&flow)?;
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
            anstream::println!("{NO_ACTIVITY}");
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
        // anstream, like every other output here: a closed reader is not a panic
        anstream::println!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }
    if format == FormatArg::Md {
        anstream::print!("{}", view.md());
        return Ok(());
    }
    if !watch {
        print_painted(&view.text());
        return Ok(());
    }
    if std::io::stdout().is_terminal() {
        return view.animate();
    }
    // shortcut: polls each second rather than watching the file; enough for a pipe or a file.
    // No positioning sequence here, whatever `--color` says: the output is the list, as before.
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
            print_painted(&view.text());
        }
        last = Some((session, len));
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

/// The step of `--watch` in a terminal: a `stat` of the log, a redraw only when it changed
/// or a step runs, so the spinner turns four times a second.
const PACE: std::time::Duration = std::time::Duration::from_millis(250);

/// How often `--watch` in a terminal looks for a newer session (no `--session`).
const LATEST_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// The image to write: `None` when it is the one written last (never write it twice).
fn to_write<'a>(last: Option<&str>, image: &'a str) -> Option<&'a str> {
    (last != Some(image)).then_some(image)
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
        let (session, path) = self.log();
        self.snapshot_of(session, path)
    }

    /// `snapshot` of a session and log already found.
    fn snapshot_of(
        &self,
        session: Option<String>,
        path: Option<PathBuf>,
    ) -> (Option<(String, Vec<FlowEvent>)>, Vec<Run>) {
        let Some(session) = session else {
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

    fn md(&self) -> String {
        let (session, found) = self.snapshot();
        match (session, self.log().1) {
            (Some((session, events)), Some(path)) if path.is_file() => {
                render::render_md(&found, &session, events.last().map(|e| e.ts))
            }
            _ => format!("{NO_ACTIVITY}\n"),
        }
    }

    /// `--watch` in a terminal: the drawn flow, redrawn in place (no clear after the
    /// first image), the cursor hidden until the loop ends or Ctrl-C.
    fn animate(&self) -> anyhow::Result<()> {
        use std::io::Write;
        let _cursor = term::HiddenCursor::new()?;
        // the positioning sequences go to the raw stdout; only the content follows `--color`
        let colour =
            anstream::AutoStream::choice(&std::io::stdout()) != anstream::ColorChoice::Never;
        let mut key: Option<(Option<String>, Option<u64>)> = None;
        let mut cached: Option<(String, Vec<Run>)> = None;
        let mut last: Option<String> = None;
        let mut last_size = None;
        // the latest session is looked for (a `read_dir` and a `stat` per log) once a
        // second at most, like `master`; between two looks only its log is `stat`ed
        let mut latest: Option<(std::time::Instant, Option<String>)> = None;
        let (mut tick, mut first) = (0u64, true);
        loop {
            let session = match &self.session {
                Some(s) => Some(s.clone()),
                None => {
                    let stale = latest
                        .as_ref()
                        .is_none_or(|(at, _)| at.elapsed() >= LATEST_EVERY);
                    if stale {
                        latest = Some((
                            std::time::Instant::now(),
                            render::latest_session(&self.root),
                        ));
                    }
                    latest.as_ref().and_then(|(_, s)| s.clone())
                }
            };
            let path = session.as_deref().and_then(|s| log_path(&self.root, s));
            let len = (path.as_ref())
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len());
            let changed = key.as_ref() != Some(&(session.clone(), len));
            if changed {
                cached = match (self.snapshot_of(session.clone(), path.clone()), path) {
                    ((Some((id, _)), runs), Some(p)) if p.is_file() => Some((id, runs)),
                    _ => None,
                };
                key = Some((session, len));
            }
            let running = cached.as_ref().is_some_and(|(_, runs)| {
                open_run(runs).is_some_and(|r| r.steps.iter().any(|v| v.mark == Mark::Running))
            });
            // a terminal resized while nothing happens still gets an image of its size
            let size = term::size();
            if changed || running || last_size != Some(size) {
                let image = match &cached {
                    Some((id, runs)) => watch::frame(&self.project, runs, id, tick, size),
                    None => format!("{NO_ACTIVITY}\n"),
                };
                if let Some(image) = to_write(last.as_deref(), &image) {
                    let mut buf = String::from(if first { "\x1b[2J\x1b[H" } else { "\x1b[H" });
                    for line in image.lines() {
                        let painted = paint_image_line(line);
                        match colour {
                            true => buf += &painted,
                            false => buf += &anstream::adapter::strip_str(&painted).to_string(),
                        }
                        buf += "\x1b[K\n";
                    }
                    buf += "\x1b[J";
                    let mut out = std::io::stdout().lock();
                    out.write_all(buf.as_bytes())?;
                    out.flush()?;
                    first = false;
                    last = Some(image.to_string());
                }
                last_size = Some(size);
                if running {
                    tick += 1;
                }
            }
            std::thread::sleep(PACE);
        }
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
    for line in text.lines() {
        anstream::println!("{}", paint_line(line));
    }
}

/// One line of `render`, in the style of what it starts with.
fn paint_line(line: &str) -> String {
    use crate::cli::style;
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
    format!("{s}{line}{s:#}")
}

const MARKS: [Mark; 7] = [
    Mark::Done,
    Mark::Running,
    Mark::Pending,
    Mark::Unsure,
    Mark::Unexpected,
    Mark::Relay,
    Mark::NotRun,
];

/// A line of the animated image: a line with a box (`[`, a sign, a space) gets its marks painted, any other
/// line is painted as `list` paints it.
fn paint_image_line(line: &str) -> String {
    let box_at = |(i, _): (usize, &str)| {
        MARKS.iter().any(|m| {
            line[i + 1..]
                .strip_prefix(m.sign())
                .is_some_and(|rest| rest.starts_with(' '))
        })
    };
    if line.match_indices('[').any(box_at) {
        paint_boxes(line)
    } else {
        paint_line(line)
    }
}

/// The sign right after each `[` of a line of boxes, in the style of its mark.
fn paint_boxes(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find('[') {
        let (head, tail) = rest.split_at(at + 1);
        out += head;
        rest = tail;
        if let Some(m) = MARKS.iter().find(|m| rest.starts_with(m.sign())) {
            let sign = m.sign();
            out += &painted(&[(Piece::Mark(m.clone()), sign.to_string())]);
            rest = &rest[sign.len()..];
        }
    }
    out + rest
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
    let Some(runs) = line_runs(flow, session, false) else {
        return String::new();
    };
    painted(&crate::flow_status::line::line(&runs))
}

/// `armadai __claude-statusline`: the line of the session Claude Code names in
/// its JSON, always coloured; nothing (0 bytes) when there is nothing to show.
pub fn statusline_main() -> anyhow::Result<()> {
    anstream::ColorChoice::write_global(anstream::ColorChoice::Always);
    if let Some(runs) = line_runs(None, None, true) {
        let pieces = crate::flow_status::line::line(&runs);
        if !pieces.is_empty() {
            anstream::println!("{}", painted(&pieces));
        }
    }
    Ok(())
}

/// `armadai __claude-subagent-statusline`: one JSON line per subagent row
/// that holds a step of a run in progress; nothing, and exit 0, otherwise.
pub fn subagent_statusline_main() -> anyhow::Result<()> {
    anstream::ColorChoice::write_global(anstream::ColorChoice::Always);
    let json = stdin_json();
    let tasks = json
        .as_ref()
        .map(crate::claude_adapter::subagent_status::tasks)
        .unwrap_or_default();
    if tasks.is_empty() {
        return Ok(());
    }
    if let Some((project, runs)) = session_runs(json.as_ref(), None, None, false) {
        for row in crate::claude_adapter::subagent_status::rows(&project, &runs, &tasks) {
            println!("{row}");
        }
    }
    Ok(())
}

/// The pieces joined by single spaces, each wrapped in its style (the
/// printing macros keep or strip the escapes as `--color` says).
pub(crate) fn painted(pieces: &[(Piece, String)]) -> String {
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
/// `strict` (the `__claude-statusline` way): only the JSON's `session_id`, never the latest log.
fn line_runs(flow: Option<String>, session: Option<String>, strict: bool) -> Option<Vec<Run>> {
    Some(session_runs(stdin_json().as_ref(), flow, session, strict)?.1)
}

/// `line_runs` on an already read JSON, with the project it resolved.
fn session_runs(
    json: Option<&serde_json::Value>,
    flow: Option<String>,
    session: Option<String>,
    strict: bool,
) -> Option<(armadai_model::Project, Vec<Run>)> {
    let field = |k: &str| json?.get(k)?.as_str().map(str::to_string);
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
    if strict && session.is_none() {
        return None;
    }
    if let Some(id) = &session {
        log_path(&root, id)?;
    }
    let view = View {
        root,
        project,
        flow,
        session,
    };
    let runs = view.snapshot().1;
    Some((view.project, runs))
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
    fn the_mark_after_a_box_bracket_is_painted() {
        assert_eq!(
            paint_boxes("  [✓ a (x)  ]   [● b (y) |]"),
            format!(
                "  [{} a (x)  ]   [{} b (y) |]",
                paint(style::ok(), "✓"),
                paint(style::running(), "●")
            )
        );
        // a sign that is not right after a `[` (here in a label) stays plain
        assert_eq!(
            paint_boxes("[○ c ✓  ]"),
            format!("[{} c ✓  ]", paint(style::muted(), "○"))
        );
    }

    #[test]
    fn a_box_after_an_edge_is_painted() {
        assert_eq!(
            paint_image_line("  │   [● c (z) |]"),
            format!("  │   [{} c (z) |]", paint(style::running(), "●"))
        );
        // a `[` and a sign that are no box (no space after the sign) paint nothing
        assert_eq!(paint_image_line("  see [✓x]"), paint_line("  see [✓x]"));
        // a step line is a list line: painted by its first sign, as `list` does
        assert_eq!(
            paint_image_line("  ● guard     running"),
            paint_line("  ● guard     running")
        );
        assert_eq!(
            paint_image_line("  ● guard     running"),
            paint(style::running(), "  ● guard     running")
        );
    }

    #[test]
    fn an_identical_image_is_not_written() {
        assert_eq!(to_write(None, "a\n"), Some("a\n"));
        assert_eq!(to_write(Some("a\n"), "a\n"), None);
        assert_eq!(to_write(Some("a\n"), "b\n"), Some("b\n"));
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
