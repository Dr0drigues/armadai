//! The image of `flow status --watch` in a terminal: pure, plain text (no colour,
//! no escape). The running flow is drawn in boxes with the mark of each step and a
//! spinner on the running one; the other runs stay listed under it.

use armadai_model::Project;

use super::line::open_run;
use super::render::{ordered, render, run_body, run_heading};
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

/// The image of `--watch` in a terminal. The last open run of a known flow is
/// drawn (heading, drawing, then its body) and the other runs are listed under
/// it as `render` lists them. Without such a run, or when the drawn part does
/// not fit `size`, the image is `render(runs, session)`.
pub fn frame(project: &Project, runs: &[Run], session: &str, tick: u64, size: Size) -> String {
    let list = || render(runs, session);
    let Some(run) = open_run(runs) else {
        return list();
    };
    let Some(flow) = project.flows.get(&run.flow) else {
        return list();
    };
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
        None => ("  ".into(), "  ".into()),
    });
    // the first line is `flow: <id>`, the heading says it already
    let drawing = drawing.split_once('\n').map_or("", |(_, rest)| rest);
    let mut out = format!(
        "session {session} (UTC)\n\n{}\n{drawing}\n{}",
        run_heading(run),
        run_body(run)
    );
    if !fits(&out, size) {
        return list();
    }
    for other in ordered(runs).into_iter().filter(|r| !std::ptr::eq(*r, run)) {
        out += &format!("\n{}\n{}", run_heading(other), run_body(other));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::runs::StepView;
    use super::*;

    const DELIVER: &str = "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  guard: { agent: qa, states: [approved, rejected] }\n  validate: { agent: ops }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: guard }\n  - { from: guard, to: build, when: rejected }\n  - { from: guard, to: validate, when: approved }\n";
    const PIPELINE: &str = "steps:\n  a: { agent: a }\n  b: { agent: b }\n  c: { agent: c }\nlinks:\n  - { from: a, to: b }\n  - { from: b, to: c }\n";
    const PBV: &str = "steps:\n  plan: { agent: a }\n  build: { agent: b }\n  check: { agent: c }\nlinks:\n  - { from: plan, to: build }\n  - { from: build, to: check }\n  - { from: check, to: build, when: failed }\n";
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

    #[test]
    fn two_ticks_differ_only_by_the_spinner() {
        let p = project(&[("deliver", DELIVER)]);
        let (a, b) = (
            frame(&p, &[deliver_run()], "s1", 0, BIG),
            frame(&p, &[deliver_run()], "s1", 1, BIG),
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
        assert!(diffs[0].2.contains("guard"), "{diffs:?}");
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

    #[test]
    fn the_last_open_run_is_drawn_and_the_others_listed() {
        let p = project(&[("deliver", DELIVER), ("pipeline", PIPELINE)]);
        let pipeline = run("pipeline", &[("a", Mark::Running), ("b", Mark::Pending)]);
        let mut hotfix = run("hotfix", &[("x", Mark::Done)]);
        hotfix.finished = true;
        let mut bugfix = run("bugfix", &[("y", Mark::Pending)]);
        bugfix.interrupted = true;
        let runs = [pipeline, deliver_run(), hotfix, bugfix];
        let out = frame(&p, &runs, "s1", 0, BIG);
        let drawn = frame(&p, &[deliver_run()], "s1", 0, BIG);
        assert_eq!(
            out,
            format!(
                "{drawn}
hotfix  finished
  ✓ x

bugfix  interrupted
  ○ y  pending

pipeline  in progress
  ● a  running
  ○ b  pending
"
            )
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
