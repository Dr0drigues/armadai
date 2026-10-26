//! `armadai flow status`: where each flow of a Claude Code session stands,
//! from the logs the recording hooks write.
//!
//! The project root is resolved exactly as the hooks resolve it (from a
//! subdirectory or a git worktree), so the logs read are the ones written.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use armadai_model::Project;

use crate::flow_status::record::{FlowEvent, GROUP_SEP, log_path};
use crate::flow_status::render;
use crate::flow_status::runs::runs;

const NO_ACTIVITY: &str =
    "no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)";

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
            run opened at, when the session started mid-flow), the links taken more than once, and the timed \
            transitions (UTC). A run is attributed to the flow whose id is the command you typed \
            (when the recording hook saw it) and shown `interrupted` when a later flow command \
            closed it before it finished. Without --session, the latest session.",
        after_help = "Examples:\n  \
            armadai flow status\n  \
            armadai flow status deliver --watch\n  \
            armadai flow status --list\n  \
            armadai flow status --session 3f2a9c"
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
    },
}

pub fn execute(cmd: FlowCommand) -> anyhow::Result<()> {
    let FlowCommand::Status {
        flow,
        session,
        watch,
        list,
    } = cmd;
    let cwd = std::env::current_dir()?;
    let Some(root) = crate::claude_adapter::policy_gate::resolve_project(&cwd) else {
        // the same advice as every project command (legacy included)
        crate::compile::find_project(&cwd)?;
        bail!("no ArmadAI project found (.armadai/project.yaml)");
    };
    let project = match armadai_model::load_checked(&root) {
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
        if !watch && !path.is_file() {
            bail!("no session `{id}` recorded; see `armadai flow status --list`");
        }
    }
    let view = View {
        root,
        project,
        flow,
        session,
    };
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

    fn text(&self) -> String {
        let (Some(session), Some(path)) = self.log() else {
            return format!("{NO_ACTIVITY}\n");
        };
        if !path.is_file() {
            // --watch on a session not written yet
            return format!("{NO_ACTIVITY}\n");
        }
        let mut found = runs(&self.project, &read_log(&path));
        if let Some(id) = &self.flow {
            found.retain(|r| r.flow.split(GROUP_SEP).any(|f| f == id));
        }
        render::render(&found, &session)
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
