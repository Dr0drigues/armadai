//! The one-line summary of a session for a status bar (`flow status --line`):
//! pure, the CLI adds the colour piece by piece.

use super::runs::{Mark, Run};

/// What a piece of the line is, so the CLI can colour it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Flow,
    Mark(Mark),
    Progress,
    Loops,
    Status,
}

/// The pieces of the line (empty when there is no run). The run shown is the
/// last one still open, else the last one.
pub fn line(runs: &[Run]) -> Vec<(Piece, String)> {
    let Some(run) = runs
        .iter()
        .rfind(|r| !r.finished && !r.interrupted)
        .or(runs.last())
    else {
        return Vec::new();
    };
    let mut out = vec![(Piece::Flow, run.flow.clone())];
    if run.finished || run.interrupted {
        let word = if run.finished {
            "finished"
        } else {
            "interrupted"
        };
        out.push((Piece::Status, word.into()));
    } else if let Some(v) = current(run) {
        out.push((Piece::Mark(v.mark.clone()), v.mark.sign().into()));
        out.push((Piece::Mark(v.mark.clone()), v.step.clone()));
    }
    let n = run
        .steps
        .iter()
        .filter(|v| !matches!(v.mark, Mark::Relay | Mark::NotRun))
        .count();
    let d = run.steps.iter().filter(|v| v.mark == Mark::Done).count();
    out.push((Piece::Progress, format!("{d}/{n}")));
    let k: u32 = run
        .loops
        .iter()
        .map(|(_, _, times)| times.saturating_sub(1))
        .sum();
    if k > 0 {
        out.push((Piece::Loops, format!("(loops {k})")));
    }
    out
}

/// The step to name: the first running one, else the first `?`/`✗`, else the
/// first pending one.
fn current(run: &Run) -> Option<&super::runs::StepView> {
    let find = |f: fn(&Mark) -> bool| run.steps.iter().find(|v| f(&v.mark));
    find(|m| *m == Mark::Running)
        .or_else(|| find(|m| matches!(m, Mark::Unsure | Mark::Unexpected)))
        .or_else(|| find(|m| *m == Mark::Pending))
}

/// The plain text: the pieces joined by single spaces (the CLI paints them instead).
#[cfg(test)]
pub fn text(pieces: &[(Piece, String)]) -> String {
    pieces
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::super::runs::StepView;
    use super::*;

    fn v(step: &str, mark: Mark) -> StepView {
        StepView {
            step: step.into(),
            mark,
            state: None,
            note: None,
        }
    }

    fn run(flow: &str, steps: Vec<StepView>, loops: Vec<(&str, &str, u32)>) -> Run {
        Run {
            flow: flow.into(),
            steps,
            loops: loops
                .into_iter()
                .map(|(a, b, n)| (a.into(), b.into(), n))
                .collect(),
            transitions: vec![],
            finished: false,
            interrupted: false,
        }
    }

    fn shown(runs: &[Run]) -> String {
        text(&line(runs))
    }

    #[test]
    fn a_running_step_with_a_loop() {
        let r = run(
            "deliver",
            vec![
                v("design", Mark::Done),
                v("build", Mark::Done),
                v("guard", Mark::Running),
                v("validate", Mark::Pending),
            ],
            vec![("build", "guard", 2)],
        );
        assert_eq!(shown(&[r]), "deliver ● guard 2/4 (loops 1)");
    }

    #[test]
    fn loops_count_the_extra_times_of_every_link() {
        let r = run(
            "d",
            vec![v("a", Mark::Running)],
            vec![("a", "b", 3), ("b", "a", 2)],
        );
        assert_eq!(shown(&[r]), "d ● a 0/1 (loops 3)");
    }

    #[test]
    fn a_step_to_check_is_named_before_a_pending_one() {
        let r = run(
            "review",
            vec![
                v("frame", Mark::Relay),
                v("check", Mark::Unsure),
                v("ship", Mark::Pending),
            ],
            vec![],
        );
        assert_eq!(shown(&[r]), "review ? check 0/2");
    }

    #[test]
    fn unsure_comes_before_pending_even_when_pending_is_first() {
        let r = run(
            "r",
            vec![v("a", Mark::Pending), v("b", Mark::Unexpected)],
            vec![],
        );
        assert_eq!(shown(&[r]), "r ✗ b 0/2");
    }

    #[test]
    fn nothing_to_name_leaves_the_progress_alone() {
        let r = run(
            "deliver",
            vec![
                v("a", Mark::Done),
                v("b", Mark::Done),
                v("c", Mark::Done),
                v("d", Mark::NotRun),
                v("e", Mark::Relay),
                v("f", Mark::Pending),
            ],
            vec![],
        );
        // `f` is pending so it is named; without it the line has no step
        assert_eq!(shown(std::slice::from_ref(&r)), "deliver ○ f 3/4");
        let mut r = r;
        r.steps.pop();
        assert_eq!(shown(&[r]), "deliver 3/3");
    }

    #[test]
    fn a_finished_and_an_interrupted_run_say_so() {
        let mut f = run(
            "hotfix",
            vec![v("a", Mark::Done), v("b", Mark::Done)],
            vec![],
        );
        f.finished = true;
        assert_eq!(shown(&[f]), "hotfix finished 2/2");
        let mut i = run(
            "bugfix",
            vec![v("a", Mark::Done), v("b", Mark::Pending)],
            vec![],
        );
        i.interrupted = true;
        assert_eq!(shown(&[i]), "bugfix interrupted 1/2");
    }

    #[test]
    fn the_last_open_run_is_shown_not_the_first() {
        let first = run("one", vec![v("a", Mark::Running)], vec![]);
        let second = run("two", vec![v("b", Mark::Running)], vec![]);
        let mut done = run("three", vec![v("c", Mark::Done)], vec![]);
        done.finished = true;
        assert_eq!(shown(&[first, second, done]), "two ● b 0/1");
    }

    #[test]
    fn without_an_open_run_the_last_run_is_shown() {
        let mut a = run("a", vec![v("x", Mark::Done)], vec![]);
        a.finished = true;
        let mut b = run("b", vec![v("y", Mark::Done)], vec![]);
        b.interrupted = true;
        assert_eq!(shown(&[a, b]), "b interrupted 1/1");
    }

    #[test]
    fn no_run_is_an_empty_line() {
        assert_eq!(shown(&[]), "");
    }

    #[test]
    fn pieces_carry_the_mark_for_the_colour() {
        let r = run("d", vec![v("a", Mark::Running)], vec![("a", "a", 2)]);
        let kinds: Vec<Piece> = line(&[r]).into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            kinds,
            vec![
                Piece::Flow,
                Piece::Mark(Mark::Running),
                Piece::Mark(Mark::Running),
                Piece::Progress,
                Piece::Loops
            ]
        );
    }
}
