//! The text of `armadai flow status`: plain, without colour (the CLI adds it
//! line by line, see `cli::flow`).

use std::path::{Path, PathBuf};

use super::record::{LOGS_DIR, log_path};
use super::runs::{Mark, Run};

/// The runs of one session: a `session <id>` heading, then each run (finished
/// ones first, each group in start order) with one line per step, its loops
/// and its timed transitions.
pub fn render(runs: &[Run], session: &str) -> String {
    let mut out = format!("session {session} (UTC)\n");
    if runs.is_empty() {
        out += "\nno run of a declared flow in this session yet\n";
    }
    let mut ordered: Vec<&Run> = runs.iter().collect();
    ordered.sort_by_key(|r| !r.finished); // stable: start order kept
    for run in ordered {
        let status = if run.finished {
            "finished"
        } else {
            "in progress"
        };
        out += &format!("\n{}  {status}\n", run.flow);
        let w = run.steps.iter().map(|v| v.step.len()).max().unwrap_or(0);
        for v in &run.steps {
            let (sign, word) = match v.mark {
                Mark::Done => ("✓", None),
                Mark::Running => ("●", Some("running")),
                Mark::Pending => ("○", Some("pending")),
                Mark::Unsure => ("?", None),
                Mark::Unexpected => ("✗", None),
            };
            let what = (v.state.as_deref().or(word).map(str::to_string))
                .into_iter()
                .chain(v.note.as_ref().map(|n| format!("({n})")))
                .collect::<Vec<_>>()
                .join(" ");
            out += format!("  {sign} {:<w$}  {what}", v.step).trim_end();
            out.push('\n');
        }
        for (from, to, n) in &run.loops {
            out += &format!("  {from} ⇄ {to} : {n}\n");
        }
        for (ts, from, to, state) in &run.transitions {
            let state = state
                .as_ref()
                .map(|s| format!("  ({s})"))
                .unwrap_or_default();
            out += &format!("  {}  {from} → {to}{state}\n", utc(*ts, "%H:%M:%S"));
        }
    }
    out
}

fn utc(ts: u64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|d| d.format(fmt).to_string())
        .unwrap_or_else(|| "?".into())
}

fn logs_dir(root: &Path) -> PathBuf {
    root.join(LOGS_DIR)
}

/// Every recorded session as `(id, last modified in unix seconds)`, newest first.
pub fn sessions(root: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = std::fs::read_dir(logs_dir(root)) else {
        return Vec::new();
    };
    let mut found: Vec<(String, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path
                .file_name()?
                .to_str()?
                .strip_suffix(".jsonl")?
                .to_string();
            log_path(root, &id)?; // a name `--session` could not reach is not listed
            let mtime = e.metadata().ok()?.modified().ok()?;
            let secs = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some((id, secs))
        })
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    found
}

/// The session whose log was modified last.
pub fn latest_session(root: &Path) -> Option<String> {
    sessions(root).into_iter().next().map(|(id, _)| id)
}

/// `armadai flow status --list`: one row per `(id, last modified, runs)`.
pub fn render_sessions(sessions: &[(String, u64, usize)]) -> String {
    let w = (sessions.iter().map(|s| s.0.len()))
        .chain(["SESSION".len()])
        .max()
        .unwrap_or(0);
    let mut out = format!("{:<w$}  LAST EVENT (UTC)     RUNS\n", "SESSION");
    for (id, at, runs) in sessions {
        out += &format!("{id:<w$}  {}  {runs}\n", utc(*at, "%Y-%m-%d %H:%M:%S"));
    }
    out
}

/// `--watch` draws again only when the log's length (`None`: no file) changed.
pub fn should_redraw(old: Option<u64>, new: Option<u64>) -> bool {
    old != new
}

#[cfg(test)]
mod tests {
    use super::super::runs::StepView;
    use super::*;

    fn view(step: &str, mark: Mark, state: Option<&str>, note: Option<&str>) -> StepView {
        StepView {
            step: step.into(),
            mark,
            state: state.map(str::to_string),
            note: note.map(str::to_string),
        }
    }

    fn t(
        ts: u64,
        from: &str,
        to: &str,
        state: Option<&str>,
    ) -> (u64, String, String, Option<String>) {
        (ts, from.into(), to.into(), state.map(str::to_string))
    }

    /// 2026-10-22 09:15:00 UTC
    const T0: u64 = 1_792_660_500;

    fn looping() -> Run {
        Run {
            flow: "deliver".into(),
            steps: vec![
                view("design", Mark::Done, Some("done"), None),
                view("build", Mark::Done, Some("done"), None),
                view("guard", Mark::Running, None, None),
                view("validate", Mark::Pending, None, None),
            ],
            loops: vec![("build".into(), "guard".into(), 2)],
            transitions: vec![
                t(T0 + 20, "design", "build", Some("done")),
                t(T0 + 30, "build", "guard", Some("done")),
                t(T0 + 40, "guard", "build", Some("rejected")),
                t(T0 + 50, "build", "guard", None),
            ],
            finished: false,
        }
    }

