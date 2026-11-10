//! Durations, calls, verdicts and loops of a session, per agent, step or lot
//! (`armadai flow stats`). Pure: the project and the session log in.
//!
//! A call is an `agent_id` that started (a `stop` without a type or a start is
//! the main thread's and counts for nothing; one invocation writes two
//! `stop`s, which `merge_stops` folds). Its duration runs from its first
//! `start` to its first `stop`: a sub-agent resumed by `SendMessage` keeps its
//! id, counts once (`resumed` counts the restarts) and its later segments are
//! not timed, so durations are lower bounds. States are the ones the merged
//! `stop`s rendered, a resumed agent rendering one per segment.

use std::collections::{BTreeMap, HashMap};

use armadai_model::Project;

use super::record::{EventKind, FlowEvent, GROUP_SEP};
use super::runs::{merge_stops, runs_and_markers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Agent,
    Step,
    Lot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// The agent id, `flow/step`, or the lot (`(none)` for a marker without one).
    pub key: String,
    pub calls: u32,
    pub resumed: u32,
    /// Over the calls that stopped; 0 when none did.
    pub median_s: u64,
    pub max_s: u64,
    pub total_s: u64,
    pub states: BTreeMap<String, u32>,
    /// `rejected` or `rejected-*` over the states rendered; `None` when no
    /// step the group holds declares one.
    pub rejection_rate: Option<f64>,
    /// Links taken more than once: the sum of `times - 1`, by the step they leave.
    pub loops: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub by: By,
    pub groups: Vec<Group>,
    /// Calls left out of `--by step|lot` for lack of a marker; 0 for `--by agent`.
    pub unmarked_calls: u32,
    /// Whether any call of the session carries a marker.
    pub any_marked: bool,
}

pub const NO_LOT: &str = "(none)";

fn is_rejection(state: &str) -> bool {
    state == "rejected" || state.starts_with("rejected-")
}

#[derive(Default)]
struct Call {
    agent: String,
    started: u64,
    stopped: Option<u64>,
    starts: u32,
}

#[derive(Default)]
struct Acc {
    durations: Vec<u64>,
    calls: u32,
    resumed: u32,
    states: BTreeMap<String, u32>,
    can_reject: bool,
    loops: u32,
}

