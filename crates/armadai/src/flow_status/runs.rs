//! Where each declared flow stands, rebuilt from the recorded sub-agent events
//! (spec §4). Pure: the project and the session log in, the runs out.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use armadai_model::{Flow, Project};

use super::record::{EventKind, FlowEvent, GROUP_SEP};

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
///   them, the one a link not yet taken allows from a finished step of an open
///   run, else an entry step of a flow without an open run (which opens one;
///   an open run that is stuck, nothing running and its last step `?`/`✗`, is
///   closed first and stays visible, unfinished).
///   Flows sharing that entry step open one candidate run each, grouped: each
///   later event keeps the candidates it extends and drops the others; while
///   more than one remains they show as one run named `a | b`. Steps of two
///   distinct runs (or two steps of one): each is marked `?` "ambiguous step"
///   and no run moves.
/// - A run finishes when no step is running and no finished step has a link
///   still to take (a link is taken once its target starts after it stopped).
pub fn runs(project: &Project, events: &[FlowEvent]) -> Vec<Run> {
    let mut b = Builder {
        project,
        open: Vec::new(),
        assigned: HashMap::new(),
        tick: 0,
    };
    for e in merge_stops(events) {
        let targets = match e.event {
            EventKind::Start => b.start(&e),
            EventKind::Stop => match b.assigned.remove(&e.agent_id) {
                Some(t) => t,
                None => b.start(&e), // its start was not recorded
            },
        };
        if e.event == EventKind::Stop && !b.ambiguous(&targets) {
            for (i, step) in &targets {
                b.stop(*i, step, e.state.as_deref());
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<Building>> = BTreeMap::new();
    for r in b.open.into_iter().filter(|r| !r.dropped) {
        groups.entry(r.group).or_default().push(r);
    }
    groups.into_values().map(merge_candidates).collect()
}

/// One run for the candidates still grouped: their flows joined by ` | `, the
/// steps they all have (as the first sees them), finished when all are.
fn merge_candidates(mut members: Vec<Building>) -> Run {
    // already sorted: candidates open in the order of `project.flows`
    let flow = (members.iter().map(|m| m.run.flow.as_str()))
        .collect::<Vec<_>>()
        .join(GROUP_SEP);
    let finished = members.iter().all(|m| m.run.finished);
    let others: Vec<&'_ Flow> = members[1..].iter().map(|m| m.flow).collect();
    let mut run = members.swap_remove(0).finish();
    run.steps
        .retain(|v| others.iter().all(|f| f.steps.contains_key(&v.step)));
    run.flow = flow;
    run.finished = finished;
    run
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
    /// Candidate runs opened by one event share it; a dropped one extended nothing.
    group: usize,
    dropped: bool,
    /// Left stuck when its entry step started again.
    closed: bool,
    run: Run,
    /// Step -> tick of its last start / stop.
    started: HashMap<String, usize>,
    stopped: HashMap<String, usize>,
    taken: BTreeMap<(String, String), u32>,
}

impl<'p> Building<'p> {
    fn new(flow_id: &str, flow: &'p Flow, group: usize) -> Self {
        let pending = |s: &str| StepView {
            step: s.to_string(),
            mark: Mark::Pending,
            state: None,
            note: None,
        };
        Building {
            flow,
            group,
            dropped: false,
            closed: false,
            run: Run {
                flow: flow_id.to_string(),
                steps: step_order(flow).into_iter().map(pending).collect(),
                loops: Vec::new(),
                transitions: Vec::new(),
                finished: false,
            },
            started: HashMap::new(),
            stopped: HashMap::new(),
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

    /// The finished step (the latest) with a link to `to` still to take: `to`
    /// has not started since that step stopped, nor has the step itself (a
    /// restarted step's old stop opens nothing).
    fn linked_from(&self, to: &str) -> Option<String> {
        (self.stopped.iter())
            .filter(|(s, at)| {
                self.started.get(to).is_none_or(|t| t < *at)
                    && self.started.get(*s).is_none_or(|t| t < *at)
                    && next_steps(self.flow, s, self.leads_with(s)).contains(&to)
            })
            .max_by_key(|(_, at)| **at)
            .map(|(s, _)| s.clone())
    }

    fn running(&self) -> bool {
        self.run.steps.iter().any(|v| v.mark == Mark::Running)
    }

    /// Nothing running, and the last step that stopped left no known state.
    fn stuck(&self) -> bool {
        let last = self.stopped.iter().max_by_key(|(_, at)| **at);
        !self.running()
            && last.is_some_and(|(s, _)| {
                (self.run.steps.iter())
                    .any(|v| &v.step == s && matches!(v.mark, Mark::Unsure | Mark::Unexpected))
            })
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
    /// Orders starts and stops (several events can share a second).
    tick: usize,
}

impl<'p> Builder<'p> {
    fn open_run(&self, flow: &str) -> Option<usize> {
        (self.open.iter())
            .rposition(|r| r.run.flow == flow && !r.dropped && !r.closed)
            .filter(|i| !self.open[*i].run.finished)
    }

    /// Steps of two distinct runs, or two steps of one candidate.
    fn ambiguous(&self, targets: &[(usize, String)]) -> bool {
        let runs: BTreeSet<usize> = targets.iter().map(|(i, _)| *i).collect();
        let groups: BTreeSet<usize> = runs.iter().map(|i| self.open[*i].group).collect();
        groups.len() > 1 || runs.len() < targets.len()
    }

    fn start(&mut self, e: &FlowEvent) -> Vec<(usize, String)> {
        self.tick += 1;
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
        for (fid, _, sid) in &held {
            if let Some(i) = self.open_run(fid)
                && let Some(from) = self.open[i].linked_from(sid)
            {
                linked.push((i, sid, Some(from)));
            }
        }
        if linked.is_empty() {
            for (fid, flow, sid) in &held {
                if let Some(i) = self.open_run(fid)
                    && self.open[i].stuck()
                    && armadai_model::entry_steps(flow).iter().any(|s| s == sid)
                {
                    let group = self.open[i].group;
                    (self.open.iter_mut())
                        .filter(|r| r.group == group)
                        .for_each(|r| r.closed = true);
                }
            }
            let entries: Vec<_> = (held.iter())
                .filter(|(fid, flow, sid)| {
                    self.open_run(fid).is_none()
                        && armadai_model::entry_steps(flow).iter().any(|s| s == sid)
                })
                .collect();
            let group = self.open.len();
            for (fid, flow, sid) in entries {
                // two entry steps of one flow share the run they open
                let i = self.open_run(fid).unwrap_or_else(|| {
                    self.open.push(Building::new(fid, flow, group));
                    self.open.len() - 1
                });
                linked.push((i, sid, None));
            }
        }
        let targets: Vec<(usize, String)> = (linked.iter())
            .map(|(i, s, _)| (*i, s.to_string()))
            .collect();
        let ambiguous = self.ambiguous(&targets);
        if !ambiguous && let Some((i, _, _)) = linked.first() {
            // the candidates of this run that the event does not extend
            let group = self.open[*i].group;
            for (j, r) in self.open.iter_mut().enumerate() {
                if r.group == group && !linked.iter().any(|l| l.0 == j) {
                    r.dropped = true;
                }
            }
        }
        for (i, sid, from) in &linked {
            let run = &mut self.open[*i];
            if !ambiguous {
                run.started.insert(sid.to_string(), self.tick);
            }
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
        self.assigned.insert(e.agent_id.clone(), targets.clone());
        targets
    }

    fn stop(&mut self, i: usize, step: &str, state: Option<&str>) {
        self.tick += 1;
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
        run.stopped.insert(step.to_string(), self.tick);
        let to_take = (run.flow.steps.keys()).any(|t| run.linked_from(t).is_some());
        // a `?` or `✗` step never ends a run
        let unknown =
            (run.run.steps.iter()).any(|v| matches!(v.mark, Mark::Unsure | Mark::Unexpected));
        run.run.finished = !run.running() && !to_take && !unknown;
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
        for agent in [
            "architect",
            "dev",
            "qa",
            "ops",
            "lead",
            "analyst",
            "implementer",
            "coach",
            "ax",
            "ay",
            "az",
            "fan",
            "b1",
            "b2",
            "docs",
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

    /// `deliver` and `guide` share their entry step, then diverge.
    const SHARED_ENTRY: [(&str, &str); 2] = [
        (
            "deliver",
            "steps:\n  refine: { agent: analyst }\n  build: { agent: implementer }\n  ship: { agent: ops }\nlinks:\n  - { from: refine, to: build }\n  - { from: build, to: ship }\n",
        ),
        (
            "guide",
            "steps:\n  refine: { agent: analyst }\n  build: { agent: coach }\nlinks:\n  - { from: refine, to: build }\n",
        ),
    ];

    #[test]
    fn flows_sharing_their_entry_resolve_at_the_divergence() {
        let events = seq(&[
            &step(10, "r1", "analyst", None),
            &[start(20, "i1", "implementer")],
        ]);
        assert_eq!(
            runs(&project(&SHARED_ENTRY), &events),
            vec![Run {
                flow: "deliver".into(),
                steps: vec![
                    view("refine", Mark::Done, Some("done"), None),
                    view("build", Mark::Running, None, None),
                    view("ship", Mark::Pending, None, None),
                ],
                loops: vec![],
                transitions: vec![t(20, "refine", "build", Some("done"))],
                finished: false,
            }]
        );
    }

    #[test]
    fn flows_sharing_their_entry_stay_one_run_before_the_divergence() {
        let events = step(10, "r1", "analyst", None);
        assert_eq!(
            runs(&project(&SHARED_ENTRY), &events),
            vec![Run {
                flow: "deliver | guide".into(),
                steps: vec![
                    view("refine", Mark::Done, Some("done"), None),
                    view("build", Mark::Pending, None, None),
                ],
                loops: vec![],
                transitions: vec![],
                finished: false,
            }]
        );
    }

    const A_XY: (&str, &str) = (
        "a",
        "steps:\n  x: { agent: ax }\n  y: { agent: ay }\nlinks:\n  - { from: x, to: y }\n",
    );
    const B_XYZ: (&str, &str) = (
        "b",
        "steps:\n  x: { agent: ax }\n  y: { agent: ay }\n  z: { agent: az }\nlinks:\n  - { from: x, to: y }\n  - { from: y, to: z }\n",
    );

    #[test]
    fn a_group_with_one_finished_candidate_is_not_finished() {
        let events = seq(&[&step(10, "1", "ax", None), &step(20, "2", "ay", None)]);
        assert_eq!(
            runs(&project(&[A_XY, B_XYZ]), &events),
            vec![Run {
                flow: "a | b".into(),
                steps: vec![
                    view("x", Mark::Done, Some("done"), None),
                    view("y", Mark::Done, Some("done"), None),
                ],
                loops: vec![],
                transitions: vec![t(20, "x", "y", Some("done"))],
                finished: false,
            }]
        );
    }

    #[test]
    fn a_group_whose_candidates_all_end_is_finished() {
        let c_xy = ("c", A_XY.1);
        let events = seq(&[&step(10, "1", "ax", None), &step(20, "2", "ay", None)]);
        let got = runs(&project(&[A_XY, c_xy]), &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.finished))
                .collect::<Vec<_>>(),
            vec![("a | c", true)]
        );
    }

    #[test]
    fn a_finished_candidate_that_the_next_step_does_not_extend_is_dropped() {
        let events = seq(&[
            &step(10, "1", "ax", None),
            &step(20, "2", "ay", None),
            &[start(30, "3", "az")],
        ]);
        assert_eq!(
            runs(&project(&[A_XY, B_XYZ]), &events),
            vec![Run {
                flow: "b".into(),
                steps: vec![
                    view("x", Mark::Done, Some("done"), None),
                    view("y", Mark::Done, Some("done"), None),
                    view("z", Mark::Running, None, None),
                ],
                loops: vec![],
                transitions: vec![t(20, "x", "y", Some("done")), t(30, "y", "z", Some("done"))],
                finished: false,
            }]
        );
    }

    /// The `fan-out` shape: one step, then branches that end the flow.
    const FAN: (&str, &str) = (
        "fan",
        "steps:\n  a0: { agent: fan }\n  b1: { agent: b1 }\n  b2: { agent: b2 }\nlinks:\n  - { from: a0, to: b1 }\n  - { from: a0, to: b2 }\n",
    );

    #[test]
    fn a_fan_out_run_in_sequence_ends_after_its_last_branch() {
        let p = project(&[FAN]);
        let first = seq(&[&step(10, "1", "fan", None), &step(20, "2", "b1", None)]);
        let got = &runs(&p, &first)[0];
        assert_eq!(
            (got.steps[2].clone(), got.finished),
            (view("b2", Mark::Pending, None, None), false)
        );
        let all = seq(&[&first, &step(30, "3", "b2", None)]);
        assert!(runs(&p, &all)[0].finished);
    }

    #[test]
    fn a_step_is_linked_from_any_finished_step_not_only_the_last() {
        let p = project(&[(
            "deliver",
            "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  docs: { agent: docs }\n  guard: { agent: qa }\nlinks:\n  - { from: design, to: build }\n  - { from: design, to: docs }\n  - { from: build, to: guard }\n",
        )]);
        let events = seq(&[
            &step(10, "1", "architect", None),
            &[
                start(20, "2", "dev"),
                start(21, "3", "docs"),
                stop(22, "2", "dev", None),
                stop(23, "3", "docs", None),
                start(30, "4", "qa"),
            ],
        ]);
        let got = &runs(&p, &events)[0];
        assert_eq!(
            (got.steps[3].clone(), got.transitions[2].clone()),
            (
                view("guard", Mark::Running, None, None),
                t(30, "build", "guard", Some("done"))
            )
        );
    }

    #[test]
    fn an_entry_restarted_after_a_stuck_run_opens_a_new_one() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", None),
            &[start(40, "a4", "architect")],
        ]);
        let got = runs(&deliver(), &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.steps[0].clone(), r.steps[2].clone(), r.finished))
                .collect::<Vec<_>>(),
            vec![
                (
                    view("design", Mark::Done, Some("done"), None),
                    view("guard", Mark::Unsure, None, Some("no STATE line")),
                    false
                ),
                (
                    view("design", Mark::Running, None, None),
                    view("guard", Mark::Pending, None, None),
                    false
                ),
            ]
        );
    }

    #[test]
    fn an_entry_restarted_in_a_run_that_moves_on_is_ignored() {
        let healthy = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
        ]);
        let restarted = seq(&[&healthy, &[start(30, "a3", "architect")]]);
        assert_eq!(runs(&deliver(), &restarted), runs(&deliver(), &healthy));
    }

    /// design, build, guard `rejected`, build, then guard starts again.
    fn second_guard() -> Vec<FlowEvent> {
        seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("rejected")),
            &step(40, "a4", "dev", None),
            &[start(50, "a5", "qa")],
        ])
    }

    #[test]
    fn a_restarted_step_does_not_reopen_its_links() {
        let events = seq(&[&second_guard(), &[start(60, "a6", "ops")]]);
        assert_eq!(
            runs(&deliver(), &events)[0].steps,
            vec![
                view("design", Mark::Done, Some("done"), None),
                view("build", Mark::Done, Some("done"), None),
                view("guard", Mark::Running, None, None),
                view("validate", Mark::Pending, None, None),
            ]
        );
    }

    #[test]
    fn a_restarted_step_does_not_capture_the_entry_of_another_flow() {
        let release = ("release", "steps:\n  ship: { agent: ops }\n");
        let events = seq(&[&second_guard(), &[start(60, "a6", "ops")]]);
        let got = runs(&project(&[("deliver", DELIVER), release]), &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps.last().unwrap().clone()))
                .collect::<Vec<_>>(),
            vec![
                ("deliver", view("validate", Mark::Pending, None, None)),
                ("release", view("ship", Mark::Running, None, None)),
            ]
        );
    }

    #[test]
    fn a_last_step_without_a_known_state_does_not_end_the_run() {
        let p = project(&[(
            "check",
            "steps:\n  a: { agent: architect }\n  r: { agent: qa, states: [ok, ko] }\nlinks:\n  - { from: a, to: r }\n",
        )]);
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "qa", None),
        ]);
        let got = &runs(&p, &events)[0];
        assert_eq!(
            (got.steps[1].clone(), got.finished),
            (view("r", Mark::Unsure, None, Some("no STATE line")), false)
        );
    }

    #[test]
    fn two_runs_of_one_flow_follow_each_other() {
        let once = |base: u64, id: &str| {
            seq(&[
                &step(base, &format!("{id}1"), "architect", None),
                &step(base + 10, &format!("{id}2"), "dev", None),
                &step(base + 20, &format!("{id}3"), "qa", Some("approved")),
                &step(base + 30, &format!("{id}4"), "ops", None),
            ])
        };
        let got = runs(&deliver(), &seq(&[&once(10, "a"), &once(100, "b")]));
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.transitions[0].0, r.finished))
                .collect::<Vec<_>>(),
            vec![("deliver", 20, true), ("deliver", 110, true)]
        );
        assert!(
            got.iter()
                .all(|r| r.steps.iter().all(|v| v.mark == Mark::Done))
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