    fn finished() -> Run {
        Run {
            flow: "hotfix".into(),
            steps: vec![
                view("triage", Mark::Done, Some("done"), None),
                view("check", Mark::Unsure, None, Some("no STATE line")),
                view(
                    "verify",
                    Mark::Unexpected,
                    Some("scope_changed"),
                    Some("undeclared state"),
                ),
            ],
            loops: vec![],
            transitions: vec![t(T0 + 5, "triage", "check", Some("done"))],
            finished: true,
        }
    }

    #[test]
    fn a_running_run_with_a_loop_renders_every_line() {
        assert_eq!(
            render(&[looping()], "abc123"),
            "session abc123 (UTC)\n\
             \n\
             deliver  in progress\n\
             \x20 ✓ design    done\n\
             \x20 ✓ build     done\n\
             \x20 ● guard     running\n\
             \x20 ○ validate  pending\n\
             \x20 build ⇄ guard : 2\n\
             \x20 09:15:20  design → build  (done)\n\
             \x20 09:15:30  build → guard  (done)\n\
             \x20 09:15:40  guard → build  (rejected)\n\
             \x20 09:15:50  build → guard\n"
        );
    }

    #[test]
    fn finished_runs_stay_and_come_before_the_running_one() {
        assert_eq!(
            render(&[looping(), finished()], "s"),
            "session s (UTC)\n\
             \n\
             hotfix  finished\n\
             \x20 ✓ triage  done\n\
             \x20 ? check   (no STATE line)\n\
             \x20 ✗ verify  scope_changed (undeclared state)\n\
             \x20 09:15:05  triage → check  (done)\n\
             \n\
             deliver  in progress\n\
             \x20 ✓ design    done\n\
             \x20 ✓ build     done\n\
             \x20 ● guard     running\n\
             \x20 ○ validate  pending\n\
             \x20 build ⇄ guard : 2\n\
             \x20 09:15:20  design → build  (done)\n\
             \x20 09:15:30  build → guard  (done)\n\
             \x20 09:15:40  guard → build  (rejected)\n\
             \x20 09:15:50  build → guard\n"
        );
    }

    #[test]
    fn a_step_with_nothing_to_say_leaves_no_trailing_space() {
        let run = Run {
            flow: "f".into(),
            steps: vec![
                view("a", Mark::Done, None, None),
                view("long-step", Mark::Unsure, None, None),
            ],
            loops: vec![],
            transitions: vec![],
            finished: true,
        };
        assert_eq!(
            render(&[run], "s"),
            "session s (UTC)\n\nf  finished\n  ✓ a\n  ? long-step\n"
        );
    }

    #[test]
    fn only_names_session_can_reach_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join(LOGS_DIR);
        std::fs::create_dir_all(&logs).unwrap();
        for name in ["ok-1_A.jsonl", "has space.jsonl", "dot.dot.jsonl", ".jsonl"] {
            std::fs::write(logs.join(name), "").unwrap();
        }
        let ids: Vec<String> = sessions(dir.path()).into_iter().map(|s| s.0).collect();
        assert_eq!(ids, ["ok-1_A"]);
    }

    #[test]
    fn a_session_without_a_run_says_so() {
        assert_eq!(
            render(&[], "s"),
            "session s (UTC)\n\nno run of a declared flow in this session yet\n"
        );
    }

    #[test]
    fn sessions_list_their_id_date_and_runs() {
        assert_eq!(
            render_sessions(&[("abc".into(), T0, 2), ("long-session".into(), T0 + 3600, 0)]),
            "SESSION       LAST EVENT (UTC)     RUNS\n\
             abc           2026-10-22 09:15:00  2\n\
             long-session  2026-10-22 10:15:00  0\n"
        );
    }

    #[test]
    fn the_latest_session_is_the_last_modified_log() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(latest_session(dir.path()), None);
        let logs = dir.path().join(LOGS_DIR);
        std::fs::create_dir_all(&logs).unwrap();
        assert_eq!(latest_session(dir.path()), None);
        let at = |name: &str, secs: u64| {
            let f = std::fs::File::create(logs.join(name)).unwrap();
            f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
                .unwrap();
        };
        at("old.jsonl", T0);
        at("new.jsonl", T0 + 10);
        at("mid.jsonl", T0 + 5);
        at("newest.txt", T0 + 20); // not a log
        assert_eq!(latest_session(dir.path()).as_deref(), Some("new"));
    }

    #[test]
    fn watch_redraws_only_when_the_log_changes() {
        assert!(!should_redraw(None, None)); // still no file: keep waiting
        assert!(should_redraw(None, Some(10))); // the first line arrived
        assert!(!should_redraw(Some(10), Some(10)));
        assert!(should_redraw(Some(10), Some(42)));
        assert!(should_redraw(Some(42), None)); // purged
    }
}
