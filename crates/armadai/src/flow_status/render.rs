//! The text of `armadai flow status`: plain, without colour (the CLI adds it
//! line by line, see `cli::flow`).

use std::path::{Path, PathBuf};

use super::record::{LOGS_DIR, log_path};
use super::runs::{Mark, Run, StepView};
use super::stats::{By, Stats};

/// The runs of one session: a `session <id>` heading, then each run (finished
/// and interrupted ones first, each group in start order) with one line per step, its loops
/// and its timed transitions.
pub fn render(runs: &[Run], session: &str) -> String {
    let mut out = format!("session {session} (UTC)\n");
    if runs.is_empty() {
        out += "\nno run of a declared flow in this session yet\n";
    }
    for run in ordered(runs) {
        out += &format!("\n{}\n{}", run_heading(run), run_body(run));
    }
    out
}

/// `{flow}  {status word}`.
pub(super) fn run_heading(run: &Run) -> String {
    match &run.lot {
        Some(lot) => format!("{} #{lot}  {}", run.flow, status_word(run)),
        None => format!("{}  {}", run.flow, status_word(run)),
    }
}

/// The step lines, the links taken more than once and the timed transitions,
/// each line ending in `\n`.
pub(super) fn run_body(run: &Run) -> String {
    let mut out = String::new();
    let w = run.steps.iter().map(|v| v.step.len()).max().unwrap_or(0);
    for v in &run.steps {
        out += format!("  {} {:<w$}  {}", v.mark.sign(), v.step, status_text(v)).trim_end();
        out.push('\n');
    }
    for (from, to, n) in &run.loops {
        out += &format!("  {from} → {to} : {n}\n");
    }
    for (ts, from, to, state) in &run.transitions {
        let state = state
            .as_ref()
            .map(|s| format!("  ({s})"))
            .unwrap_or_default();
        out += &format!("  {}  {from} → {to}{state}\n", utc(*ts, "%H:%M:%S"));
    }
    out
}

/// Finished and interrupted runs first, each group in start order.
pub(super) fn ordered(runs: &[Run]) -> Vec<&Run> {
    let mut ordered: Vec<&Run> = runs.iter().collect();
    ordered.sort_by_key(|r| !(r.finished || r.interrupted));
    ordered
}

fn status_word(run: &Run) -> &'static str {
    if run.finished {
        "finished"
    } else if run.interrupted {
        "interrupted"
    } else {
        "in progress"
    }
}

