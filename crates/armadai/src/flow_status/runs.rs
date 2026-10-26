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
    /// A step the coordinator holds: the main thread, never recorded.
    Relay,
    /// A step before the one a run opened at: not seen in this session.
    NotRun,
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
///   run (a `?`/`✗` step leads nowhere; a step of the coordinator, never
///   recorded and shown `~`, is crossed to the steps after it); else a `?` step
///   of an open run, retried in place; else a step where a run opens (an entry
///   step, or one right after a step of the coordinator) of a flow without an
///   open run; else, when no flow holding the agent has an open run (none, or
///   finished), the step itself, the steps before it shown `-` (not run in
///   this session; two steps of one flow: one run, both `?`), but never a step
///   right after a step of the coordinator in mid-flow. An entry
///   starting again first closes the open run of its flow (kept, unfinished)
///   when nothing leads back to it, when that run is stuck (nothing running
///   and its last step `?`/`✗`), or when the entry was not run in it.
///   Flows sharing that entry step open one candidate run each, grouped: each
///   later event keeps the candidates it extends and drops the others; while
///   more than one remains they show as one run named `a | b`. Links of
///   several runs to the step: the most recent (its source stopped last, by
///   `ts`) wins, none when another run already took a more recent link to a
///   step the agent holds. Steps of two distinct runs left (a tie), or two
///   steps of one: each is marked `?` "ambiguous step" and no run moves.
/// - A run finishes when no step is running or `?`/`✗`, and no finished step
///   has a link still to take (taken once its target starts after it stopped;
///   a loop back to the same step through the coordinator does not count, and
///   goes on in the same run when that step starts again).
pub fn runs(project: &Project, events: &[FlowEvent]) -> Vec<Run> {
    let mut b = Builder {
        project,
        open: Vec::new(),
        assigned: HashMap::new(),
        tick: 0,
    };
    for e in merge_stops(events) {
        let targets = match e.event {
            EventKind::Command => continue, // used for attribution (task 3)
            EventKind::Start => b.start(&e),
            EventKind::Stop => match b.assigned.remove(&e.agent_id) {
                Some(t) => t,
                None => b.start(&e), // its start was not recorded
            },
        };
        if e.event == EventKind::Stop && !b.ambiguous(&targets) {
            for (i, step) in &targets {
                b.stop(*i, step, e.ts, e.state.as_deref());
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
            EventKind::Command => {}
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

/// A finished step and the coordinator's steps crossed from it.
type Via<'p> = (String, Vec<&'p str>);

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
    /// Step -> `ts` of its last stop.
    stopped_ts: HashMap<String, u64>,
    /// Step -> `ts` of the stop of the step whose link it last started on.
    linked_at: HashMap<String, u64>,
    /// Hop -> tick it was last recorded at.
    crossed: HashMap<(String, String), usize>,
    taken: BTreeMap<(String, String), u32>,
}

impl<'p> Building<'p> {
    fn new(flow_id: &str, flow: &'p Flow, group: usize, coordinator: Option<&str>) -> Self {
        let pending = |s: &str| {
            let relay = coordinator == Some(flow.steps[s].agent.as_str());
            StepView {
                step: s.to_string(),
                mark: if relay { Mark::Relay } else { Mark::Pending },
                state: None,
                note: relay.then(|| "coordinator, not recorded".to_string()),
            }
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
            stopped_ts: HashMap::new(),
            linked_at: HashMap::new(),
            crossed: HashMap::new(),
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
    /// A `?` or `✗` step leads nowhere; a step of the coordinator is crossed
    /// (all its links: it records no state). Returns the step and the
    /// coordinator's steps crossed on the way.
    fn linked_from(&self, to: &str) -> Option<Via<'p>> {
        let mark = |step: &str| {
            (self.run.steps.iter())
                .find(|v| v.step == step)
                .map(|v| &v.mark)
        };
        if mark(to) == Some(&Mark::Relay) {
            return None;
        }
        (self.stopped.iter())
            .filter(|(s, at)| {
                !matches!(mark(s), Some(Mark::Unsure | Mark::Unexpected))
                    && self.started.get(to).is_none_or(|t| t < *at)
                    && self.started.get(*s).is_none_or(|t| t < *at)
            })
            .filter_map(|(s, at)| {
                let path = self.reach(s).into_iter().find(|(t, _)| *t == to)?.1;
                Some((at, s.clone(), path))
            })
            .max_by_key(|(at, _, _)| **at)
            .map(|(_, s, path)| (s, path))
    }

    /// The recorded steps `from` leads to, each with the coordinator's steps
    /// crossed to reach it.
    fn reach(&self, from: &str) -> Vec<(&'p str, Vec<&'p str>)> {
        let relay = |s: &str| (self.run.steps.iter()).any(|v| v.step == s && v.mark == Mark::Relay);
        let mut todo: Vec<(&'p str, Vec<&'p str>)> =
            (next_steps(self.flow, from, self.leads_with(from)))
                .into_iter()
                .map(|t| (t, Vec::new()))
                .collect();
        let (mut out, mut crossed) = (Vec::new(), BTreeSet::new());
        while let Some((t, path)) = todo.pop() {
            if !relay(t) {
                out.push((t, path));
            } else if crossed.insert(t) {
                let path = [path, vec![t]].concat();
                todo.extend(
                    next_steps(self.flow, t, None)
                        .into_iter()
                        .map(|n| (n, path.clone())),
                );
            }
        }
        out
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

    fn relay(&self, flow: &Flow, step: &str) -> bool {
        self.project.manifest.coordinator.as_deref() == Some(flow.steps[step].agent.as_str())
    }

    /// The coordinator's steps a flow starts with: entry steps, or steps
    /// only such steps lead to.
    fn leading_relays<'f>(&self, flow: &'f Flow) -> BTreeSet<&'f str> {
        let entries = armadai_model::entry_steps(flow);
        let relays: Vec<&str> = (flow.steps.keys().map(String::as_str))
            .filter(|r| self.relay(flow, r))
            .collect();
        let mut lead: BTreeSet<&str> = (relays.iter().copied())
            .filter(|r| entries.iter().any(|e| e == r))
            .collect();
        loop {
            let more: Vec<&str> = (relays.iter().copied())
                .filter(|r| !lead.contains(r))
                .filter(|r| {
                    (flow.steps.keys())
                        .filter(|s| next_steps(flow, s, None).contains(r))
                        .all(|s| lead.contains(s.as_str()))
                })
                .collect();
            if more.is_empty() {
                return lead;
            }
            lead.extend(more);
        }
    }

    /// Where a run can open: an entry step, or a step right after a step of the
    /// coordinator the flow starts with (its handing over is not recorded).
    fn opens_at(&self, flow: &Flow, step: &str) -> bool {
        armadai_model::entry_steps(flow).iter().any(|s| s == step)
            || (self.leading_relays(flow).iter()).any(|r| next_steps(flow, r, None).contains(&step))
    }

    /// A step leads to `step` (a link or an `on_failure`), the coordinator's
    /// steps the flow starts with aside: they are no way back.
    fn has_incoming(&self, flow: &Flow, step: &str) -> bool {
        let lead = self.leading_relays(flow);
        (flow.steps.keys())
            .any(|s| !lead.contains(s.as_str()) && next_steps(flow, s, None).contains(&step))
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
        let mut linked: Vec<(usize, &str, Option<Via>)> = Vec::new();
        for (fid, _, sid) in &held {
            // a finished run has no link left but a loop back through the
            // coordinator to the same step (see `stop`): that one goes on
            let last =
                (self.open.iter()).rposition(|r| r.run.flow == *fid && !r.dropped && !r.closed);
            if let Some(i) = last
                && let Some(from) = self.open[i].linked_from(sid)
            {
                linked.push((i, sid, Some(from)));
            }
        }
        // links of several groups of runs: the most recent (its source
        // stopped last) wins, a tie stays ambiguous; none wins against a more
        // recent link another run already took to a step the agent holds.
        // Candidates of one group are never told apart this way.
        // The guard needs no group filter: an event extends every candidate
        // left, so a link still open in a group stopped after any it took.
        let link_ts = |(i, _, from): &(usize, &str, Option<Via>)| {
            from.as_ref()
                .map_or(0, |(f, _)| self.open[*i].stopped_ts[f])
        };
        let newest = linked.iter().map(link_ts).max();
        let taken = (held.iter())
            .filter_map(|(fid, _, sid)| {
                let i = (self.open.iter())
                    .rposition(|r| r.run.flow == *fid && !r.dropped && !r.closed)?;
                self.open[i].linked_at.get(*sid).copied()
            })
            .max();
        if newest.is_some_and(|n| taken.is_some_and(|t| t > n)) {
            return Vec::new();
        }
        if let Some(n) = newest {
            let groups: BTreeSet<usize> = (linked.iter())
                .filter(|l| link_ts(l) == n)
                .map(|l| self.open[l.0].group)
                .collect();
            linked.retain(|l| groups.contains(&self.open[l.0].group));
        }
        if linked.is_empty() {
            // a `?` or `✗` step started again: retried in place, no transition
            for (fid, _, sid) in &held {
                if let Some(i) = self.open_run(fid)
                    && self.open[i].stopped.contains_key(*sid)
                    && self.open[i].run.steps.iter().any(|v| {
                        v.step == *sid && matches!(v.mark, Mark::Unsure | Mark::Unexpected)
                    })
                {
                    linked.push((i, sid, None));
                }
            }
        }
        if linked.is_empty() {
            // an entry starting again closes the open run when nothing leads
            // back to it, or when that run is stuck
            for (fid, flow, sid) in &held {
                if let Some(i) = self.open_run(fid)
                    && self.opens_at(flow, sid)
                    && (!self.has_incoming(flow, sid)
                        || self.open[i].stuck()
                        || self.open[i]
                            .run
                            .steps
                            .iter()
                            .any(|v| v.step == *sid && v.mark == Mark::NotRun))
                {
                    let group = self.open[i].group;
                    (self.open.iter_mut())
                        .filter(|r| r.group == group)
                        .for_each(|r| r.closed = true);
                }
            }
            let entries: Vec<_> = (held.iter())
                .filter(|(fid, flow, sid)| self.open_run(fid).is_none() && self.opens_at(flow, sid))
                .collect();
            let group = self.open.len();
            for (fid, flow, sid) in entries {
                // two entry steps of one flow share the run they open
                let i = self.open_run(fid).unwrap_or_else(|| {
                    let coordinator = self.project.manifest.coordinator.as_deref();
                    self.open.push(Building::new(fid, flow, group, coordinator));
                    self.open.len() - 1
                });
                linked.push((i, sid, None));
            }
        }
        if linked.is_empty() {
            // no flow holding the agent has a run open (none, or finished): a
            // run opens where the step starts, the steps before it not run in
            // this session; not right after a step of the coordinator in
            // mid-flow, whose handing over cannot be told from a new run
            let none_open = held.iter().all(|(fid, _, _)| self.open_run(fid).is_none());
            let group = self.open.len();
            for (fid, flow, sid) in held.iter().filter(|_| none_open) {
                let after_relay = (flow.steps.keys())
                    .any(|r| self.relay(flow, r) && next_steps(flow, r, None).contains(sid));
                if after_relay {
                    continue;
                }
                // two steps of one flow share the run they open (ambiguous)
                let i = self.open_run(fid).unwrap_or_else(|| {
                    let coordinator = self.project.manifest.coordinator.as_deref();
                    self.open.push(Building::new(fid, flow, group, coordinator));
                    self.open.len() - 1
                });
                linked.push((i, sid, None));
            }
            // the steps before every step the run may have opened at
            for (i, _, _) in &linked {
                let flow = self.open[*i].flow;
                let firsts: Vec<&str> =
                    (linked.iter()).filter(|l| l.0 == *i).map(|l| l.1).collect();
                for before in upstream(flow, firsts[0]) {
                    let v = self.open[*i].view(before);
                    if v.mark == Mark::Pending
                        && firsts.iter().all(|f| upstream(flow, f).contains(&before))
                    {
                        v.mark = Mark::NotRun;
                        v.note = Some("not run in this session".to_string());
                    }
                }
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
                run.run.finished = false;
            }
            if let (false, Some((from, crossed))) = (ambiguous, from) {
                // one transition per hop, through the coordinator's steps; read
                // before the view below: `from` may be this very step
                let mut state = run.leads_with(from).map(str::to_string);
                let mut prev = from.clone();
                let left = run.stopped[from];
                run.linked_at.insert(sid.to_string(), run.stopped_ts[from]);
                for (k, to) in crossed.iter().copied().chain([*sid]).enumerate() {
                    let hop = (prev.clone(), to.to_string());
                    // a hop into the coordinator counts once per stop of `from`,
                    // however many steps it then reaches (a fan-out)
                    if k == crossed.len() || run.crossed.get(&hop).is_none_or(|at| *at < left) {
                        run.crossed.insert(hop.clone(), self.tick);
                        (run.run.transitions).push((
                            e.ts,
                            prev.clone(),
                            to.to_string(),
                            state.clone(),
                        ));
                        *run.taken.entry(hop).or_default() += 1;
                    }
                    state = None;
                    prev = to.to_string();
                }
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
        }
        self.assigned.insert(e.agent_id.clone(), targets.clone());
        targets
    }

    fn stop(&mut self, i: usize, step: &str, ts: u64, state: Option<&str>) {
        self.tick += 1;
        let run = &mut self.open[i];
        let declared = &run.flow.steps[step].states;
        let (mark, state, note) = match state {
            Some("failed") => (Mark::Done, Some("failed"), None),
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
        run.stopped_ts.insert(step.to_string(), ts);
        // a loop back to the same step through the coordinator holds nothing open
        let to_take =
            (run.flow.steps.keys()).any(|t| run.linked_from(t).is_some_and(|(s, _)| &s != t));
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

/// The steps before `step`: reached from an entry step without crossing it,
/// and leading to it.
fn upstream<'f>(flow: &'f Flow, step: &str) -> Vec<&'f str> {
    let entries = armadai_model::entry_steps(flow);
    let from = (flow.steps.keys().map(String::as_str)).filter(|s| entries.iter().any(|e| e == s));
    (reachable(flow, from, Some(step)).into_iter())
        .filter(|s| reachable(flow, [*s], None).contains(step))
        .collect()
}

/// The steps `from` and the links lead to, never through `avoid`.
fn reachable<'f>(
    flow: &'f Flow,
    from: impl IntoIterator<Item = &'f str>,
    avoid: Option<&str>,
) -> BTreeSet<&'f str> {
    let mut todo: Vec<&str> = from.into_iter().filter(|s| Some(*s) != avoid).collect();
    let mut seen = BTreeSet::new();
    while let Some(s) = todo.pop() {
        if seen.insert(s) {
            todo.extend(
                next_steps(flow, s, None)
                    .into_iter()
                    .filter(|t| Some(*t) != avoid),
            );
        }
    }
    seen
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
        project_with("schema_version: 1\n", flows)
    }

    fn project_with(manifest: &str, flows: &[(&str, &str)]) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(".armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), manifest).unwrap();
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
            "framer",
            "designer",
            "guardian",
            "validator",
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
            command: None,
        }
    }

    fn stop(ts: u64, id: &str, agent: &str, state: Option<&str>) -> FlowEvent {
        FlowEvent {
            ts,
            event: EventKind::Stop,
            agent_id: id.into(),
            agent_type: agent.into(),
            state: state.map(str::to_string),
            command: None,
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

    fn run(
        flow: &str,
        steps: Vec<StepView>,
        transitions: Vec<(u64, String, String, Option<String>)>,
        finished: bool,
    ) -> Run {
        Run {
            flow: flow.into(),
            steps,
            loops: vec![],
            transitions,
            finished,
        }
    }

    fn done(step: &str, state: &str) -> StepView {
        view(step, Mark::Done, Some(state), None)
    }

    fn pending(step: &str) -> StepView {
        view(step, Mark::Pending, None, None)
    }

    /// A whole `deliver` run, approved at once, from `base` on.
    fn approved_run(base: u64, id: &str) -> Vec<FlowEvent> {
        seq(&[
            &step(base, &format!("{id}1"), "architect", None),
            &step(base + 10, &format!("{id}2"), "dev", None),
            &step(base + 20, &format!("{id}3"), "qa", Some("approved")),
            &step(base + 30, &format!("{id}4"), "ops", None),
        ])
    }

    fn approved_view(base: u64, finished: bool) -> Run {
        run(
            "deliver",
            vec![
                done("design", "done"),
                done("build", "done"),
                done("guard", "approved"),
                done("validate", "done"),
            ],
            vec![
                t(base + 10, "design", "build", Some("done")),
                t(base + 20, "build", "guard", Some("done")),
                t(base + 30, "guard", "validate", Some("approved")),
            ],
            finished,
        )
    }

    /// Review E12: a `?` step leads nowhere, so `ops` is not taken for
    /// `validate`, and the second run is not merged into the first.
    #[test]
    fn an_unsure_step_leads_nowhere() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", None),
            &step(40, "a4", "ops", None),
            &approved_run(100, "b"),
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![
                run(
                    "deliver",
                    vec![
                        done("design", "done"),
                        done("build", "done"),
                        view("guard", Mark::Unsure, None, Some("no STATE line")),
                        pending("validate"),
                    ],
                    vec![
                        t(20, "design", "build", Some("done")),
                        t(30, "build", "guard", Some("done")),
                    ],
                    false,
                ),
                approved_view(100, true),
            ]
        );
    }

    /// Review E12 with an undeclared state: a `✗` step leads nowhere either.
    #[test]
    fn an_unexpected_step_leads_nowhere() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("scope_changed")),
            &step(40, "a4", "ops", None),
            &approved_run(100, "b"),
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![
                run(
                    "deliver",
                    vec![
                        done("design", "done"),
                        done("build", "done"),
                        view(
                            "guard",
                            Mark::Unexpected,
                            Some("scope_changed"),
                            Some("undeclared state")
                        ),
                        pending("validate"),
                    ],
                    vec![
                        t(20, "design", "build", Some("done")),
                        t(30, "build", "guard", Some("done")),
                    ],
                    false,
                ),
                approved_view(100, true),
            ]
        );
    }

    /// Review E7: a `?` step started again is retried, without a transition,
    /// and its new state replaces the `?`.
    #[test]
    fn an_unsure_step_can_be_retried() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", None),
            &step(35, "a3b", "qa", Some("approved")),
            &step(40, "a4", "ops", None),
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![run(
                "deliver",
                vec![
                    done("design", "done"),
                    done("build", "done"),
                    done("guard", "approved"),
                    done("validate", "done"),
                ],
                vec![
                    t(20, "design", "build", Some("done")),
                    t(30, "build", "guard", Some("done")),
                    t(40, "guard", "validate", Some("approved")),
                ],
                true,
            )]
        );
    }

    /// Review N7: a `✗` step started again is retried in place, like a `?` one.
    #[test]
    fn an_unexpected_step_can_be_retried() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("bogus")),
            &step(40, "a4", "qa", Some("approved")),
            &step(50, "a5", "ops", None),
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![run(
                "deliver",
                vec![
                    done("design", "done"),
                    done("build", "done"),
                    done("guard", "approved"),
                    done("validate", "done"),
                ],
                vec![
                    t(20, "design", "build", Some("done")),
                    t(30, "build", "guard", Some("done")),
                    t(50, "guard", "validate", Some("approved")),
                ],
                true,
            )]
        );
    }

    /// Review E8: an entry step without incoming links that starts again closes
    /// the open run, even with a step still running, and opens a new one.
    #[test]
    fn an_entry_restarted_closes_the_open_run_even_with_a_step_running() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("approved")),
            &[start(40, "a4", "ops")],
            &approved_run(100, "b"),
        ]);
        let mut first = approved_view(0, false);
        first.steps[3] = view("validate", Mark::Running, None, None);
        first.transitions = vec![
            t(20, "design", "build", Some("done")),
            t(30, "build", "guard", Some("done")),
            t(40, "guard", "validate", Some("approved")),
        ];
        assert_eq!(
            runs(&deliver(), &events),
            vec![first, approved_view(100, true)]
        );
    }

    /// Review E8b: the restart from `design` is a new run, not the loop
    /// `guard -> build` of the first one.
    #[test]
    fn an_entry_restarted_after_a_rejection_is_a_new_run_not_the_loop() {
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", None),
            &step(30, "a3", "qa", Some("rejected")),
            &step(100, "b1", "architect", None),
            &step(110, "b2", "dev", None),
        ]);
        assert_eq!(
            runs(&deliver(), &events),
            vec![
                run(
                    "deliver",
                    vec![
                        done("design", "done"),
                        done("build", "done"),
                        done("guard", "rejected"),
                        pending("validate"),
                    ],
                    vec![
                        t(20, "design", "build", Some("done")),
                        t(30, "build", "guard", Some("done")),
                    ],
                    false,
                ),
                run(
                    "deliver",
                    vec![
                        done("design", "done"),
                        view("build", Mark::Done, Some("done"), None),
                        pending("guard"),
                        pending("validate"),
                    ],
                    vec![t(110, "design", "build", Some("done"))],
                    false,
                ),
            ]
        );
    }

    /// Review E2: `STATE: failed` on a step without states follows `on_failure`.
    #[test]
    fn a_failed_stateless_step_follows_its_failure_path() {
        let p = project(&[(
            "f",
            "steps:\n  design: { agent: architect }\n  build: { agent: dev, on_failure: fix }\n  fix: { agent: implementer }\n  test: { agent: qa }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: test }\n  - { from: fix, to: test }\n",
        )]);
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "dev", Some("failed")),
            &step(30, "a3", "implementer", None),
            &step(40, "a4", "qa", None),
        ]);
        assert_eq!(
            runs(&p, &events),
            vec![run(
                "f",
                vec![
                    done("design", "done"),
                    done("build", "failed"),
                    done("test", "done"),
                    done("fix", "done"),
                ],
                vec![
                    t(20, "design", "build", Some("done")),
                    t(30, "build", "fix", Some("failed")),
                    t(40, "fix", "test", Some("done")),
                ],
                true,
            )]
        );
    }

    /// Review E11b: an entry a loop leads back to closes a run only when it is stuck.
    #[test]
    fn an_entry_with_a_loop_back_reopens_only_a_stuck_run() {
        let p = project(&[(
            "check",
            "steps:\n  a: { agent: architect }\n  r: { agent: qa, states: [ok, ko] }\nlinks:\n  - { from: a, to: r }\n  - { from: r, to: a, when: ko }\n",
        )]);
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(20, "a2", "qa", None),
            &step(30, "a3", "architect", None),
            &step(40, "a4", "qa", Some("ok")),
        ]);
        assert_eq!(
            runs(&p, &events),
            vec![
                run(
                    "check",
                    vec![
                        done("a", "done"),
                        view("r", Mark::Unsure, None, Some("no STATE line")),
                    ],
                    vec![t(20, "a", "r", Some("done"))],
                    false,
                ),
                run(
                    "check",
                    vec![done("a", "done"), done("r", "ok")],
                    vec![t(40, "a", "r", Some("done"))],
                    true,
                ),
            ]
        );
    }

    const COORDINATED: &str = "schema_version: 1\ncoordinator: lead\n";

    fn relay(step: &str) -> StepView {
        view(step, Mark::Relay, None, Some("coordinator, not recorded"))
    }

    /// A flow that starts with the coordinator opens on the step after it.
    #[test]
    fn a_flow_led_by_the_coordinator_opens_on_its_first_delegated_step() {
        let p = project_with(
            COORDINATED,
            &[(
                "plan",
                "steps:\n  frame: { agent: lead }\n  build: { agent: dev }\n  check: { agent: qa }\nlinks:\n  - { from: frame, to: build }\n  - { from: build, to: check }\n",
            )],
        );
        let events = seq(&[&step(10, "a1", "dev", None), &step(20, "a2", "qa", None)]);
        assert_eq!(
            runs(&p, &events),
            vec![run(
                "plan",
                vec![relay("frame"), done("build", "done"), done("check", "done")],
                vec![t(20, "build", "check", Some("done"))],
                true,
            )]
        );
    }

    /// The coordinator's step is crossed: the run goes on to the step after it.
    #[test]
    fn a_coordinator_step_is_crossed_within_one_run() {
        let p = project_with(
            COORDINATED,
            &[(
                "pass",
                "steps:\n  build: { agent: dev }\n  hand: { agent: lead }\n  check: { agent: qa }\nlinks:\n  - { from: build, to: hand }\n  - { from: hand, to: check }\n",
            )],
        );
        let half = step(10, "a1", "dev", None);
        let all = seq(&[&half, &step(20, "a2", "qa", None)]);
        let steps = |check: StepView| vec![done("build", "done"), relay("hand"), check];
        assert_eq!(
            (runs(&p, &half), runs(&p, &all)),
            (
                vec![run("pass", steps(pending("check")), vec![], false)],
                vec![run(
                    "pass",
                    steps(done("check", "done")),
                    vec![
                        t(20, "build", "hand", Some("done")),
                        t(20, "hand", "check", None),
                    ],
                    true,
                )],
            )
        );
    }

    /// A review loop through the coordinator, twice: one run, counted.
    #[test]
    fn a_review_loop_through_the_coordinator_is_one_run() {
        let p = project_with(
            COORDINATED,
            &[(
                "rev",
                "steps:\n  triage: { agent: lead }\n  write: { agent: dev }\n  review: { agent: qa, states: [approved, rejected] }\n  ship: { agent: ops }\nlinks:\n  - { from: triage, to: write }\n  - { from: write, to: review }\n  - { from: review, to: triage, when: rejected }\n  - { from: review, to: ship, when: approved }\n",
            )],
        );
        let events = seq(&[
            &step(10, "a1", "dev", None),
            &step(20, "a2", "qa", Some("rejected")),
            &step(30, "a3", "dev", None),
            &step(40, "a4", "qa", Some("rejected")),
            &step(50, "a5", "dev", None),
            &step(60, "a6", "qa", Some("approved")),
            &step(70, "a7", "ops", None),
        ]);
        let got = runs(&p, &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.steps.clone(), r.loops.clone(), r.finished))
                .collect::<Vec<_>>(),
            vec![(
                vec![
                    relay("triage"),
                    done("write", "done"),
                    done("review", "approved"),
                    done("ship", "done"),
                ],
                vec![
                    ("review".into(), "triage".into(), 2),
                    ("triage".into(), "write".into(), 2),
                    ("write".into(), "review".into(), 3),
                ],
                true,
            )]
        );
    }

    /// A loop back to the same step through the coordinator: the step may run
    /// again in the same run, and the loop never holds the run open.
    #[test]
    fn a_loop_through_the_coordinator_to_the_same_step_ends_the_run() {
        let p = project_with(
            COORDINATED,
            &[(
                "again",
                // a cycle has no entry step of its own: `start` gives it one
                "start: build\nsteps:\n  build: { agent: dev }\n  check: { agent: lead }\nlinks:\n  - { from: build, to: check }\n  - { from: check, to: build }\n",
            )],
        );
        let once = step(10, "a1", "dev", None);
        let again = seq(&[&once, &[start(20, "a2", "dev")]]);
        assert!(!runs(&p, &again)[0].finished, "running again");
        let twice = seq(&[&once, &step(20, "a2", "dev", None)]);
        assert_eq!(
            (runs(&p, &once), runs(&p, &twice)),
            (
                vec![run(
                    "again",
                    vec![done("build", "done"), relay("check")],
                    vec![],
                    true
                )],
                vec![Run {
                    flow: "again".into(),
                    steps: vec![done("build", "done"), relay("check")],
                    loops: vec![],
                    transitions: vec![
                        t(20, "build", "check", Some("done")),
                        t(20, "check", "build", None),
                    ],
                    finished: true,
                }],
            )
        );
    }

    const PLAN: &str = "steps:\n  frame: { agent: lead }\n  build: { agent: dev }\n  check: { agent: qa }\nlinks:\n  - { from: frame, to: build }\n  - { from: build, to: check }\n";
    const PASS: &str = "steps:\n  build: { agent: dev }\n  hand: { agent: lead }\n  check: { agent: qa }\nlinks:\n  - { from: build, to: hand }\n  - { from: hand, to: check }\n";

    fn plan_second_run() -> Run {
        run(
            "plan",
            vec![relay("frame"), done("build", "done"), done("check", "done")],
            vec![t(110, "build", "check", Some("done"))],
            true,
        )
    }

    /// Review N1: a flow led by the coordinator, left after `build`, starts
    /// again: the link from the leading coordinator's step is no way back.
    #[test]
    fn a_coordinator_led_flow_left_halfway_starts_again() {
        let p = project_with(COORDINATED, &[("plan", PLAN)]);
        let events = seq(&[
            &step(10, "a1", "dev", None),
            &step(100, "b1", "dev", None),
            &step(110, "b2", "qa", None),
        ]);
        assert_eq!(
            runs(&p, &events),
            vec![
                run(
                    "plan",
                    vec![relay("frame"), done("build", "done"), pending("check")],
                    vec![],
                    false,
                ),
                plan_second_run(),
            ]
        );
    }

    /// Review N1b: the same with the stop of `check` lost.
    #[test]
    fn a_coordinator_led_flow_with_a_lost_stop_starts_again() {
        let p = project_with(COORDINATED, &[("plan", PLAN)]);
        let events = seq(&[
            &step(10, "a1", "dev", None),
            &[start(20, "a2", "qa")],
            &step(100, "b1", "dev", None),
            &step(110, "b2", "qa", None),
        ]);
        assert_eq!(
            runs(&p, &events),
            vec![
                run(
                    "plan",
                    vec![
                        relay("frame"),
                        done("build", "done"),
                        view("check", Mark::Running, None, None),
                    ],
                    vec![t(20, "build", "check", Some("done"))],
                    false,
                ),
                plan_second_run(),
            ]
        );
    }

    /// Two coordinator's steps in a row at the head: the run opens after both.
    #[test]
    fn a_flow_led_by_two_coordinator_steps_opens_after_them() {
        let p = project_with(
            COORDINATED,
            &[(
                "plan",
                "steps:\n  frame: { agent: lead }\n  note: { agent: lead }\n  build: { agent: dev }\n  check: { agent: qa }\nlinks:\n  - { from: frame, to: note }\n  - { from: note, to: build }\n  - { from: build, to: check }\n",
            )],
        );
        let events = seq(&[&step(10, "a1", "dev", None), &step(20, "a2", "qa", None)]);
        assert_eq!(
            runs(&p, &events),
            vec![run(
                "plan",
                vec![
                    relay("frame"),
                    relay("note"),
                    done("build", "done"),
                    done("check", "done"),
                ],
                vec![t(20, "build", "check", Some("done"))],
                true,
            )]
        );
    }

    /// Review N2c: a step after a coordinator's step in mid-flow opens no run.
    #[test]
    fn a_step_after_a_mid_flow_coordinator_step_opens_no_run() {
        let p = project_with(COORDINATED, &[("pass", PASS)]);
        assert_eq!(runs(&p, &step(30, "a3", "qa", None)), vec![]);
    }

    /// Review N2b: nor after a finished run.
    #[test]
    fn a_step_after_a_mid_flow_coordinator_step_adds_no_run_after_the_end() {
        let p = project_with(COORDINATED, &[("pass", PASS)]);
        let events = seq(&[
            &step(10, "a1", "dev", None),
            &step(20, "a2", "qa", None),
            &step(30, "a3", "qa", None),
        ]);
        assert_eq!(
            runs(&p, &events),
            vec![run(
                "pass",
                vec![done("build", "done"), relay("hand"), done("check", "done")],
                vec![
                    t(20, "build", "hand", Some("done")),
                    t(20, "hand", "check", None),
                ],
                true,
            )]
        );
    }

    /// Review N5: a fan-out through the coordinator crosses `f -> hub` once.
    #[test]
    fn a_fan_out_through_the_coordinator_crosses_it_once() {
        let p = project_with(
            COORDINATED,
            &[(
                "fan",
                "steps:\n  f: { agent: fan }\n  hub: { agent: lead }\n  x: { agent: b1 }\n  y: { agent: b2 }\n  join: { agent: docs }\nlinks:\n  - { from: f, to: hub }\n  - { from: hub, to: x }\n  - { from: hub, to: y }\n  - { from: x, to: join }\n  - { from: y, to: join }\n",
            )],
        );
        let events = seq(&[
            &step(10, "a1", "fan", None),
            &[
                start(20, "x1", "b1"),
                start(21, "y1", "b2"),
                stop(30, "y1", "b2", None),
                stop(31, "x1", "b1", None),
            ],
            &step(40, "j1", "docs", None),
        ]);
        assert_eq!(
            runs(&p, &events),
            vec![run(
                "fan",
                vec![
                    done("f", "done"),
                    relay("hub"),
                    done("x", "done"),
                    done("y", "done"),
                    done("join", "done"),
                ],
                vec![
                    t(20, "f", "hub", Some("done")),
                    t(20, "hub", "x", None),
                    t(21, "hub", "y", None),
                    t(40, "x", "join", Some("done")),
                ],
                true,
            )]
        );
    }

    /// A link to the coordinator counts as taken: it does not hold the run open.
    #[test]
    fn a_link_to_the_coordinator_does_not_block_the_end() {
        let p = project_with(
            COORDINATED,
            &[(
                "ask",
                "steps:\n  build: { agent: dev }\n  review: { agent: lead }\nlinks:\n  - { from: build, to: review }\n",
            )],
        );
        assert_eq!(
            runs(&p, &step(10, "a1", "dev", None)),
            vec![run(
                "ask",
                vec![done("build", "done"), relay("review")],
                vec![],
                true,
            )]
        );
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

    /// The `concept-to-code` flows: `deliver` and `guide` differ at `build` only.
    fn concept_to_code() -> Project {
        let flow = |builder: &str| {
            format!(
                "start: refine\nsteps:\n  refine: {{ agent: analyst }}\n  frame: {{ agent: framer }}\n  mockup: {{ agent: designer }}\n  design: {{ agent: architect }}\n  build: {{ agent: {builder} }}\n  guard: {{ agent: guardian, states: [approved, rejected] }}\n  validate: {{ agent: validator, states: [approved, rejected, scope_changed] }}\nlinks:\n  - {{ from: refine, to: frame }}\n  - {{ from: frame, to: mockup }}\n  - {{ from: mockup, to: design }}\n  - {{ from: design, to: build }}\n  - {{ from: build, to: guard }}\n  - {{ from: guard, to: build, when: rejected }}\n  - {{ from: guard, to: validate, when: approved }}\n  - {{ from: validate, to: build, when: rejected }}\n  - {{ from: validate, to: refine, when: scope_changed }}\n"
            )
        };
        project_with(
            COORDINATED,
            &[("deliver", &flow("implementer")), ("guide", &flow("coach"))],
        )
    }

    fn not_run(step: &str) -> StepView {
        view(step, Mark::NotRun, None, Some("not run in this session"))
    }

    /// A real session (2026-10-23): `/brief` done before, so the coordinator
    /// started at `frame`; framer ran 3 times and designer twice under one
    /// `agent_id` each, among internal agents with an empty type.
    fn real_session() -> Vec<FlowEvent> {
        include_str!("../../tests/fixtures/flow-status/mid-entry-session.jsonl")
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn a_step_mid_flow_with_no_run_open_opens_one_there() {
        let mut events = real_session();
        assert_eq!(events.len(), 27);
        let tail = events.split_off(25); // the architect's two stop lines
        let mid = run(
            "deliver | guide",
            vec![
                not_run("refine"),
                done("frame", "done"),
                done("mockup", "done"),
                view("design", Mark::Running, None, None),
                pending("build"),
                pending("guard"),
                pending("validate"),
            ],
            vec![
                t(1792747780, "frame", "mockup", Some("done")),
                t(1792748113, "mockup", "design", Some("done")),
            ],
            false,
        );
        assert_eq!(runs(&concept_to_code(), &events), vec![mid.clone()]);
        // the whole log: `design` done, `build` still to take
        events.extend(tail);
        let mut whole = mid;
        whole.steps[3] = done("design", "done");
        assert_eq!(runs(&concept_to_code(), &events), vec![whole]);
    }

    #[test]
    fn an_entry_started_after_a_mid_flow_run_opens_a_new_one() {
        let events = seq(&[
            &step(10, "a1", "framer", None),
            &step(20, "a2", "designer", None),
            &[start(30, "a3", "analyst")],
        ]);
        let first = run(
            "deliver | guide",
            vec![
                not_run("refine"),
                done("frame", "done"),
                done("mockup", "done"),
                pending("design"),
                pending("build"),
                pending("guard"),
                pending("validate"),
            ],
            vec![t(20, "frame", "mockup", Some("done"))],
            false,
        );
        let second = run(
            "deliver | guide",
            vec![
                view("refine", Mark::Running, None, None),
                pending("frame"),
                pending("mockup"),
                pending("design"),
                pending("build"),
                pending("guard"),
                pending("validate"),
            ],
            vec![],
            false,
        );
        assert_eq!(runs(&concept_to_code(), &events), vec![first, second]);
    }

    /// A sibling branch of a fan-out is not before the step: it stays pending.
    #[test]
    fn a_mid_flow_run_marks_only_the_steps_leading_to_its_first_step() {
        let p = project(&[(
            "spread",
            "steps:\n  f: { agent: fan }\n  x: { agent: b1 }\n  y: { agent: b2 }\nlinks:\n  - { from: f, to: x }\n  - { from: f, to: y }\n",
        )]);
        assert_eq!(
            runs(&p, &[start(10, "a1", "b2")])[0].steps,
            vec![
                not_run("f"),
                pending("x"),
                view("y", Mark::Running, None, None)
            ]
        );
    }

    /// One agent holding two steps of a flow with no run open: one run, both
    /// steps `?`, only the steps before both marked `-` (`mid` is before `c1`
    /// only).
    #[test]
    fn an_agent_holding_two_steps_of_a_flow_opens_one_run_ambiguous() {
        let p = project(&[(
            "twin",
            "steps:\n  a: { agent: architect }\n  c2: { agent: qa }\n  mid: { agent: dev }\n  c1: { agent: qa }\nlinks:\n  - { from: a, to: c2 }\n  - { from: c2, to: mid }\n  - { from: mid, to: c1 }\n",
        )]);
        let ambiguous = |s: &str| view(s, Mark::Unsure, None, Some("ambiguous step"));
        assert_eq!(
            runs(&p, &[start(10, "a1", "qa")]),
            vec![run(
                "twin",
                vec![
                    not_run("a"),
                    ambiguous("c2"),
                    pending("mid"),
                    ambiguous("c1")
                ],
                vec![],
                false,
            )]
        );
    }

    /// A step one flow's open run does not take opens no run of another flow.
    #[test]
    fn a_step_another_flow_ignored_opens_no_mid_flow_run() {
        let release = (
            "release",
            "steps:\n  prep: { agent: lead }\n  ship: { agent: ops }\nlinks:\n  - { from: prep, to: ship }\n",
        );
        let events = seq(&[&second_guard(), &[start(60, "a6", "ops")]]);
        let got = runs(&project(&[("deliver", DELIVER), release]), &events);
        assert_eq!(
            got.iter().map(|r| r.flow.as_str()).collect::<Vec<_>>(),
            ["deliver"]
        );
    }

    /// `triage` (hotfix) stops, then `design` (deliver): both lead to `dev`,
    /// the most recent link wins.
    #[test]
    fn a_shared_agent_two_links_allow_takes_the_most_recent() {
        let p = project(&[("deliver", DELIVER), ("hotfix", HOTFIX)]);
        let events = seq(&[
            &step(10, "l1", "lead", None),
            &step(12, "a1", "architect", None),
            &step(20, "d1", "dev", None),
        ]);
        let got = runs(&p, &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps[1].clone(), r.transitions.len()))
                .collect::<Vec<_>>(),
            vec![
                ("hotfix", view("patch", Mark::Pending, None, None), 0),
                ("deliver", done("build", "done"), 1),
            ]
        );
    }

    /// Both links' sources stopped in the same second: a tie, `?` in both.
    #[test]
    fn a_shared_agent_two_links_of_the_same_second_is_unsure_in_both() {
        let p = project(&[("deliver", DELIVER), ("hotfix", HOTFIX)]);
        let events = seq(&[
            &[start(10, "a1", "architect"), start(10, "l1", "lead")],
            &[
                stop(11, "a1", "architect", None),
                stop(11, "l1", "lead", None),
            ],
            &[start(20, "d1", "dev")],
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

    /// `patch` (hotfix) took its link from `triage` stopped at 11; `design`
    /// (deliver) stopped in that same second, after `patch` started: a tie
    /// with a link already taken elsewhere leaves the available link to win.
    #[test]
    fn a_link_as_recent_as_one_taken_elsewhere_is_still_taken() {
        let p = project(&[("deliver", DELIVER), ("hotfix", HOTFIX)]);
        let events = seq(&[
            &[start(10, "l1", "lead"), start(10, "a1", "architect")],
            &[stop(11, "l1", "lead", None), start(11, "d1", "dev")],
            &[
                stop(11, "a1", "architect", None),
                stop(12, "d1", "dev", None),
            ],
            &[start(20, "d2", "dev")],
        ]);
        let got = runs(&p, &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps[1].clone()))
                .collect::<Vec<_>>(),
            vec![
                ("hotfix", done("patch", "done")),
                ("deliver", view("build", Mark::Running, None, None)),
            ]
        );
    }

    /// Two candidates of one group (shared entry `e`) link to `q` from
    /// sources stopped at different times (`p` in `a`, `r` in `b`): the log
    /// does not tell which flow runs, both stay.
    #[test]
    fn candidates_of_one_group_are_not_split_by_their_link_times() {
        let flow = |src: &str| {
            format!(
                "steps:\n  e: {{ agent: ax }}\n  p: {{ agent: ay }}\n  r: {{ agent: az }}\n  q: {{ agent: qa }}\nlinks:\n  - {{ from: e, to: p }}\n  - {{ from: e, to: r }}\n  - {{ from: {src}, to: q }}\n"
            )
        };
        let p = project(&[("a", &flow("p")), ("b", &flow("r"))]);
        let events = seq(&[
            &step(90, "x1", "ax", None),
            &step(99, "y1", "ay", None),
            &step(104, "z1", "az", None),
            &[start(110, "q1", "qa")],
        ]);
        let got = runs(&p, &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps.last().unwrap().clone()))
                .collect::<Vec<_>>(),
            vec![("a | b", view("q", Mark::Running, None, None))]
        );
    }

    /// `a` and `b` (shared entry `e`) and `c` share `qa`.
    fn group_and_distinct_run() -> Project {
        let flow = |src: &str| {
            format!(
                "steps:\n  e: {{ agent: ax }}\n  p: {{ agent: ay }}\n  r: {{ agent: az }}\n  q: {{ agent: qa, states: [ok, ko] }}\n  s: {{ agent: qa }}\nlinks:\n  - {{ from: e, to: p }}\n  - {{ from: e, to: r }}\n  - {{ from: {src}, to: q }}\n  - {{ from: q, to: s, when: ko }}\n"
            )
        };
        let c = "steps:\n  g: { agent: b1 }\n  q: { agent: qa }\nlinks:\n  - { from: g, to: q }\n";
        project(&[("a", &flow("p")), ("b", &flow("r")), ("c", c)])
    }

    /// The group `a | b` takes `q` from `p` (99) in `a` and `r` (104) in
    /// `b`; `c`'s link from `g` (100) loses. The next `qa` does not take it:
    /// `b` already took a more recent one.
    #[test]
    fn a_link_older_than_the_latest_a_group_took_is_not_taken() {
        let events = seq(&[
            &step(90, "x1", "ax", None),
            &step(98, "y1", "ay", None),
            &step(99, "g1", "b1", None),
            &step(103, "z1", "az", None),
            &step(110, "q1", "qa", Some("ok")),
            &[start(120, "q2", "qa")],
        ]);
        let got = runs(&group_and_distinct_run(), &events);
        assert_eq!(
            got.iter()
                .map(|r| (
                    r.flow.as_str(),
                    (r.steps.iter()).find(|v| v.step == "q").unwrap().clone()
                ))
                .collect::<Vec<_>>(),
            vec![("a | b", done("q", "ok")), ("c", pending("q")),]
        );
    }

    /// `b` took `q` from `r` (104), `a` from `p` (99); `q` stops in that
    /// second: its link to `s`, as recent as the one `b` took, is taken.
    #[test]
    fn a_group_takes_a_link_as_recent_as_the_latest_it_took() {
        let events = seq(&[
            &step(90, "x1", "ax", None),
            &step(98, "y1", "ay", None),
            &step(103, "z1", "az", None),
            &[start(104, "q1", "qa"), stop(104, "q1", "qa", Some("ko"))],
            &[start(120, "s1", "qa")],
        ]);
        let got = runs(&group_and_distinct_run(), &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps.last().unwrap().clone()))
                .collect::<Vec<_>>(),
            vec![("a | b", view("s", Mark::Running, None, None))]
        );
    }

    /// A `?` step retried stops again later: its latest stop dates its link.
    #[test]
    fn a_retried_step_dates_its_link_by_its_latest_stop() {
        let check = (
            "check",
            "steps:\n  a: { agent: architect, states: [ok] }\n  b: { agent: dev }\nlinks:\n  - { from: a, to: b }\n",
        );
        let p = project(&[check, ("hotfix", HOTFIX)]);
        let events = seq(&[
            &step(10, "a1", "architect", None),
            &step(12, "l1", "lead", None),
            &step(14, "a2", "architect", Some("ok")),
            &[start(20, "d1", "dev")],
        ]);
        let got = runs(&p, &events);
        assert_eq!(
            got.iter()
                .map(|r| (r.flow.as_str(), r.steps[1].clone()))
                .collect::<Vec<_>>(),
            vec![
                ("check", view("b", Mark::Running, None, None)),
                ("hotfix", pending("patch")),
            ]
        );
    }

    /// A real session (2026-10-23, 164 lines): `guide` up to `guard`
    /// rejected and `build` again, then the user hands the rest to
    /// `implementer` (`deliver` only), whose `build` the next `guardian`
    /// follows: its link is the most recent. The last `guardian`, after
    /// `deliver` ended, follows `guide`'s link older than the one `deliver`
    /// already took: it is not shown.
    #[test]
    fn a_step_two_open_runs_link_to_goes_to_the_most_recent_link() {
        let events: Vec<FlowEvent> =
            include_str!("../../tests/fixtures/flow-status/recent-link-session.jsonl")
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
        assert_eq!(events.len(), 164);
        let guide = run(
            "guide",
            vec![
                not_run("refine"),
                done("frame", "done"),
                done("mockup", "done"),
                done("design", "done"),
                done("build", "done"),
                done("guard", "rejected"),
                pending("validate"),
            ],
            vec![
                t(1792747780, "frame", "mockup", Some("done")),
                t(1792748113, "mockup", "design", Some("done")),
                t(1792749634, "design", "build", Some("done")),
                t(1792751453, "build", "guard", Some("done")),
                t(1792751563, "guard", "build", Some("rejected")),
            ],
            false,
        );
        let deliver = run(
            "deliver",
            vec![
                not_run("refine"),
                not_run("frame"),
                not_run("mockup"),
                not_run("design"),
                done("build", "done"),
                done("guard", "approved"),
                done("validate", "approved"),
            ],
            vec![
                t(1792763656, "build", "guard", Some("done")),
                t(1792763710, "guard", "validate", Some("approved")),
            ],
            true,
        );
        assert_eq!(runs(&concept_to_code(), &events), vec![guide, deliver]);
    }

    #[test]
    fn upstream_is_what_leads_to_the_step_from_the_entry() {
        let p = concept_to_code();
        let flow = &p.flows["deliver"];
        // `validate` leads back to `refine` but comes after `build`
        assert_eq!(
            upstream(flow, "build"),
            ["design", "frame", "mockup", "refine"]
        );
        // the loop `validate -> refine` does not put the steps after `frame` before it
        assert_eq!(upstream(flow, "frame"), ["refine"]);
        assert_eq!(upstream(flow, "refine"), Vec::<&str>::new());
    }
}
