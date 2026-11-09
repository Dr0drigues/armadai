//! The image of `flow status --watch` in a terminal: pure, plain text (no colour,
//! no escape). The running flow is drawn in boxes with the mark of each step and a
//! spinner on the running one; the other runs are counted on one line.

use armadai_model::Project;

use super::line::open_run;
use super::render::{render, run_body, run_heading};
use super::runs::{Mark, Run};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

/// The spinner of the running step: `| / - \`, one per tick.
pub fn spinner(tick: u64) -> char {
    ['|', '/', '-', '\\'][(tick % 4) as usize]
}

/// Whether `text` (the session header included) fits: no line wider than `cols`
/// (in chars), at most `rows - 1` lines (the last row keeps the cursor, so nothing scrolls).
fn fits(text: &str, size: Size) -> bool {
    text.lines().all(|l| l.chars().count() <= size.cols) && text.lines().count() < size.rows
}

/// `+ <n> other runs: armadai flow status`, `run` when n == 1; `None` when n == 0.
fn others_line(n: usize) -> Option<String> {
    let s = if n == 1 { "" } else { "s" };
    (n > 0).then(|| format!("+ {n} other run{s}: armadai flow status\n"))
}

/// `text` cut to `size`: at most `rows - 1` lines, the last one saying what was cut
/// (`+ <n> more lines: armadai flow status`, always plural: it replaces a line too), and every line at most `cols` chars,
/// a cut one ending in `…`. With `rows <= 2` there is no room for the saying line.
fn cut(text: &str, size: Size) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let room = size.rows.saturating_sub(1);
    let mut kept: Vec<String> = if lines.len() <= room {
        lines.iter().map(|l| l.to_string()).collect()
    } else if room <= 1 {
        lines[..room].iter().map(|l| l.to_string()).collect()
    } else {
        // at least 2: the saying line takes the place of one more
        let left = lines.len() - (room - 1);
        let mut k: Vec<String> = lines[..room - 1].iter().map(|l| l.to_string()).collect();
        k.push(format!("+ {left} more lines: armadai flow status"));
        k
    };
    for l in &mut kept {
        if l.chars().count() > size.cols {
            *l = match size.cols {
                0 => String::new(),
                n => l.chars().take(n - 1).chain(['…']).collect(),
            };
        }
    }
    kept.iter().map(|l| format!("{l}\n")).collect()
}

/// The image of `--watch` in a terminal, the first of three that fits `size`:
/// 1. the last open run of a known flow drawn (heading, drawing, blank line, its
///    block), then one line counting the other runs;
/// 2. the list: the run `--line` names as `render` lists it, then the same count line;
/// 3. that list cut to the screen, its last line saying how much was left out.
pub fn frame(project: &Project, runs: &[Run], session: &str, tick: u64, size: Size) -> String {
    let others = others_line(runs.len().saturating_sub(1)).unwrap_or_default();
    if let Some(run) = open_run(runs)
        && let Some(flow) = project.flows.get(&run.flow)
    {
        let graph = armadai_model::flow_graph(&run.flow, flow);
        let drawing = crate::ascii_graph::flow_to_ascii_marked(&graph, &|id| match run
            .steps
            .iter()
            .find(|v| v.step == id)
        {
            Some(v) => {
                let after = if v.mark == Mark::Running {
                    format!(" {}", spinner(tick))
                } else {
                    "  ".into()
                };
                (format!("{} ", v.mark.sign()), after)
            }
            // shortcut: unreachable today, `runs` makes a view for every step of the flow
            // (`step_order`); kept so a step without a view is drawn, never a panic.
            None => ("  ".into(), "  ".into()),
        });
        // the first line is `flow: <id>`, the heading says it already
        let drawing = drawing.split_once('\n').map_or("", |(_, rest)| rest);
        let out = format!(
            "session {session} (UTC)\n\n{}\n{drawing}\n{}{others}",
            run_heading(run),
            run_body(run)
        );
        if fits(&out, size) {
            return out;
        }
    }
    let list = match open_run(runs).or(runs.last()) {
        Some(run) => format!(
            "session {session} (UTC)\n\n{}\n{}{others}",
            run_heading(run),
            run_body(run)
        ),
        None => render(runs, session),
    };
    if fits(&list, size) {
        list
    } else {
        cut(&list, size)
    }
}

#[cfg(test)]
mod tests {
    use super::super::runs::StepView;
    use super::*;

