//! Event log persistence for orchestration runs (OH1 Lot 1 socle).
//!
//! `EventLog` is the storage-agnostic trait; `InMemoryLog` is the always-on
//! implementation (used by tests and non-persistent runs), `SqliteLog` (gated
//! behind the `storage` feature) persists into the `execution_events` table
//! (schema v3, see `crate::storage::schema`).

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
/// (schema v3). Gated behind the `storage` feature.
#[cfg(feature = "storage")]
pub struct SqliteLog {
    db: crate::storage::Database,
}

#[cfg(feature = "storage")]
impl SqliteLog {
    /// Wrap an existing storage handle.
    pub fn new(db: crate::storage::Database) -> Self {
        Self { db }
    }
}

#[cfg(feature = "storage")]
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
    use crate::core::orchestration::es::event::ExecutionEvent as E;

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

    #[cfg(feature = "storage")]
    #[test]
    fn sqlite_log_roundtrip() {
        let db = crate::storage::init_embedded().unwrap();
        let mut log = SqliteLog::new(db);
        for e in sample() {
            log.append("r1", &e).unwrap();
        }
        let got = log.events("r1").unwrap();
        assert_eq!(got.len(), 3);
        assert!(matches!(got[2], E::Completed { .. }));
    }
}