/// The state (or `running`/`pending`) of a step, then its note in parentheses.
fn status_text(v: &StepView) -> String {
    let word = match v.mark {
        Mark::Running => Some("running"),
        Mark::Pending => Some("pending"),
        _ => None,
    };
    (v.state.as_deref().or(word).map(str::to_string))
        .into_iter()
        .chain(v.reference.as_ref().map(|r| format!("[{r}]")))
        .chain(v.note.as_ref().map(|n| format!("({n})")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A table cell: a `|` written by an agent (a state, say) must not end the cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// `armadai flow status --format md`: the same runs as a Markdown snapshot, to
/// paste in a PR or an issue. Never coloured.
pub fn render_md(runs: &[Run], session: &str, last_event_at: Option<u64>) -> String {
    let mut out = format!("# Flow status: session {session}\n");
    if runs.is_empty() {
        return out + "\nNo run of a declared flow in this session yet.\n";
    }
    if let Some(ts) = last_event_at {
        out += &format!(
            "\nSnapshot at the last recorded event, {} UTC.\n",
            utc(ts, "%Y-%m-%d %H:%M:%S")
        );
    }
    for run in ordered(runs) {
        let lot = run
            .lot
            .as_ref()
            .map(|l| format!(" #{l}"))
            .unwrap_or_default();
        out += &format!("\n## {}{lot}: {}\n\n", run.flow, status_word(run));
        out += "| Step | Mark | Status |\n|---|---|---|\n";
        for v in &run.steps {
            let (step, status) = (cell(&v.step), cell(&status_text(v)));
            out += &format!("| {step} | {} | {status} |\n", v.mark.sign());
        }
        if !run.loops.is_empty() {
            let links: Vec<String> = (run.loops.iter())
                .map(|(from, to, n)| format!("{from} → {to} ({n})"))
                .collect();
            out += &format!("\nLinks taken more than once: {}.\n", links.join(", "));
        }
        if !run.transitions.is_empty() {
            out += "\n| Time (UTC) | From | To | State |\n|---|---|---|---|\n";
            let mut day = None;
            for (ts, from, to, state) in &run.transitions {
                let (from, to) = (cell(from), cell(to));
                let state = cell(state.as_deref().unwrap_or(""));
                // the date shows on the first row and whenever the UTC day changes
                let today = ts / 86_400;
                let fmt = if day == Some(today) {
                    "%H:%M:%S"
                } else {
                    "%Y-%m-%d %H:%M:%S"
                };
                day = Some(today);
                out += &format!("| {} | {from} | {to} | {state} |\n", utc(*ts, fmt));
            }
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

/// `4m12s`, `45s`, `1h03m`.
pub fn duration(s: u64) -> String {
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
    }
}

/// `armadai flow stats`: a header, one row per group, then the notes (the
/// calls left out of `--by step|lot`, and how durations are measured).
pub fn render_stats(stats: &Stats, scope: &str) -> String {
    let by = match stats.by {
        By::Agent => "agent",
        By::Step => "step",
        By::Lot => "lot",
    };
    let mut rows = vec![[
        by.to_uppercase(),
        "CALLS".into(),
        "RESUMED".into(),
        "MEDIAN".into(),
        "MAX".into(),
        "TOTAL".into(),
        "STATES".into(),
        "REJECTED".into(),
        "LOOPS".into(),
    ]];
    for g in &stats.groups {
        let states = (g.states.iter())
            .map(|(w, n)| format!("{w} {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push([
            g.key.clone(),
            g.calls.to_string(),
            g.resumed.to_string(),
            duration(g.median_s),
            duration(g.max_s),
            duration(g.total_s),
            if states.is_empty() {
                "-".into()
            } else {
                states
            },
            (g.rejection_rate).map_or("-".into(), |r| format!("{}%", (r * 100.0).round())),
            g.loops.to_string(),
        ]);
    }
    let width: Vec<usize> = (0..9)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    // counts line up on their right edge, the rest on their left
    let right = [1, 2];
    let mut out = format!("flow stats  {scope}  by {by}\n");
    for r in &rows {
        let cells: Vec<String> = (0..9)
            .map(|c| match right.contains(&c) {
                true => format!("{:>w$}", r[c], w = width[c]),
                false => format!("{:<w$}", r[c], w = width[c]),
            })
            .collect();
        out += cells.join("  ").trim_end();
        out.push('\n');
    }
    out += "durations run from a start to the first stop of the same agent: a resumed agent counts once, so they are lower bounds\n";
    if stats.unmarked_calls > 0 && stats.by != By::Agent {
        let n = stats.unmarked_calls;
        let (calls, are) = if n == 1 {
            ("call", "is")
        } else {
            ("calls", "are")
        };
        out += &format!(
            "{n} {calls} {are} not marked and {} left out of --by {by}\n",
            if n == 1 { "is" } else { "are" }
        );
    }
    out
}

/// The one line of `--by step|lot` when no call of the session is marked.
pub const NO_MARKER: &str = "no call is marked: --by step and --by lot need [flow/step#lot] markers (see armadai flow status)";

#[cfg(test)]
mod tests {
    use super::super::runs::StepView;
    use super::*;

    #[test]
    fn a_rejection_rate_rounds_to_the_nearest_percent_and_four_decimals_in_json() {
        let g = |rate| super::super::stats::Group {
            key: "qa".into(),
            calls: 1,
            resumed: 0,
            median_s: 1,
            max_s: 1,
            total_s: 1,
            states: Default::default(),
            rejection_rate: Some(rate),
            loops: 0,
        };
        let stats = Stats {
            by: By::Agent,
            groups: vec![g(2.0 / 3.0), g(0.2727)],
            unmarked_calls: 0,
            any_marked: false,
        };
        let text = render_stats(&stats, "session s");
        let rows: Vec<&str> = text.lines().skip(2).take(2).collect();
        assert_eq!(
            rows,
            [
                "qa         1        0  1s      1s   1s     -       67%       0",
                "qa         1        0  1s      1s   1s     -       27%       0"
            ]
        );
        let doc = super::super::json::stats_document(&[], &stats);
        assert_eq!(doc["groups"][0]["rejection_rate"], 0.6667);
    }

    #[test]
    fn durations_read_as_seconds_minutes_or_hours() {
        let got: Vec<String> = [0, 45, 59, 60, 252, 3599, 3600, 3725, 7200 + 59 * 60 + 59]
            .map(duration)
            .into();
        assert_eq!(
            got,
            [
                "0s", "45s", "59s", "1m00s", "4m12s", "59m59s", "1h00m", "1h02m", "2h59m"
            ]
        );
    }

    fn view(step: &str, mark: Mark, state: Option<&str>, note: Option<&str>) -> StepView {
        StepView {
            step: step.into(),
            mark,
            state: state.map(str::to_string),
            note: note.map(str::to_string),
            reference: None,
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
                t(T0 + 50, "build", "guard", Some("done")),
            ],
            finished: false,
            interrupted: false,
            lot: None,
        }
    }

    fn finished() -> Run {
        Run {
            flow: "hotfix".into(),
            steps: vec![
                view("triage", Mark::Done, Some("done"), None),
                view("check", Mark::Done, Some("approved"), None),
            ],
            loops: vec![],
            transitions: vec![t(T0 + 5, "triage", "check", Some("done"))],
            finished: true,
            interrupted: false,
            lot: None,
        }
    }

    /// What `runs` gives for `?`, `✗` and a coordinator's step: never finished.
    fn uncertain() -> Run {
        Run {
            flow: "review".into(),
            steps: vec![
                view(
                    "frame",
                    Mark::Relay,
                    None,
                    Some("coordinator, not recorded"),
                ),
                view("check", Mark::Unsure, None, Some("no STATE line")),
                view(
                    "verify",
                    Mark::Unexpected,
                    Some("scope_changed"),
                    Some("undeclared state"),
                ),
            ],
            loops: vec![],
            transitions: vec![],
            finished: false,
            interrupted: false,
            lot: None,
        }
    }

    #[test]
    fn md_is_a_snapshot_with_tables_and_no_escape() {
        let want = "# Flow status: session s1\n\
             \n\
             Snapshot at the last recorded event, 2026-10-22 09:15:50 UTC.\n\
             \n\
             ## hotfix: finished\n\
             \n\
             | Step | Mark | Status |\n\
             |---|---|---|\n\
             | triage | ✓ | done |\n\
             | check | ✓ | approved |\n\
             \n\
             | Time (UTC) | From | To | State |\n\
             |---|---|---|---|\n\
             | 2026-10-22 09:15:05 | triage | check | done |\n\
             \n\
             ## deliver: in progress\n\
             \n\
             | Step | Mark | Status |\n\
             |---|---|---|\n\
             | design | ✓ | done |\n\
             | build | ✓ | done |\n\
             | guard | ● | running |\n\
             | validate | ○ | pending |\n\
             \n\
             Links taken more than once: build → guard (2).\n\
             \n\
             | Time (UTC) | From | To | State |\n\
             |---|---|---|---|\n\
             | 2026-10-22 09:15:20 | design | build | done |\n\
             | 09:15:30 | build | guard | done |\n\
             | 09:15:40 | guard | build | rejected |\n\
             | 09:15:50 | build | guard | done |\n";
        let got = render_md(&[looping(), finished()], "s1", Some(T0 + 50));
        assert_eq!(got, want);
        assert!(!got.contains('\x1b'));
    }

    #[test]
    fn md_status_column_carries_the_note() {
        let got = render_md(&[uncertain()], "s", None);
        assert!(
            got.contains("| check | ? | (no STATE line) |\n")
                && got.contains("| verify | ✗ | scope_changed (undeclared state) |\n"),
            "{got}"
        );
    }

    #[test]
    fn md_escapes_a_pipe_in_a_cell_and_text_does_not() {
        let mut run = uncertain();
        run.steps[2] = view("verify", Mark::Unexpected, Some("a|b"), Some("x|y"));
        run.transitions = vec![t(T0, "frame", "verify", Some("a|b"))];
        let got = render_md(&[run.clone()], "s", None);
        assert!(
            got.contains("| verify | ✗ | a\\|b (x\\|y) |\n")
                && got.contains("| 2026-10-22 09:15:00 | frame | verify | a\\|b |\n"),
            "{got}"
        );
        assert!(render(&[run], "s").contains("a|b (x|y)"));
    }

    #[test]
    fn md_shows_the_date_on_the_first_row_and_when_the_day_changes() {
        const DAY: u64 = 86_400;
        let mut run = finished();
        run.transitions = vec![
            t(T0, "a", "b", None),
            t(T0 + 60, "b", "c", None),
            t(T0 + DAY, "c", "d", None),
            t(T0 + DAY + 60, "d", "e", None),
        ];
        let got = render_md(&[run], "s", None);
        let rows: Vec<&str> = got
            .lines()
            .filter(|l| {
                l.contains(" | a ")
                    || l.contains(" | b ")
                    || l.contains(" | c ")
                    || l.contains(" | d ")
            })
            .collect();
        assert_eq!(
            rows,
            [
                "| 2026-10-22 09:15:00 | a | b |  |",
                "| 09:16:00 | b | c |  |",
                "| 2026-10-23 09:15:00 | c | d |  |",
                "| 09:16:00 | d | e |  |",
            ]
        );
    }

    #[test]
    fn md_of_a_session_without_run_says_so() {
        assert_eq!(
            render_md(&[], "s1", Some(T0)),
            "# Flow status: session s1\n\nNo run of a declared flow in this session yet.\n"
        );
    }

    #[test]
    fn uncertain_and_coordinator_steps_say_why() {
        assert_eq!(
            render(&[uncertain()], "s"),
            "session s (UTC)\n\
             \n\
             review  in progress\n\
             \x20 ~ frame   (coordinator, not recorded)\n\
             \x20 ? check   (no STATE line)\n\
             \x20 ✗ verify  scope_changed (undeclared state)\n"
        );
    }

    #[test]
    fn a_step_not_run_in_this_session_says_so() {
        let run = Run {
            flow: "deliver | guide".into(),
            steps: vec![
                view(
                    "refine",
                    Mark::NotRun,
                    None,
                    Some("not run in this session"),
                ),
                view("frame", Mark::Running, None, None),
                view("mockup", Mark::Pending, None, None),
            ],
            loops: vec![],
            transitions: vec![],
            finished: false,
            interrupted: false,
            lot: None,
        };
        assert_eq!(
            render(&[run], "s"),
            "session s (UTC)\n\
             \n\
             deliver | guide  in progress\n\
             \x20 - refine  (not run in this session)\n\
             \x20 ● frame   running\n\
             \x20 ○ mockup  pending\n"
        );
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
             \x20 build → guard : 2\n\
             \x20 09:15:20  design → build  (done)\n\
             \x20 09:15:30  build → guard  (done)\n\
             \x20 09:15:40  guard → build  (rejected)\n\
             \x20 09:15:50  build → guard  (done)\n"
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
             \x20 ✓ check   approved\n\
             \x20 09:15:05  triage → check  (done)\n\
             \n\
             deliver  in progress\n\
             \x20 ✓ design    done\n\
             \x20 ✓ build     done\n\
             \x20 ● guard     running\n\
             \x20 ○ validate  pending\n\
             \x20 build → guard : 2\n\
             \x20 09:15:20  design → build  (done)\n\
             \x20 09:15:30  build → guard  (done)\n\
             \x20 09:15:40  guard → build  (rejected)\n\
             \x20 09:15:50  build → guard  (done)\n"
        );
    }

    #[test]
    fn a_step_with_nothing_to_say_leaves_no_trailing_space() {
        let run = Run {
            flow: "f".into(),
            steps: vec![
                view("a", Mark::Done, None, None),
                view("long-step", Mark::Pending, None, None),
            ],
            loops: vec![],
            transitions: vec![],
            finished: false,
            interrupted: false,
            lot: None,
        };
        assert_eq!(
            render(&[run], "s"),
            "session s (UTC)\n\nf  in progress\n  ✓ a\n  ○ long-step  pending\n"
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

    #[test]
    fn an_interrupted_run_says_so_and_comes_before_the_running_one() {
        let interrupted = Run {
            flow: "bugfix".into(),
            steps: vec![
                view("analyse", Mark::Done, Some("done"), None),
                view("test", Mark::Running, None, None),
            ],
            loops: vec![],
            transitions: vec![t(T0 + 20, "analyse", "test", Some("done"))],
            finished: false,
            interrupted: true,
            lot: None,
        };
        // started in this order: running, interrupted, finished
        assert_eq!(
            render(&[looping(), interrupted, finished()], "s"),
            "session s (UTC)\n\
             \n\
             bugfix  interrupted\n\
             \x20 ✓ analyse  done\n\
             \x20 ● test     running\n\
             \x20 09:15:20  analyse → test  (done)\n\
             \n\
             hotfix  finished\n\
             \x20 ✓ triage  done\n\
             \x20 ✓ check   approved\n\
             \x20 09:15:05  triage → check  (done)\n\
             \n\
             deliver  in progress\n\
             \x20 ✓ design    done\n\
             \x20 ✓ build     done\n\
             \x20 ● guard     running\n\
             \x20 ○ validate  pending\n\
             \x20 build → guard : 2\n\
             \x20 09:15:20  design → build  (done)\n\
             \x20 09:15:30  build → guard  (done)\n\
             \x20 09:15:40  guard → build  (rejected)\n\
             \x20 09:15:50  build → guard  (done)\n"
        );
    }

    /// T4 and T5: the lot follows the flow, the ref follows the state.
    #[test]
    fn a_lot_and_a_ref_show_in_text_and_markdown() {
        let run = Run {
            lot: Some("menu".into()),
            steps: vec![
                StepView {
                    reference: Some("feature/menu".into()),
                    ..view("triage", Mark::Done, Some("approved"), Some("n"))
                },
                view("check", Mark::Running, None, None),
            ],
            loops: vec![],
            transitions: vec![],
            ..finished()
        };
        assert_eq!(
            render(std::slice::from_ref(&run), "s"),
            "session s (UTC)\n\n\
             hotfix #menu  finished\n\
             \x20 ✓ triage  approved [feature/menu] (n)\n\
             \x20 ● check   running\n"
        );
        let md = render_md(std::slice::from_ref(&run), "s", None);
        assert_eq!(
            md,
            "# Flow status: session s\n\n\
             ## hotfix #menu: finished\n\n\
             | Step | Mark | Status |\n|---|---|---|\n\
             | triage | ✓ | approved [feature/menu] (n) |\n\
             | check | ● | running |\n"
        );
    }
}