pub fn stats(project: &Project, events: &[FlowEvent], by: By) -> Stats {
    let mut calls: BTreeMap<&str, Call> = BTreeMap::new();
    for e in events.iter().filter(|e| !e.agent_id.is_empty()) {
        match e.event {
            EventKind::Start if !e.agent_type.is_empty() => {
                let c = calls.entry(&e.agent_id).or_insert_with(|| Call {
                    agent: e.agent_type.clone(),
                    started: e.ts,
                    ..Call::default()
                });
                c.starts += 1;
            }
            EventKind::Stop => {
                if let Some(c) = calls.get_mut(e.agent_id.as_str()) {
                    c.stopped.get_or_insert(e.ts);
                }
            }
            _ => {}
        }
    }
    let (runs, markers) = runs_and_markers(project, events);
    let declares_rejection = |flow: &str, step: &str| {
        (project.flows.get(flow))
            .and_then(|f| f.steps.get(step))
            .is_some_and(|s| s.states.iter().any(|w| is_rejection(w)))
    };
    let held_rejection = |agent: &str| {
        (project.flows.values())
            .flat_map(|f| f.steps.values())
            .any(|s| s.agent == agent && s.states.iter().any(|w| is_rejection(w)))
    };
    // id -> its group key and whether its step can reject; `None`: left out
    let key_of = |id: &str, c: &Call| -> Option<(String, bool)> {
        match by {
            By::Agent => Some((c.agent.clone(), held_rejection(&c.agent))),
            By::Step | By::Lot => {
                let (flow, step, lot) = markers.get(id)?;
                let key = match by {
                    By::Step => format!("{flow}/{step}"),
                    _ => lot.clone().unwrap_or_else(|| NO_LOT.to_string()),
                };
                Some((key, declares_rejection(flow, step)))
            }
        }
    };
    let mut groups: BTreeMap<String, Acc> = BTreeMap::new();
    let mut key_for: HashMap<&str, String> = HashMap::new();
    let mut unmarked_calls = 0;
    for (id, c) in &calls {
        let Some((key, can_reject)) = key_of(id, c) else {
            unmarked_calls += 1;
            continue;
        };
        let g = groups.entry(key.clone()).or_default();
        g.calls += 1;
        g.resumed += c.starts - 1;
        g.can_reject |= can_reject;
        g.durations
            .extend(c.stopped.map(|s| s.saturating_sub(c.started)));
        key_for.insert(id, key);
    }
    for e in merge_stops(events) {
        let (EventKind::Stop, Some(state)) = (&e.event, &e.state) else {
            continue;
        };
        if let Some(g) = key_for
            .get(e.agent_id.as_str())
            .and_then(|k| groups.get_mut(k))
        {
            *g.states.entry(state.clone()).or_default() += 1;
        }
    }
    for r in &runs {
        let flow = r.flow.split(GROUP_SEP).next().unwrap_or_default();
        for (from, _, times) in &r.loops {
            let agent = (project.flows.get(flow))
                .and_then(|f| f.steps.get(from))
                .map(|s| s.agent.as_str());
            let key = match by {
                By::Agent => agent.map(str::to_string),
                By::Step => Some(format!("{flow}/{from}")),
                By::Lot => Some(r.lot.clone().unwrap_or_else(|| NO_LOT.to_string())),
            };
            if let Some(g) = key.and_then(|k| groups.get_mut(&k)) {
                g.loops += times - 1;
            }
        }
    }
    let groups = groups
        .into_iter()
        .map(|(key, mut a)| {
            a.durations.sort_unstable();
            let n = a.durations.len();
            let median_s = match n {
                0 => 0,
                _ if n % 2 == 1 => a.durations[n / 2],
                _ => (a.durations[n / 2 - 1] + a.durations[n / 2]) / 2,
            };
            let rendered: u32 = a.states.values().sum();
            let rejected: u32 = (a.states.iter())
                .filter(|(w, _)| is_rejection(w))
                .map(|(_, n)| n)
                .sum();
            Group {
                key,
                calls: a.calls,
                resumed: a.resumed,
                median_s,
                max_s: a.durations.last().copied().unwrap_or(0),
                total_s: a.durations.iter().sum(),
                states: a.states,
                rejection_rate: (a.can_reject && rendered > 0)
                    .then(|| f64::from(rejected) / f64::from(rendered)),
                loops: a.loops,
            }
        })
        .collect();
    Stats {
        by,
        groups,
        unmarked_calls,
        any_marked: !markers.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELIVER: &str = "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  guard: { agent: qa, states: [approved, rejected, rejected-minor] }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: guard }\n  - { from: guard, to: build, when: rejected }\n";

    fn project() -> Project {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(".armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), "schema_version: 1\n").unwrap();
        for agent in ["architect", "dev", "qa"] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(a.join("flows/deliver.yaml"), DELIVER).unwrap();
        armadai_model::load_checked(dir.path()).unwrap().0
    }

    fn ev(ts: u64, kind: EventKind, id: &str, agent: &str, state: Option<&str>) -> FlowEvent {
        FlowEvent {
            ts,
            event: kind,
            agent_id: id.into(),
            agent_type: agent.into(),
            state: state.map(str::to_string),
            command: None,
            flow: None,
            step: None,
            lot: None,
            reference: None,
        }
    }

    fn start(ts: u64, id: &str, agent: &str) -> FlowEvent {
        ev(ts, EventKind::Start, id, agent, None)
    }

    /// One invocation's two `stop`s: the state on the first, none on the second.
    fn stops(ts: u64, id: &str, agent: &str, state: Option<&str>) -> [FlowEvent; 2] {
        [
            ev(ts, EventKind::Stop, id, agent, state),
            ev(ts + 1, EventKind::Stop, id, agent, None),
        ]
    }

    fn mark(ts: u64, agent: &str, step: &str, lot: Option<&str>) -> FlowEvent {
        FlowEvent {
            flow: Some("deliver".into()),
            step: Some(step.into()),
            lot: lot.map(str::to_string),
            ..ev(ts, EventKind::Mark, "", agent, None)
        }
    }

    fn group<'a>(s: &'a Stats, key: &str) -> &'a Group {
        s.groups.iter().find(|g| g.key == key).expect(key)
    }

    /// design, build, guard rejects, build again, guard approves: durations
    /// 20 / 30 and 45 / 12 / 41, distinct two by two; two invocations of an
    /// agent that no step holds stay out of the steps.
    fn one_loop() -> Vec<FlowEvent> {
        let mut e = vec![start(1000, "a1", "architect")];
        e.extend(stops(1020, "a1", "architect", Some("done")));
        e.push(start(1100, "d1", "dev"));
        e.extend(stops(1130, "d1", "dev", Some("done")));
        e.push(start(1200, "q1", "qa"));
        e.extend(stops(1212, "q1", "qa", Some("rejected")));
        e.push(start(1300, "d2", "dev"));
        e.extend(stops(1345, "d2", "dev", Some("done")));
        e.push(start(1400, "q2", "qa"));
        e.extend(stops(1441, "q2", "qa", Some("approved")));
        // the main thread's stop: never started, no type
        e.push(ev(1500, EventKind::Stop, "main", "", None));
        e
    }

    #[test]
    fn by_agent_counts_starts_and_floors_an_even_median() {
        let s = stats(&project(), &one_loop(), By::Agent);
        assert_eq!(s.by, By::Agent);
        assert_eq!(s.unmarked_calls, 0);
        assert!(!s.any_marked);
        let keys: Vec<&str> = s.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["architect", "dev", "qa"]);
        let dev = group(&s, "dev");
        // 30 and 45: 37.5 rounds down; two starts, not four stops
        assert_eq!(
            (dev.calls, dev.resumed, dev.median_s, dev.max_s, dev.total_s),
            (2, 0, 37, 45, 75)
        );
        // build -> guard was taken twice: one loop, attributed to its source
        assert_eq!(dev.loops, 1);
        assert_eq!(dev.rejection_rate, None);
        let qa = group(&s, "qa");
        assert_eq!(
            (qa.calls, qa.median_s, qa.max_s, qa.total_s, qa.loops),
            (2, 26, 41, 53, 0)
        );
        assert_eq!(
            qa.states,
            BTreeMap::from([("approved".to_string(), 1), ("rejected".to_string(), 1)])
        );
        assert_eq!(qa.rejection_rate, Some(0.5));
        let a = group(&s, "architect");
        assert_eq!((a.calls, a.median_s, a.max_s, a.total_s), (1, 20, 20, 20));
        assert_eq!(
            a.rejection_rate, None,
            "no step of architect declares a rejection"
        );
    }

    #[test]
    fn a_resume_counts_once_and_its_segments_give_states_not_time() {
        let mut e = vec![start(100, "q1", "qa")];
        e.extend(stops(112, "q1", "qa", Some("rejected-minor")));
        e.push(start(200, "q2", "qa"));
        e.extend(stops(240, "q2", "qa", Some("approved")));
        // q1 resumed twice: a second and a third segment, 50 s and 5 s
        e.push(start(300, "q1", "qa"));
        e.extend(stops(350, "q1", "qa", Some("approved")));
        e.push(start(400, "q1", "qa"));
        e.extend(stops(405, "q1", "qa", Some("approved")));
        e.push(start(500, "q3", "qa")); // never stopped
        let s = stats(&project(), &e, By::Agent);
        let qa = group(&s, "qa");
        assert_eq!((qa.calls, qa.resumed), (3, 2));
        // first start to first stop: 12 and 40; q3 has no duration
        assert_eq!((qa.median_s, qa.max_s, qa.total_s), (26, 40, 52));
        assert_eq!(
            qa.states,
            BTreeMap::from([
                ("approved".to_string(), 3),
                ("rejected-minor".to_string(), 1)
            ])
        );
        assert_eq!(qa.rejection_rate, Some(0.25));
    }

    #[test]
    fn an_odd_number_of_calls_takes_the_middle_one() {
        let mut e = Vec::new();
        for (i, (t0, d)) in [(100, 70), (300, 30), (500, 45)].into_iter().enumerate() {
            let id = format!("d{i}");
            e.push(start(t0, &id, "dev"));
            e.extend(stops(t0 + d, &id, "dev", None));
        }
        let dev = &stats(&project(), &e, By::Agent).groups[0];
        assert_eq!((dev.median_s, dev.max_s, dev.total_s), (45, 70, 145));
        assert!(dev.states.is_empty());
    }

    /// Two lots interleaved, a call with no marker.
    fn marked() -> Vec<FlowEvent> {
        let mut e = vec![mark(10, "dev", "build", Some("12"))];
        e.push(start(11, "d1", "dev"));
        e.extend(stops(41, "d1", "dev", Some("done")));
        e.push(mark(50, "dev", "build", Some("7")));
        e.push(start(51, "d2", "dev"));
        e.extend(stops(66, "d2", "dev", Some("done")));
        e.push(mark(70, "qa", "guard", Some("12")));
        e.push(start(71, "q1", "qa"));
        e.extend(stops(83, "q1", "qa", Some("rejected")));
        e.push(mark(90, "qa", "guard", None));
        e.push(start(91, "q2", "qa"));
        e.extend(stops(101, "q2", "qa", Some("approved")));
        e.push(start(110, "a1", "architect"));
        e.extend(stops(125, "a1", "architect", Some("done")));
        e
    }

    #[test]
    fn by_step_and_by_lot_count_marked_starts_and_set_the_rest_aside() {
        let p = project();
        let s = stats(&p, &marked(), By::Step);
        assert!(s.any_marked);
        assert_eq!(s.unmarked_calls, 1);
        let keys: Vec<&str> = s.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["deliver/build", "deliver/guard"]);
        let build = group(&s, "deliver/build");
        assert_eq!(
            (build.calls, build.median_s, build.max_s, build.total_s),
            (2, 22, 30, 45)
        );
        let guard = group(&s, "deliver/guard");
        assert_eq!((guard.calls, guard.median_s, guard.total_s), (2, 11, 22));
        assert_eq!(guard.rejection_rate, Some(0.5));
        assert_eq!(build.rejection_rate, None);

        let s = stats(&p, &marked(), By::Lot);
        assert_eq!(s.unmarked_calls, 1);
        let keys: Vec<&str> = s.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["(none)", "12", "7"]);
        let (none, l12, l7) = (group(&s, NO_LOT), group(&s, "12"), group(&s, "7"));
        assert_eq!(
            (none.calls, none.total_s, none.rejection_rate),
            (1, 10, Some(0.0))
        );
        assert_eq!(
            (l12.calls, l12.total_s, l12.rejection_rate),
            (2, 42, Some(0.5))
        );
        assert_eq!((l7.calls, l7.total_s, l7.rejection_rate), (1, 15, None));
    }

    #[test]
    fn without_a_marker_nothing_is_grouped_by_step_or_lot() {
        for by in [By::Step, By::Lot] {
            let s = stats(&project(), &one_loop(), by);
            assert!(s.groups.is_empty());
            assert!(!s.any_marked);
            assert_eq!(s.unmarked_calls, 5);
        }
    }

    #[test]
    fn loops_sum_the_extra_times_and_a_state_on_both_stops_counts_once() {
        let mut e = Vec::new();
        for (i, verdict) in ["rejected", "rejected", "approved"].into_iter().enumerate() {
            let t = 1000 * (i as u64 + 1);
            let (d, q) = (format!("d{i}"), format!("q{i}"));
            e.push(start(t, &d, "dev"));
            e.extend(stops(t + 10, &d, "dev", None));
            e.push(start(t + 100, &q, "qa"));
            // both stops of the invocation say the state
            e.push(ev(t + 120, EventKind::Stop, &q, "qa", Some(verdict)));
            e.push(ev(t + 121, EventKind::Stop, &q, "qa", Some(verdict)));
        }
        let s = stats(&project(), &e, By::Agent);
        // build -> guard three times, guard -> build twice
        assert_eq!((group(&s, "dev").loops, group(&s, "qa").loops), (2, 1));
        let qa = group(&s, "qa");
        assert_eq!(qa.states.values().sum::<u32>(), 3);
        assert_eq!(qa.rejection_rate, Some(2.0 / 3.0));
    }

    #[test]
    fn no_events_no_groups() {
        let s = stats(&project(), &[], By::Agent);
        assert_eq!(
            (s.groups.len(), s.unmarked_calls, s.any_marked),
            (0, 0, false)
        );
    }
}
