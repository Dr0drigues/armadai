//! Event log persistence for orchestration runs (OH1 Lot 1 socle).
//!
//! `EventLog` is the storage-agnostic trait; `InMemoryLog` is the always-on
//! implementation (used by tests and non-persistent runs). A SQL-backed
//! implementation lives bin-side under `armadai_storage` (behind the
//! `storage` feature), since `core` must stay storage-free.

use std::collections::HashMap;

use super::event::ExecutionEvent;

/// An append-only, per-run log of `ExecutionEvent`s.
pub trait EventLog {
    /// Append `event` to the log for `run_id`, preserving insertion order.
    fn append(&mut self, run_id: &str, event: &ExecutionEvent) -> anyhow::Result<()>;
    /// Return all events recorded for `run_id`, in append order. Returns an
    /// empty vec for an unknown `run_id` (not an error).
    fn events(&self, run_id: &str) -> anyhow::Result<Vec<ExecutionEvent>>;
}

/// In-memory `EventLog`, keyed by `run_id`. Always compiled (no feature
/// gate) — the default log for tests and non-persistent runs.
#[derive(Debug, Default)]
pub struct InMemoryLog {
    events: HashMap<String, Vec<ExecutionEvent>>,
}

impl EventLog for InMemoryLog {
    fn append(&mut self, run_id: &str, event: &ExecutionEvent) -> anyhow::Result<()> {
        self.events
            .entry(run_id.to_string())
            .or_default()
            .push(event.clone());
        Ok(())
    }

    fn events(&self, run_id: &str) -> anyhow::Result<Vec<ExecutionEvent>> {
        Ok(self.events.get(run_id).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::es::event::ExecutionEvent as E;

    fn sample() -> Vec<E> {
        vec![
            E::RunStarted {
                run_id: "r1".into(),
                pattern: "direct".into(),
                agents: vec!["a".into()],
                input: "x".into(),
                project: None,
                roster: Default::default(),
            },
            E::AgentObserved {
                agent: "a".into(),
                content: "hi".into(),
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
                model: "m".into(),
            },
            E::Completed {
                content: "hi".into(),
            },
        ]
    }

    #[test]
    fn in_memory_log_roundtrip_preserves_order() {
        let mut log = InMemoryLog::default();
        for e in sample() {
            log.append("r1", &e).unwrap();
        }
        let got = log.events("r1").unwrap();
        assert_eq!(got.len(), 3);
        assert!(matches!(got[0], E::RunStarted { .. }));
        assert!(matches!(got[2], E::Completed { .. }));
        assert!(log.events("absent").unwrap().is_empty());
    }

    /// Extract the `content` field of an `AgentObserved` event, panicking on
    /// any other variant. Used to check per-event identity/order in the
    /// multi-`run_id` isolation tests below.
    fn observed_content(event: &E) -> &str {
        match event {
            E::AgentObserved { content, .. } => content,
            other => panic!("expected AgentObserved, got {other:?}"),
        }
    }

    /// Build an `AgentObserved` event carrying `run_id` and `idx` in its
    /// `content`, so a mis-isolated log (e.g. a `seq`/PK collision between
    /// two `run_id`s) shows up as a wrong-content or leaked event rather
    /// than just a wrong count.
    fn marker(run_id: &str, idx: usize) -> E {
        E::AgentObserved {
            agent: "a".into(),
            content: format!("{run_id}-{idx}"),
            tokens_in: 1,
            tokens_out: 1,
            cost: 0.0,
            model: "m".into(),
        }
    }

    /// Shared assertion for both backends: interleave appends to two
    /// distinct `run_id`s (rA, rB, rA, rB, rA) and verify each `run_id`
    /// gets back exactly its own events, in insertion order, with none of
    /// the other's leaking in. This is the scenario that would break under
    /// a `seq`/`(run_id, seq)` collision.
    fn assert_multi_run_id_seq_isolation<L: EventLog>(mut log: L) {
        let run_ids = ["rA", "rB", "rA", "rB", "rA"];
        for (idx, run_id) in run_ids.iter().enumerate() {
            log.append(run_id, &marker(run_id, idx)).unwrap();
        }

        let a = log.events("rA").unwrap();
        let b = log.events("rB").unwrap();

        assert_eq!(a.len(), 3, "rA should have exactly its 3 own events");
        assert_eq!(b.len(), 2, "rB should have exactly its 2 own events");

        let a_contents: Vec<&str> = a.iter().map(observed_content).collect();
        assert_eq!(a_contents, vec!["rA-0", "rA-2", "rA-4"]);

        let b_contents: Vec<&str> = b.iter().map(observed_content).collect();
        assert_eq!(b_contents, vec!["rB-1", "rB-3"]);
    }

    #[test]
    fn in_memory_multi_run_id_seq_isolation() {
        assert_multi_run_id_seq_isolation(InMemoryLog::default());
    }
}
