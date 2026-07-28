//! SQLite-backed `EventLog` implementation (OH7 Lot 1 Task 1e).
//!
//! `SqliteLog` persists into the `execution_events` table (schema v3, see
//! `armadai_storage::schema`). It lives bin-side (not in `core`) because it
//! depends on `rusqlite` and `armadai_storage::Database` — `core` only owns
//! the storage-agnostic `EventLog` trait and the always-on `InMemoryLog`
//! (see `armadai_core::orchestration::es::log`).

use armadai_core::orchestration::es::event::ExecutionEvent;
use armadai_core::orchestration::es::log::EventLog;

/// Extract the internal serde tag (`t`, e.g. `"run_started"`) from an
/// `ExecutionEvent`'s serialized form, used as the `kind` column value.
fn event_kind(event: &ExecutionEvent) -> anyhow::Result<String> {
    let value = serde_json::to_value(event)?;
    value
        .get("t")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("ExecutionEvent serialized without a `t` tag"))
}

/// SQLite-backed `EventLog`, persisting into `execution_events`
/// (schema v3).
pub struct SqliteLog {
    db: armadai_storage::Database,
}

impl SqliteLog {
    /// Wrap an existing storage handle.
    pub fn new(db: armadai_storage::Database) -> Self {
        Self { db }
    }
}

impl EventLog for SqliteLog {
    fn append(&mut self, run_id: &str, event: &ExecutionEvent) -> anyhow::Result<()> {
        let kind = event_kind(event)?;
        let payload_json = serde_json::to_string(event)?;
        let conn = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
        let seq: i64 = conn.query_row(
            "SELECT COUNT(*) FROM execution_events WHERE run_id = ?1",
            rusqlite::params![run_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO execution_events (run_id, seq, kind, payload_json) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![run_id, seq, kind, payload_json],
        )?;
        Ok(())
    }

    fn events(&self, run_id: &str) -> anyhow::Result<Vec<ExecutionEvent>> {
        let conn = self
            .db
            .lock()
            .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT payload_json FROM execution_events WHERE run_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![run_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armadai_core::orchestration::es::event::ExecutionEvent as E;

    fn sample() -> Vec<E> {
        vec![
            E::RunStarted {
                run_id: "r1".into(),
                pattern: "direct".into(),
                agents: vec!["a".into()],
                input: "x".into(),
                project: None,
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
    fn sqlite_log_roundtrip() {
        let db = armadai_storage::open_in_memory().unwrap();
        let mut log = SqliteLog::new(db);
        for e in sample() {
            log.append("r1", &e).unwrap();
        }
        let got = log.events("r1").unwrap();
        assert_eq!(got.len(), 3);
        // Assert the full ordering, not just the last element — this is the
        // property that would break if `seq` were computed wrong (e.g.
        // reversed or unstable ORDER BY).
        assert!(matches!(got[0], E::RunStarted { .. }));
        assert!(matches!(got[1], E::AgentObserved { .. }));
        assert!(matches!(got[2], E::Completed { .. }));
    }

    /// Extract the `content` field of an `AgentObserved` event, panicking on
    /// any other variant. Used to check per-event identity/order in the
    /// multi-`run_id` isolation test below.
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

    /// Interleave appends to two distinct `run_id`s (rA, rB, rA, rB, rA) and
    /// verify each `run_id` gets back exactly its own events, in insertion
    /// order, with none of the other's leaking in. This is the scenario
    /// that would break under a `seq`/`(run_id, seq)` collision. (Mirrors
    /// `armadai_core::orchestration::es::log::tests::assert_multi_run_id_seq_isolation`
    /// for `InMemoryLog` — duplicated here since `SqliteLog` now lives in a
    /// different crate/module and that helper is private to core's tests.)
    #[test]
    fn sqlite_multi_run_id_seq_isolation() {
        let db = armadai_storage::open_in_memory().unwrap();
        let mut log = SqliteLog::new(db);
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
}
