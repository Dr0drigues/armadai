//! Where each declared flow stands, rebuilt from the recorded sub-agent events
//! (spec §4). Pure: the project and the session log in, the runs out.

use std::collections::{BTreeMap, HashMap};

use armadai_model::{Flow, Project};

use super::record::{EventKind, FlowEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Done,
    Running,
    Pending,
    Unsure,
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepView {
    pub step: String,
    pub mark: Mark,
    pub state: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub flow: String,
    pub steps: Vec<StepView>,
    /// Links taken more than once, as `(from, to, times)`.
    pub loops: Vec<(String, String, u32)>,
    /// Links taken, as `(ts, from, to, state of from)`.
    pub transitions: Vec<(u64, String, String, Option<String>)>,
    pub finished: bool,
}

/// The runs of every flow in the session, in the order they started.
///
/// - The step of an event: the steps its `agent_type` holds (an agent no step
///   holds, internal ones with an empty type included, is ignored); among
///   them, the one a link allows from the last finished step of an open run,
///   else an entry step of a flow without an open run (which opens one). More
///   than one such step: each is marked `?` "ambiguous step" and no run moves.
/// - A run finishes when a step is done with a known state that no link leads
///   on from, and no step of the run is still running.
pub fn runs(project: &Project, events: &[FlowEvent]) -> Vec<Run> {
    let mut b = Builder {
        project,
        open: Vec::new(),
        assigned: HashMap::new(),
    };
    for e in merge_stops(events) {
        let targets = match e.event {
            EventKind::Start => b.start(&e),
            EventKind::Stop => match b.assigned.remove(&e.agent_id) {
                Some(t) => t,
                None => b.start(&e), // its start was not recorded
            },
        };
        if e.event == EventKind::Stop
            && let [(i, step)] = targets.as_slice()
        {
            b.stop(*i, step, e.state.as_deref());
        }
    }
    b.open.into_iter().map(Building::finish).collect()
}

/// One stop per sub-agent run: a `SubagentHandback` (with the state) and the
/// `SubagentStop` after it (usually without) share an `agent_id`; the first
/// state recorded wins. A new start of the same id begins a new run.
fn merge_stops(events: &[FlowEvent]) -> Vec<FlowEvent> {
    let mut out: Vec<FlowEvent> = Vec::new();
    let mut last_stop: HashMap<&str, usize> = HashMap::new();
    for e in events {
        match e.event {
            EventKind::Start => {
                last_stop.remove(e.agent_id.as_str());
            }
            EventKind::Stop => {
                if let Some(&i) = last_stop.get(e.agent_id.as_str()) {
                    if out[i].state.is_none() {
                        out[i].state = e.state.clone();
                    }
                    continue;
                }
                last_stop.insert(&e.agent_id, out.len());
            }
        }
        out.push(e.clone());
    }
    out
}

struct Building<'p> {
    flow: &'p Flow,
    run: Run,
    last_done: Option<String>,
    taken: BTreeMap<(String, String), u32>,
}

impl<'p> Building<'p> {
    fn new(flow_id: &str, flow: &'p Flow) -> Self {
        let pending = |s: &str| StepView {
            step: s.to_string(),
            mark: Mark::Pending,
            state: None,
            note: None,
        };
        Building {
            flow,
            run: Run {
                flow: flow_id.to_string(),
                steps: step_order(flow).into_iter().map(pending).collect(),
                loops: Vec::new(),
                transitions: Vec::new(),
                finished: false,
            },
            last_done: None,
            taken: BTreeMap::new(),
        }
    }

    fn view(&mut self, step: &str) -> &mut StepView {
        (self.run.steps.iter_mut())
            .find(|v| v.step == step)
            .expect("a step of the flow")
    }

    /// The state a finished step leads on with; `None` when it is not known.
    fn leads_with(&self, step: &str) -> Option<&str> {
        (self.run.steps.iter())
            .find(|v| v.step == step && v.mark == Mark::Done)
            .and_then(|v| v.state.as_deref())
    }

    fn finish(mut self) -> Run {
        self.run.loops = (self.taken.into_iter())
            .filter(|(_, n)| *n > 1)
            .map(|((from, to), n)| (from, to, n))
            .collect();
        self.run
    }
}