    const DELIVER: &str = "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  guard: { agent: qa, states: [approved, rejected] }\n  validate: { agent: ops }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: guard }\n  - { from: guard, to: build, when: rejected }\n  - { from: guard, to: validate, when: approved }\n";
    const PBV: &str = "steps:\n  plan: { agent: a }\n  build: { agent: b }\n  check: { agent: c }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: check }\n  - { from: check, to: build, when: failed }\n";
    const FAN_OUT: &str = "steps:\n  a: { agent: a }\n  b: { agent: b }\n  c: { agent: c }\nlinks:\n  - { from: a, to: b }\n  - { from: a, to: c }\n";
    const SEVEN: &str = "steps:\n  a: { agent: a }\n  b: { agent: b }\n  c: { agent: c }\n  d: { agent: d }\n  e: { agent: e }\n  f: { agent: f }\n  g: { agent: g }\nlinks:\n  - { from: a, to: b }\n  - { from: b, to: c }\n  - { from: c, to: d }\n  - { from: d, to: e }\n  - { from: e, to: f }\n  - { from: f, to: g }\n";
    const BIG: Size = Size {
        cols: 200,
        rows: 100,
    };

    fn project(flows: &[(&str, &str)]) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(".armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), "schema_version: 1\n").unwrap();
        for agent in [
            "architect",
            "dev",
            "qa",
            "ops",
            "a",
            "b",
            "c",
            "d",
            "e",
            "f",
            "g",
        ] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        for (id, body) in flows {
            std::fs::write(a.join(format!("flows/{id}.yaml")), body).unwrap();
        }
        armadai_model::load_checked(dir.path()).unwrap().0
    }

    fn run(flow: &str, steps: &[(&str, Mark)]) -> Run {
        Run {
            flow: flow.into(),
            steps: steps
                .iter()
                .map(|(s, m)| StepView {
                    step: (*s).into(),
                    mark: m.clone(),
                    state: None,
                    note: None,
                })
                .collect(),
            loops: vec![],
            transitions: vec![],
            finished: false,
            interrupted: false,
        }
    }

    fn deliver_run() -> Run {
        run(
            "deliver",
            &[
                ("design", Mark::Done),
                ("build", Mark::Done),
                ("guard", Mark::Running),
                ("validate", Mark::Pending),
            ],
        )
    }

    #[test]
    fn spinner_turns() {
        let s: String = (0..5).map(spinner).collect();
        assert_eq!(s, "|/-\\|");
    }

    #[test]
    fn a_running_run_is_drawn_with_its_marks() {
        let p = project(&[("deliver", DELIVER)]);
        let out = frame(&p, &[deliver_run()], "s1", 0, BIG);
        // the boxes and edges of `armadai graph --format ascii deliver`, each label
        // widened by its mark and its cell; a blank line, then the full block of `render`
        assert_eq!(
            out,
            "session s1 (UTC)

deliver  in progress
      [✓ ▶ design (architect)  ]
                  ┌┘
                  ↓
          [✓ build (dev)  ]
                  └┐
                   ↓
           [● guard (qa) |]
                   │
              \"approved\"
                   ↓
        [○ validate (ops) ■  ]

links not drawn:
  guard → build (rejected)

  ✓ design
  ✓ build
  ● guard     running
  ○ validate  pending
"
        );
    }

    /// Two ticks of `run`: same shape, one character apart (`|` then `/`), on the
    /// line of the box of `running`.
    fn assert_ticks_differ_once(p: &Project, run: &Run, running: &str) {
        let (a, b) = (
            frame(p, std::slice::from_ref(run), "s1", 0, BIG),
            frame(p, std::slice::from_ref(run), "s1", 1, BIG),
        );
        assert_eq!(a.lines().count(), b.lines().count());
        let mut diffs = Vec::new();
        for (la, lb) in a.lines().zip(b.lines()) {
            assert_eq!(la.chars().count(), lb.chars().count(), "{la:?} / {lb:?}");
            for (x, y) in la.chars().zip(lb.chars()) {
                if x != y {
                    diffs.push((x, y, la.to_string()));
                }
            }
        }
        assert_eq!(diffs.len(), 1, "{diffs:?}");
        assert_eq!((diffs[0].0, diffs[0].1), ('|', '/'));
        assert!(diffs[0].2.contains(running), "{diffs:?}");
    }

    #[test]
    fn two_ticks_differ_only_by_the_spinner() {
        let p = project(&[("deliver", DELIVER), ("fan-out", FAN_OUT)]);
        assert_ticks_differ_once(&p, &deliver_run(), "guard");
        // two boxes on one line: only the running one turns
        let fan = run(
            "fan-out",
            &[
                ("a", Mark::Done),
                ("b", Mark::Running),
                ("c", Mark::Pending),
            ],
        );
        assert_ticks_differ_once(&p, &fan, "[● b");
    }