struct Builder<'p> {
    project: &'p Project,
    open: Vec<Building<'p>>,
    /// `agent_id` -> the steps its start was matched to (several: ambiguous).
    assigned: HashMap<String, Vec<(usize, String)>>,
}

impl<'p> Builder<'p> {
    fn open_run(&self, flow: &str) -> Option<usize> {
        (self.open.iter())
            .rposition(|r| r.run.flow == flow)
            .filter(|i| !self.open[*i].run.finished)
    }

    fn start(&mut self, e: &FlowEvent) -> Vec<(usize, String)> {
        // (flow id, flow, step) for every step the agent holds
        let held: Vec<(&'p str, &'p Flow, &'p str)> = (self.project.flows.iter())
            .flat_map(|(fid, f)| {
                (f.steps.iter())
                    .filter(|(_, s)| s.agent == e.agent_type)
                    .map(move |(sid, _)| (fid.as_str(), f, sid.as_str()))
            })
            .collect();
        // (run, step, the finished step it is linked from)
        let mut linked: Vec<(usize, &str, Option<String>)> = Vec::new();
        for (fid, flow, sid) in &held {
            if let Some(i) = self.open_run(fid)
                && let Some(last) = &self.open[i].last_done
                && next_steps(flow, last, self.open[i].leads_with(last)).contains(sid)
            {
                linked.push((i, sid, Some(last.clone())));
            }
        }
        if linked.is_empty() {
            let entries: Vec<_> = (held.iter())
                .filter(|(fid, flow, sid)| {
                    self.open_run(fid).is_none()
                        && armadai_model::entry_steps(flow).iter().any(|s| s == sid)
                })
                .collect();
            for (fid, flow, sid) in entries {
                // two entry steps of one flow share the run they open
                let i = self.open_run(fid).unwrap_or_else(|| {
                    self.open.push(Building::new(fid, flow));
                    self.open.len() - 1
                });
                linked.push((i, sid, None));
            }
        }
        let ambiguous = linked.len() > 1;
        for (i, sid, from) in &linked {
            let run = &mut self.open[*i];
            *run.view(sid) = StepView {
                step: sid.to_string(),
                mark: if ambiguous {
                    Mark::Unsure
                } else {
                    Mark::Running
                },
                state: None,
                note: ambiguous.then(|| "ambiguous step".to_string()),
            };
            if let (false, Some(from)) = (ambiguous, from) {
                let state = run.leads_with(from).map(str::to_string);
                (run.run.transitions).push((e.ts, from.clone(), sid.to_string(), state));
                *run.taken
                    .entry((from.clone(), sid.to_string()))
                    .or_default() += 1;
            }
        }
        let targets: Vec<(usize, String)> = linked
            .into_iter()
            .map(|(i, s, _)| (i, s.to_string()))
            .collect();
        self.assigned.insert(e.agent_id.clone(), targets.clone());
        targets
    }

    fn stop(&mut self, i: usize, step: &str, state: Option<&str>) {
        let run = &mut self.open[i];
        let declared = &run.flow.steps[step].states;
        let (mark, state, note) = match state {
            _ if declared.is_empty() => (Mark::Done, Some("done"), None),
            None => (Mark::Unsure, None, Some("no STATE line")),
            // every step also has `done` and `failed` (project format)
            Some(s) if ["done", "failed"].contains(&s) || declared.iter().any(|d| d == s) => {
                (Mark::Done, Some(s), None)
            }
            Some(s) => (Mark::Unexpected, Some(s), Some("undeclared state")),
        };
        *run.view(step) = StepView {
            step: step.to_string(),
            mark,
            state: state.map(str::to_string),
            note: note.map(str::to_string),
        };
        run.last_done = Some(step.to_string());
        let ends = run
            .leads_with(step)
            .is_some_and(|s| next_steps(run.flow, step, Some(s)).is_empty());
        if ends && !run.run.steps.iter().any(|v| v.mark == Mark::Running) {
            run.run.finished = true;
        }
    }
}

/// The steps a finished `from` leads to, following the links (and
/// `on_failure`) its `state` takes; every one of them when it is not known.
fn next_steps<'f>(flow: &'f Flow, from: &str, state: Option<&str>) -> Vec<&'f str> {
    let links = (flow.links.iter())
        .filter(|l| l.from == from)
        .filter(|l| match (state, l.when.as_deref()) {
            (None, _) => true,
            (Some(s), None | Some("done")) => s != "failed",
            (Some(s), Some(w)) => s == w,
        })
        .map(|l| l.to.as_str());
    let failure = (flow.steps.get(from))
        .and_then(|s| s.on_failure.as_deref())
        .filter(|_| state.is_none_or(|s| s == "failed"));
    links.chain(failure).collect()
}

/// Entry steps first, then breadth-first along the links, then any other step.
fn step_order(flow: &Flow) -> Vec<&str> {
    let mut order: Vec<&str> = Vec::new();
    let entries = armadai_model::entry_steps(flow);
    let mut todo: std::collections::VecDeque<&str> = (flow.steps.keys())
        .map(String::as_str)
        .filter(|s| entries.iter().any(|e| e == s))
        .collect();
    while let Some(s) = todo.pop_front() {
        if order.contains(&s) {
            continue;
        }
        order.push(s);
        todo.extend(next_steps(flow, s, None));
    }
    let rest: Vec<&str> = (flow.steps.keys().map(String::as_str))
        .filter(|s| !order.contains(s))
        .collect();
    order.extend(rest);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELIVER: &str = "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  guard: { agent: qa, states: [approved, rejected] }\n  validate: { agent: ops }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: guard }\n  - { from: guard, to: build, when: rejected }\n  - { from: guard, to: validate, when: approved }\n";
    /// Shares `dev` with `deliver`.
    const HOTFIX: &str = "steps:\n  triage: { agent: lead }\n  patch: { agent: dev }\n  verify: { agent: lead }\nlinks:\n  - { from: triage, to: patch }\n  - { from: patch, to: verify }\n";

    fn project(flows: &[(&str, &str)]) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(".armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), "schema_version: 1\n").unwrap();
        for agent in ["architect", "dev", "qa", "ops", "lead"] {
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

    fn deliver() -> Project {
        project(&[("deliver", DELIVER)])
    }

    fn start(ts: u64, id: &str, agent: &str) -> FlowEvent {
        FlowEvent {
            ts,
            event: EventKind::Start,
            agent_id: id.into(),
            agent_type: agent.into(),
            state: None,
        }
    }

    fn stop(ts: u64, id: &str, agent: &str, state: Option<&str>) -> FlowEvent {
        FlowEvent {
            ts,
            event: EventKind::Stop,
            agent_id: id.into(),
            agent_type: agent.into(),
            state: state.map(str::to_string),
        }
    }

    /// A step started and stopped by one sub-agent.
    fn step(ts: u64, id: &str, agent: &str, state: Option<&str>) -> [FlowEvent; 2] {
        [start(ts, id, agent), stop(ts + 1, id, agent, state)]
    }

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

    fn seq(parts: &[&[FlowEvent]]) -> Vec<FlowEvent> {
        parts.concat()
    }

    #[test]
    fn a_nominal_run_is_done_and_finished() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("approved")),
            &step(40, "a4", "ops", None),
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![Run {
                flow: "deliver".into(),
                steps: vec![
                    view("design", Mark::Done, Some("done"), None),
                    view("build", Mark::Done, Some("done"), None),
                    view("guard", Mark::Done, Some("approved"), None),
                    view("validate", Mark::Done, Some("done"), None),
                ],
                loops: vec![],
                transitions: vec![
                    t(20, "design", "build", Some("done")),
                    t(30, "build", "guard", Some("done")),
                    t(40, "guard", "validate", Some("approved")),
                ],
                finished: true,
            }]
        );
    }

    #[test]
    fn a_loop_taken_twice_is_counted_in_one_run() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("rejected")),
            // a resumed sub-agent keeps its id: its new stop is not merged into the old one
            &step(40, "a2", "dev", None),
            &[start(50, "a5", "qa")],
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![Run {
                flow: "deliver".into(),
                steps: vec![
                    view("design", Mark::Done, Some("done"), None),
                    view("build", Mark::Done, Some("done"), None),
                    view("guard", Mark::Running, None, None),
                    view("validate", Mark::Pending, None, None),
                ],
                loops: vec![("build".into(), "guard".into(), 2)],
                transitions: vec![
                    t(20, "design", "build", Some("done")),
                    t(30, "build", "guard", Some("done")),
                    t(40, "guard", "build", Some("rejected")),
                    t(50, "build", "guard", Some("done")),
                ],
                finished: false,
            }]
        );
    }

    #[test]
    fn a_started_step_is_running_and_the_rest_pending() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &[start(20, "a2", "dev")],
        ]);
        assert_eq!(
            runs(&deliver(), &events)[0].steps,
            vec![
                view("design", Mark::Done, Some("done"), None),
                view("build", Mark::Running, None, None),
                view("guard", Mark::Pending, None, None),
                view("validate", Mark::Pending, None, None),
            ]
        );
    }

    #[test]
    fn a_step_with_states_and_no_state_line_is_unsure_and_keeps_the_run_open() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", None),
        ]);
        let got = &runs(&deliver(), &events)[0];
        assert_eq!(
            got.steps[2],
            view("guard", Mark::Unsure, None, Some("no STATE line"))
        );
        assert!(!got.finished);
    }

    #[test]
    fn an_undeclared_state_is_unexpected() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("scope_changed")),
        ]);
        let got = &runs(&deliver(), &events)[0];
        assert_eq!(
            got.steps[2],
            view(
                "guard",
                Mark::Unexpected,
                Some("scope_changed"),
                Some("undeclared state")
            )
        );
        assert!(!got.finished);
    }

    #[test]
    fn internal_and_unknown_sub_agents_are_ignored() {
        let plain = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
        ]);
        let noisy = seq(&[
            &step(10, "a1", "architect", None),
            &step(15, "x1", "", Some("approved")),
            &step(16, "x2", "Explore", None),
            &step(20, "a2", "dev", None),
            &[stop(25, "x3", "", None)],
        ]);
        assert_eq!(runs(&deliver(), &noisy), runs(&deliver(), &plain));
        assert_eq!(runs(&deliver(), &plain).len(), 1);
    }

    #[test]
    fn a_handback_and_its_subagent_stop_are_one_stop() {
        // PreToolUse(SubagentHandback) carries the state, SubagentStop follows without it.
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &[
                start(30, "a3", "qa"),
                stop(31, "a3", "qa", Some("rejected")),
                stop(32, "a3", "qa", None),
            ],
            &[start(40, "a4", "dev")],
        ]);
        let got = &runs(&deliver(), &events)[0];
        assert_eq!(
            got.steps[1..3],
            [
                view("build", Mark::Running, None, None),
                view("guard", Mark::Done, Some("rejected"), None),
            ]
        );
        assert_eq!(
            got.transitions[2],
            t(40, "guard", "build", Some("rejected"))
        );
    }

    #[test]
    fn the_state_of_either_stop_line_of_one_sub_agent_counts() {
        // a handback without a STATE line, then a SubagentStop whose last message has one
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &[
                start(30, "a3", "qa"),
                stop(31, "a3", "qa", None),
                stop(32, "a3", "qa", Some("approved")),
            ],
        ]);
        assert_eq!(
            runs(&deliver(), &events)[0].steps[2],
            view("guard", Mark::Done, Some("approved"), None)
        );
    }

    #[test]
    fn a_shared_agent_takes_the_step_a_link_allows() {
        let p = project(&[("deliver", DELIVER), ("hotfix", HOTFIX)]);
        // hotfix stays open after `patch`, where no link leads to `dev` again
        let events = seq(&[
            &step(10, "l1", "lead", None),
            &step(12, "d1", "dev", None),
            &step(14, "a1", "architect", None),
            &[start(20, "d2", "dev")],
        ]);
        let got = runs(&p, &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps[1].clone(), r.finished))
                .collect::<Vec<_>>(),
            vec![
                (
                    "hotfix",
                    view("patch", Mark::Done, Some("done"), None),
                    false
                ),
                ("deliver", view("build", Mark::Running, None, None), false),
            ]
        );
    }

    #[test]
    fn a_shared_agent_two_links_allow_is_unsure_in_both() {
        let p = project(&[("deliver", DELIVER), ("hotfix", HOTFIX)]);
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(12, "l1", "lead", None),
            &step(20, "a2", "dev", None),
        ]);
        let got = runs(&p, &events);
        let ambiguous = |s: &str| view(s, Mark::Unsure, None, Some("ambiguous step"));
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps[1].clone()))
                .collect::<Vec<_>>(),
            vec![
                ("deliver", ambiguous("build")),
                ("hotfix", ambiguous("patch"))
            ]
        );
        assert!(got.iter().all(|r| r.transitions.is_empty() && !r.finished));
    }
}