    #[test]
    fn every_mark_reads_in_plain_text() {
        let p = project(&[("seven", SEVEN)]);
        let marks = [
            Mark::Done,
            Mark::Running,
            Mark::Pending,
            Mark::Unsure,
            Mark::Unexpected,
            Mark::Relay,
            Mark::NotRun,
        ];
        let steps: Vec<(&str, Mark)> = ["a", "b", "c", "d", "e", "f", "g"]
            .into_iter()
            .zip(marks)
            .collect();
        let out = frame(&p, &[run("seven", &steps)], "s", 0, BIG);
        let boxes: Vec<&str> = out
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('['))
            .collect();
        assert_eq!(
            boxes,
            [
                "[✓ ▶ a  ]",
                "[● b |]",
                "[○ c  ]",
                "[? d  ]",
                "[✗ e  ]",
                "[~ f  ]",
                "[- g ■  ]"
            ]
        );
    }

    fn hotfix_bugfix_deliver() -> [Run; 3] {
        let mut hotfix = run("hotfix", &[("x", Mark::Done)]);
        hotfix.finished = true;
        let mut bugfix = run("bugfix", &[("y", Mark::Pending)]);
        bugfix.interrupted = true;
        [hotfix, bugfix, deliver_run()]
    }

    #[test]
    fn the_other_runs_are_one_count_line() {
        let p = project(&[("deliver", DELIVER)]);
        let out = frame(&p, &hotfix_bugfix_deliver(), "s1", 0, BIG);
        let drawn = frame(&p, &[deliver_run()], "s1", 0, BIG);
        assert_eq!(out, format!("{drawn}+ 2 other runs: armadai flow status\n"));
    }

    #[test]
    fn one_other_run_is_singular_and_none_is_no_line() {
        let p = project(&[("deliver", DELIVER)]);
        let [hotfix, _, deliver] = hotfix_bugfix_deliver();
        let out = frame(&p, &[hotfix, deliver.clone()], "s1", 0, BIG);
        assert_eq!(
            out.lines().last(),
            Some("+ 1 other run: armadai flow status")
        );
        let alone = frame(&p, &[deliver], "s1", 0, BIG);
        assert_eq!(alone.lines().last(), Some("  ○ validate  pending"));
    }

    #[test]
    fn fits_counts_the_count_line() {
        let p = project(&[("deliver", DELIVER)]);
        let runs = hotfix_bugfix_deliver();
        let image = frame(&p, &runs, "s", 0, BIG);
        let rows = image.lines().count(); // the count line included
        let at = |rows| frame(&p, &runs, "s", 0, Size { cols: 200, rows });
        assert_eq!(at(rows + 1), image);
        let list = at(rows);
        assert_ne!(list, image);
        assert!(!list.contains('['), "the drawing is gone: {list}");
    }

    #[test]
    fn a_drawing_too_big_lists_the_followed_run_and_counts_the_others() {
        let p = project(&[("deliver", DELIVER)]);
        let mut hotfix = run("hotfix", &[("x", Mark::Done)]);
        hotfix.finished = true;
        // `hotfix` is the last run, `deliver` the one `--line` names
        let out = frame(
            &p,
            &[deliver_run(), hotfix],
            "s1",
            0,
            Size { cols: 40, rows: 12 },
        );
        assert_eq!(
            out,
            "session s1 (UTC)

deliver  in progress
  ✓ design
  ✓ build
  ● guard     running
  ○ validate  pending
+ 1 other run: armadai flow status
"
        );
    }

    /// 87 runs, the one `--line` names (`deliver`) the last open one, with 20 transitions.
    fn many_runs() -> Vec<Run> {
        let marks = [
            Mark::Done,
            Mark::Running,
            Mark::Pending,
            Mark::Unsure,
            Mark::Unexpected,
            Mark::Relay,
            Mark::NotRun,
        ];
        let mut runs: Vec<Run> = (0..86)
            .map(|i| {
                let mut r = run(
                    &format!("flow-{i}"),
                    &[("s", marks[i % 7].clone()), ("t", Mark::Done)],
                );
                r.finished = i % 2 == 0;
                r.interrupted = i % 4 == 1;
                r
            })
            .collect();
        let mut open = deliver_run();
        open.transitions = (0..20)
            .map(|i| {
                (
                    1_000 + i,
                    "design".into(),
                    "build".into(),
                    Some("done".into()),
                )
            })
            .collect();
        runs.push(open);
        runs
    }

    #[test]
    fn every_image_fits_its_screen() {
        let p = project(&[("deliver", DELIVER)]);
        let runs = many_runs();
        let sizes = [(200, 100), (60, 30), (30, 10), (20, 3), (10, 1), (20, 2)];
        let at = |i: usize| {
            let (cols, rows) = sizes[i];
            frame(&p, &runs, "s", 0, Size { cols, rows })
        };
        for (i, (cols, rows)) in sizes.into_iter().enumerate() {
            let out = at(i);
            assert!(out.lines().count() < rows, "{cols}x{rows}\n{out}");
            assert!(
                out.lines().all(|l| l.chars().count() <= cols),
                "{cols}x{rows}\n{out}"
            );
        }
        let count = "+ 86 other runs: armadai flow status";
        // tier 1: the drawing, and the count line last
        assert!(at(0).contains("[✓ ▶ design (architect)  ]"));
        assert_eq!(at(0).lines().last(), Some(count));
        // tier 2: the list of the followed run, no box, the count line last
        assert!(!at(1).contains('['), "{}", at(1));
        assert_eq!(at(1).lines().last(), Some(count));
        // tier 3: cut, the last line says how many lines are left out (itself cut to the width)
        assert_eq!(at(2).lines().last(), Some("+ 20 more lines: armadai flow…"));
        assert_eq!(at(3).lines().last(), Some("+ 27 more lines: ar…"));
        assert_eq!(at(3).lines().count(), 2);
        assert_eq!(at(4), "");
        // two rows leave no room for the saying line: the first line of the list stays
        assert_eq!(at(5), "session s (UTC)\n");
    }

    #[test]
    fn a_run_longer_than_the_screen_is_cut_with_its_line() {
        let p = project(&[("deliver", DELIVER)]);
        let mut r = deliver_run();
        r.steps[0].note = Some("a note that is much wider than forty columns".into());
        r.transitions = vec![(1_000, "design".into(), "build".into(), Some("done".into()))];
        let out = frame(&p, &[r], "s1", 0, Size { cols: 40, rows: 8 });
        // 8 lines in all: 6 kept, 2 left out, the 7th line says so
        assert_eq!(
            out,
            "session s1 (UTC)

deliver  in progress
  ✓ design    (a note that is much wide…
  ✓ build
  ● guard     running
+ 2 more lines: armadai flow status
"
        );
    }

    #[test]
    fn several_lines_left_out_are_plural() {
        let p = project(&[("deliver", DELIVER)]);
        let out = frame(&p, &[deliver_run()], "s1", 0, Size { cols: 40, rows: 6 });
        assert_eq!(
            out,
            "session s1 (UTC)

deliver  in progress
  ✓ design
+ 3 more lines: armadai flow status
"
        );
    }

    #[test]
    fn a_group_or_no_open_run_is_the_list() {
        let p = project(&[("deliver", DELIVER)]);
        let group = [run("deliver | ship", &[("design", Mark::Running)])];
        assert_eq!(frame(&p, &group, "s", 0, BIG), render(&group, "s"));
        let mut done = deliver_run();
        done.finished = true;
        let runs = [done];
        assert_eq!(frame(&p, &runs, "s", 0, BIG), render(&runs, "s"));
    }

    #[test]
    fn a_screen_too_small_gives_the_list() {
        let p = project(&[("deliver", DELIVER)]);
        let runs = [deliver_run()];
        let image = frame(&p, &runs, "s", 0, BIG);
        let list = render(&runs, "s");
        assert_ne!(image, list);
        let cols = image.lines().map(|l| l.chars().count()).max().unwrap();
        let rows = image.lines().count();
        let at = |cols, rows| frame(&p, &runs, "s", 0, Size { cols, rows });
        assert_eq!(at(cols - 1, rows + 1), list, "one column short");
        assert_eq!(at(cols, rows + 1), image, "just fits");
        assert_eq!(at(cols, rows), list, "no row left for the cursor");
    }

    #[test]
    fn a_cycle_flow_names_every_link() {
        let p = project(&[("plan-build-verify", PBV)]);
        let r = run(
            "plan-build-verify",
            &[
                ("plan", Mark::Done),
                ("build", Mark::Running),
                ("check", Mark::Pending),
            ],
        );
        let out = frame(&p, &[r], "s", 0, BIG);
        assert!(out.lines().any(|l| l == "links not drawn:"), "{out}");
        assert!(
            out.lines().any(|l| l == "  check → build (failed)"),
            "{out}"
        );
    }
}
