use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub agent: String,
    pub input: String,
    pub output: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub duration_ms: i64,
    pub status: String,
    /// Project root path the run was executed from (display string), or
    /// `None` when there was no project config (default/global agent run).
    pub project: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    pub agent: String,
    pub total_runs: i64,
    pub total_cost: f64,
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
}

/// Insert a new execution record.
pub fn insert_run(db: &Database, run: RunRecord) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    insert_run_with_id(db, &id, run)
}

/// Insert an execution record with a caller-supplied id (used by orchestration
/// to share the same id across the parent `runs` row and child tables).
pub fn insert_run_with_id(db: &Database, id: &str, run: RunRecord) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO runs (id, agent, input, output, provider, model, tokens_in, tokens_out, cost, duration_ms, status, project)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![id, run.agent, run.input, run.output, run.provider, run.model,
                run.tokens_in, run.tokens_out, run.cost, run.duration_ms, run.status, run.project],
    )?;
    Ok(())
}

/// Get execution history, optionally filtered by agent name.
pub fn get_history(
    db: &Database,
    agent: Option<&str>,
    limit: u32,
) -> anyhow::Result<Vec<RunRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut records = Vec::new();

    match agent {
        Some(name) => {
            let mut stmt = conn.prepare(
                "SELECT agent, input, output, provider, model, tokens_in, tokens_out, cost, duration_ms, status, project
                 FROM runs WHERE agent = ?1 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![name, limit], |row| {
                Ok(RunRecord {
                    agent: row.get(0)?,
                    input: row.get(1)?,
                    output: row.get(2)?,
                    provider: row.get(3)?,
                    model: row.get(4)?,
                    tokens_in: row.get(5)?,
                    tokens_out: row.get(6)?,
                    cost: row.get(7)?,
                    duration_ms: row.get(8)?,
                    status: row.get(9)?,
                    project: row.get(10)?,
                })
            })?;
            for row in rows {
                records.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT agent, input, output, provider, model, tokens_in, tokens_out, cost, duration_ms, status, project
                 FROM runs ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(RunRecord {
                    agent: row.get(0)?,
                    input: row.get(1)?,
                    output: row.get(2)?,
                    provider: row.get(3)?,
                    model: row.get(4)?,
                    tokens_in: row.get(5)?,
                    tokens_out: row.get(6)?,
                    cost: row.get(7)?,
                    duration_ms: row.get(8)?,
                    status: row.get(9)?,
                    project: row.get(10)?,
                })
            })?;
            for row in rows {
                records.push(row?);
            }
        }
    }

    Ok(records)
}

/// Get cost summary grouped by agent.
pub fn get_costs_summary(
    db: &Database,
    agent_filter: Option<&str>,
) -> anyhow::Result<Vec<CostSummary>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut summaries = Vec::new();

    match agent_filter {
        Some(name) => {
            let mut stmt = conn.prepare(
                "SELECT agent, COUNT(*) AS total_runs, SUM(cost) AS total_cost,
                        SUM(tokens_in) AS total_tokens_in, SUM(tokens_out) AS total_tokens_out
                 FROM runs WHERE agent = ?1 GROUP BY agent",
            )?;
            let rows = stmt.query_map(params![name], |row| {
                Ok(CostSummary {
                    agent: row.get(0)?,
                    total_runs: row.get(1)?,
                    total_cost: row.get(2)?,
                    total_tokens_in: row.get(3)?,
                    total_tokens_out: row.get(4)?,
                })
            })?;
            for row in rows {
                summaries.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT agent, COUNT(*) AS total_runs, SUM(cost) AS total_cost,
                        SUM(tokens_in) AS total_tokens_in, SUM(tokens_out) AS total_tokens_out
                 FROM runs GROUP BY agent ORDER BY total_cost DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(CostSummary {
                    agent: row.get(0)?,
                    total_runs: row.get(1)?,
                    total_cost: row.get(2)?,
                    total_tokens_in: row.get(3)?,
                    total_tokens_out: row.get(4)?,
                })
            })?;
            for row in rows {
                summaries.push(row?);
            }
        }
    }

    Ok(summaries)
}

// ── Orchestration queries ────────────────────────────────────────

/// Record for an orchestration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationRunRecord {
    pub run_id: String,
    pub pattern: String,
    pub config_json: String,
    pub outcome_json: Option<String>,
    pub rounds: i64,
    pub halt_reason: Option<String>,
    pub parent_run_id: Option<String>,
}

/// Record for a board entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardEntryRecord {
    pub run_id: String,
    pub agent: String,
    pub round: i64,
    pub kind: String,
    pub content: String,
    pub refs_json: String,
    pub confidence: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

/// Record for a ring contribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingContributionRecord {
    pub run_id: String,
    pub agent: String,
    pub lap: i64,
    pub position_in_lap: i64,
    pub action: String,
    pub content: String,
    pub reactions_json: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
}

/// Record for a ring vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingVoteRecord {
    pub run_id: String,
    pub agent: String,
    pub position: String,
    pub confidence: f64,
    pub supports: String,
    pub concerns: String,
}

/// Record for a hierarchical delegation event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationEventRecord {
    pub run_id: String,
    pub seq: i64,
    pub from_agent: String,
    pub to_agent: String,
    pub message: String,
    pub depth: i64,
}

/// Insert an orchestration run record (finished_at populated automatically).
pub fn insert_orchestration_run(
    db: &Database,
    record: OrchestrationRunRecord,
) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO orchestration_runs (run_id, pattern, config_json, outcome_json, rounds, halt_reason, parent_run_id, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
        params![
            record.run_id,
            record.pattern,
            record.config_json,
            record.outcome_json,
            record.rounds,
            record.halt_reason,
            record.parent_run_id
        ],
    )?;
    Ok(())
}

/// Insert a board entry record.
pub fn insert_board_entry(db: &Database, record: BoardEntryRecord) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO board_entries (run_id, agent, round, kind, content, refs_json, confidence, tokens_in, tokens_out)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.run_id,
            record.agent,
            record.round,
            record.kind,
            record.content,
            record.refs_json,
            record.confidence,
            record.tokens_in,
            record.tokens_out
        ],
    )?;
    Ok(())
}

/// Insert a ring contribution record.
pub fn insert_ring_contribution(
    db: &Database,
    record: RingContributionRecord,
) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO ring_contributions (run_id, agent, lap, position_in_lap, action, content, reactions_json, tokens_in, tokens_out)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.run_id,
            record.agent,
            record.lap,
            record.position_in_lap,
            record.action,
            record.content,
            record.reactions_json,
            record.tokens_in,
            record.tokens_out
        ],
    )?;
    Ok(())
}

/// Insert a ring vote record.
pub fn insert_ring_vote(db: &Database, record: RingVoteRecord) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO ring_votes (run_id, agent, position, confidence, supports, concerns)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            record.run_id,
            record.agent,
            record.position,
            record.confidence,
            record.supports,
            record.concerns
        ],
    )?;
    Ok(())
}

/// Get orchestration run details by run_id.
#[allow(dead_code)] // API reserved for future `armadai history` / web UI
pub fn get_orchestration_run(
    db: &Database,
    run_id: &str,
) -> anyhow::Result<Option<OrchestrationRunRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason, parent_run_id
         FROM orchestration_runs WHERE run_id = ?1",
    )?;
    let mut rows = stmt.query_map(params![run_id], |row| {
        Ok(OrchestrationRunRecord {
            run_id: row.get(0)?,
            pattern: row.get(1)?,
            config_json: row.get(2)?,
            outcome_json: row.get(3)?,
            rounds: row.get(4)?,
            halt_reason: row.get(5)?,
            parent_run_id: row.get(6)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Get orchestration runs list (most recent first).
#[allow(dead_code)] // API reserved for TUI / web UI
pub fn get_orchestration_runs(
    db: &Database,
    limit: u32,
) -> anyhow::Result<Vec<OrchestrationRunRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason, parent_run_id
         FROM orchestration_runs ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(OrchestrationRunRecord {
            run_id: row.get(0)?,
            pattern: row.get(1)?,
            config_json: row.get(2)?,
            outcome_json: row.get(3)?,
            rounds: row.get(4)?,
            halt_reason: row.get(5)?,
            parent_run_id: row.get(6)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// Get root orchestration runs list (most recent first), excluding nested
/// children (`parent_run_id IS NOT NULL`).
///
/// Filters at the SQL level rather than in Rust so `limit` bounds the number
/// of *roots* returned — filtering a `get_orchestration_runs` result in Rust
/// after the fact would let nested children consume slots in the initial
/// `LIMIT`, silently returning fewer than `limit` roots whenever a run in the
/// window has children (see C6 trace list pagination).
#[allow(dead_code)] // sole caller (get_orchestration_trace) is gated behind `web`
pub fn get_root_orchestration_runs(
    db: &Database,
    limit: u32,
) -> anyhow::Result<Vec<OrchestrationRunRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason, parent_run_id
         FROM orchestration_runs WHERE parent_run_id IS NULL ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(OrchestrationRunRecord {
            run_id: row.get(0)?,
            pattern: row.get(1)?,
            config_json: row.get(2)?,
            outcome_json: row.get(3)?,
            rounds: row.get(4)?,
            halt_reason: row.get(5)?,
            parent_run_id: row.get(6)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// Get board entries for a run.
#[allow(dead_code)] // API reserved for future `armadai history` / web UI
pub fn get_board_entries(db: &Database, run_id: &str) -> anyhow::Result<Vec<BoardEntryRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, agent, round, kind, content, refs_json, confidence, tokens_in, tokens_out
         FROM board_entries WHERE run_id = ?1 ORDER BY round, id",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(BoardEntryRecord {
            run_id: row.get(0)?,
            agent: row.get(1)?,
            round: row.get(2)?,
            kind: row.get(3)?,
            content: row.get(4)?,
            refs_json: row.get(5)?,
            confidence: row.get(6)?,
            tokens_in: row.get(7)?,
            tokens_out: row.get(8)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// Get ring contributions for a run.
#[allow(dead_code)] // API reserved for future `armadai history` / web UI
pub fn get_ring_contributions(
    db: &Database,
    run_id: &str,
) -> anyhow::Result<Vec<RingContributionRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, agent, lap, position_in_lap, action, content, reactions_json, tokens_in, tokens_out
         FROM ring_contributions WHERE run_id = ?1 ORDER BY lap, position_in_lap",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(RingContributionRecord {
            run_id: row.get(0)?,
            agent: row.get(1)?,
            lap: row.get(2)?,
            position_in_lap: row.get(3)?,
            action: row.get(4)?,
            content: row.get(5)?,
            reactions_json: row.get(6)?,
            tokens_in: row.get(7)?,
            tokens_out: row.get(8)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// Get ring votes for a run.
#[allow(dead_code)] // API reserved for future `armadai history` / web UI
pub fn get_ring_votes(db: &Database, run_id: &str) -> anyhow::Result<Vec<RingVoteRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, agent, position, confidence, supports, concerns
         FROM ring_votes WHERE run_id = ?1",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(RingVoteRecord {
            run_id: row.get(0)?,
            agent: row.get(1)?,
            position: row.get(2)?,
            confidence: row.get(3)?,
            supports: row.get(4)?,
            concerns: row.get(5)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

/// Insert a delegation event.
pub fn insert_delegation_event(db: &Database, record: DelegationEventRecord) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO delegation_events (run_id, seq, from_agent, to_agent, message, depth)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            record.run_id,
            record.seq,
            record.from_agent,
            record.to_agent,
            record.message,
            record.depth
        ],
    )?;
    Ok(())
}

/// Get delegation events for a run, ordered by sequence.
#[allow(dead_code)] // consumed by web/TUI trace UI (Lot 3)
pub fn get_delegation_events(
    db: &Database,
    run_id: &str,
) -> anyhow::Result<Vec<DelegationEventRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, seq, from_agent, to_agent, message, depth
         FROM delegation_events WHERE run_id = ?1 ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(params![run_id], |row| {
        Ok(DelegationEventRecord {
            run_id: row.get(0)?,
            seq: row.get(1)?,
            from_agent: row.get(2)?,
            to_agent: row.get(3)?,
            message: row.get(4)?,
            depth: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Get the child orchestration runs of a parent (nested sub-runs).
#[allow(dead_code)] // consumed by web/TUI trace UI (Lot 3)
pub fn get_child_orchestration_runs(
    db: &Database,
    parent_run_id: &str,
) -> anyhow::Result<Vec<OrchestrationRunRecord>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason, parent_run_id
         FROM orchestration_runs WHERE parent_run_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![parent_run_id], |row| {
        Ok(OrchestrationRunRecord {
            run_id: row.get(0)?,
            pattern: row.get(1)?,
            config_json: row.get(2)?,
            outcome_json: row.get(3)?,
            rounds: row.get(4)?,
            halt_reason: row.get(5)?,
            parent_run_id: row.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Delete all projection rows for a given `run_id` across all orchestration
/// tables. Used by the idempotent projector (`project_run`) to clear any
/// existing projection before re-deriving it from the event log. Returns the
/// total number of rows deleted across all tables.
///
/// Tables cleared (in dependency order):
/// - `delegation_events` (hierarchical child table)
/// - `ring_votes` (ring child table)
/// - `ring_contributions` (ring child table)
/// - `board_entries` (blackboard child table)
/// - `orchestration_runs` (parent metadata table)
/// - `runs` (top-level parent row)
///
/// DELETEs are non-failing: removing a `run_id` that doesn't exist in a given
/// table simply returns `0` rows affected, not an error.
#[allow(dead_code)] // Called by project_run, wired in Task 3
pub fn delete_projection_for_run(db: &Database, run_id: &str) -> anyhow::Result<usize> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;

    let mut total_deleted = 0;

    // Child tables first (no FK enforcement in schema, but logical order).
    total_deleted += conn.execute(
        "DELETE FROM delegation_events WHERE run_id = ?1",
        params![run_id],
    )?;
    total_deleted += conn.execute("DELETE FROM ring_votes WHERE run_id = ?1", params![run_id])?;
    total_deleted += conn.execute(
        "DELETE FROM ring_contributions WHERE run_id = ?1",
        params![run_id],
    )?;
    total_deleted += conn.execute(
        "DELETE FROM board_entries WHERE run_id = ?1",
        params![run_id],
    )?;

    // Parent metadata.
    total_deleted += conn.execute(
        "DELETE FROM orchestration_runs WHERE run_id = ?1",
        params![run_id],
    )?;

    // Top-level parent row.
    total_deleted += conn.execute("DELETE FROM runs WHERE id = ?1", params![run_id])?;

    Ok(total_deleted)
}

/// Fetch the stored `pattern` (`"direct"`/`"blackboard"`/`"ring"`/
/// `"hierarchical"`) for `run_id` from `orchestration_runs`, or `None` if no
/// row exists there.
///
/// **Known gap**: `"direct"` runs never get an `orchestration_runs` row (see
/// `cli::run_es_record::project_run`'s own `"direct" => {}` no-op arm —
/// direct runs have no orchestration metadata to project), so this returns
/// `None` for them even though the run itself is real and present in the
/// event log. Callers that need the pattern for EVERY run (e.g.
/// `armadai run --resume`) must fall back to the event-sourced
/// `ExecutionState::pattern` (folded from the log's own `RunStarted` event
/// via [`crate::core::orchestration::es::engine::replay`]), which is always
/// populated regardless of pattern.
pub fn get_run_pattern(db: &Database, run_id: &str) -> anyhow::Result<Option<String>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare("SELECT pattern FROM orchestration_runs WHERE run_id = ?1")?;
    let mut rows = stmt.query_map(params![run_id], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Get all distinct run_ids present in the execution_events log.
///
/// Returns run IDs in sorted order. Used by `armadai projections rebuild --all`
/// to enumerate all runs that can be re-projected from the event log.
#[allow(dead_code)] // Called by projections rebuild --all
pub fn all_event_log_run_ids(db: &Database) -> anyhow::Result<Vec<String>> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    let mut stmt = conn.prepare("SELECT DISTINCT run_id FROM execution_events ORDER BY run_id")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut run_ids = Vec::new();
    for row in rows {
        run_ids.push(row?);
    }
    Ok(run_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::init_embedded;

    fn sample_run(agent: &str, cost: f64) -> RunRecord {
        RunRecord {
            agent: agent.to_string(),
            input: "test input".to_string(),
            output: "test output".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-sonnet".to_string(),
            tokens_in: 100,
            tokens_out: 200,
            cost,
            duration_ms: 500,
            status: "success".to_string(),
            project: None,
        }
    }

    #[test]
    fn test_insert_and_get_history() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("agent-a", 0.01)).unwrap();
        insert_run(&db, sample_run("agent-b", 0.02)).unwrap();

        let all = get_history(&db, None, 10).unwrap();
        assert_eq!(all.len(), 2);

        let filtered = get_history(&db, Some("agent-a"), 10).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent, "agent-a");
    }

    #[test]
    fn test_run_project_roundtrips_through_history() {
        let db = init_embedded().unwrap();
        let mut with_project = sample_run("agent-a", 0.01);
        with_project.project = Some("/home/user/my-project".to_string());
        insert_run(&db, with_project).unwrap();
        insert_run(&db, sample_run("agent-b", 0.02)).unwrap(); // no project

        let all = get_history(&db, None, 10).unwrap();
        assert_eq!(all.len(), 2);
        let a = all.iter().find(|r| r.agent == "agent-a").unwrap();
        let b = all.iter().find(|r| r.agent == "agent-b").unwrap();
        assert_eq!(a.project.as_deref(), Some("/home/user/my-project"));
        assert_eq!(b.project, None);
    }

    #[test]
    fn test_costs_summary() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("agent-a", 0.01)).unwrap();
        insert_run(&db, sample_run("agent-a", 0.02)).unwrap();
        insert_run(&db, sample_run("agent-b", 0.05)).unwrap();

        let all = get_costs_summary(&db, None).unwrap();
        assert_eq!(all.len(), 2);

        let filtered = get_costs_summary(&db, Some("agent-a")).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].total_runs, 2);
        assert!((filtered[0].total_cost - 0.03).abs() < 1e-9);
    }

    #[test]
    fn test_insert_and_get_orchestration_run() {
        let db = init_embedded().unwrap();
        // First insert a parent run
        insert_run(&db, sample_run("agent-a", 0.01)).unwrap();
        let run_id = {
            let conn = db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM runs LIMIT 1").unwrap();
            let id: String = stmt.query_row([], |row| row.get(0)).unwrap();
            id
        };

        let orch = OrchestrationRunRecord {
            run_id: run_id.clone(),
            pattern: "blackboard".to_string(),
            config_json: r#"{"max_rounds":5}"#.to_string(),
            outcome_json: Some(r#"{"state":"halted"}"#.to_string()),
            rounds: 3,
            halt_reason: Some("consensus".to_string()),
            parent_run_id: None,
        };
        insert_orchestration_run(&db, orch).unwrap();

        let result = get_orchestration_run(&db, &run_id).unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.pattern, "blackboard");
        assert_eq!(r.rounds, 3);
    }

    #[test]
    fn test_get_run_pattern_returns_stored_pattern() {
        let db = init_embedded().unwrap();
        // `orchestration_runs.run_id` is a FK into `runs.id` — insert the
        // parent row first, exactly like `test_insert_and_get_orchestration_run`.
        insert_run_with_id(
            &db,
            "run-hier-1",
            sample_run("orchestration:hierarchical", 0.0),
        )
        .unwrap();
        let orch = OrchestrationRunRecord {
            run_id: "run-hier-1".to_string(),
            pattern: "hierarchical".to_string(),
            config_json: "{}".to_string(),
            outcome_json: None,
            rounds: 0,
            halt_reason: None,
            parent_run_id: None,
        };
        insert_orchestration_run(&db, orch).unwrap();

        assert_eq!(
            get_run_pattern(&db, "run-hier-1").unwrap(),
            Some("hierarchical".to_string())
        );
    }

    /// `"direct"` runs never get an `orchestration_runs` row (see
    /// `get_run_pattern`'s doc comment) — `--resume` must fall back to the
    /// event-sourced `ExecutionState::pattern` for them, not treat this
    /// `None` as "unknown run".
    #[test]
    fn test_get_run_pattern_none_for_unknown_run() {
        let db = init_embedded().unwrap();
        assert_eq!(get_run_pattern(&db, "no-such-run").unwrap(), None);
    }

    #[test]
    fn test_get_root_orchestration_runs_excludes_children() {
        // C6 regression: pagination must filter parent_run_id at the SQL
        // level (WHERE parent_run_id IS NULL ... LIMIT), not by fetching
        // `limit` rows and filtering roots out in Rust afterwards — the
        // latter lets children consume slots in the LIMIT window and can
        // silently return fewer than `limit` roots.
        let db = init_embedded().unwrap();

        insert_run_with_id(&db, "root-1", sample_run("agent-a", 0.01)).unwrap();
        insert_run_with_id(&db, "root-2", sample_run("agent-a", 0.02)).unwrap();
        insert_run_with_id(&db, "child-1", sample_run("agent-a", 0.03)).unwrap();

        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: "root-1".to_string(),
                pattern: "blackboard".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 1,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: "root-2".to_string(),
                pattern: "ring".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 1,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: "child-1".to_string(),
                pattern: "hierarchical".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 1,
                halt_reason: None,
                parent_run_id: Some("root-1".to_string()),
            },
        )
        .unwrap();

        let roots = get_root_orchestration_runs(&db, 50).unwrap();
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().all(|r| r.parent_run_id.is_none()));
        let ids: Vec<&str> = roots.iter().map(|r| r.run_id.as_str()).collect();
        assert!(ids.contains(&"root-1"));
        assert!(ids.contains(&"root-2"));
        assert!(!ids.contains(&"child-1"));

        // Also verify the LIMIT-window-loss scenario the SQL-level filter
        // fixes: a limit of 1 must still return exactly a root (not a
        // truncated set that happened to include the child).
        let limited = get_root_orchestration_runs(&db, 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert!(limited[0].parent_run_id.is_none());
    }

    #[test]
    fn test_insert_and_get_board_entries() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("agent-a", 0.01)).unwrap();
        let run_id = {
            let conn = db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM runs LIMIT 1").unwrap();
            stmt.query_row([], |row| row.get::<_, String>(0)).unwrap()
        };

        // Insert orchestration run first
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: run_id.clone(),
                pattern: "blackboard".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 1,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();

        insert_board_entry(
            &db,
            BoardEntryRecord {
                run_id: run_id.clone(),
                agent: "security".to_string(),
                round: 0,
                kind: "finding".to_string(),
                content: "SQL injection found".to_string(),
                refs_json: "[]".to_string(),
                confidence: 0.9,
                tokens_in: 100,
                tokens_out: 50,
            },
        )
        .unwrap();

        insert_board_entry(
            &db,
            BoardEntryRecord {
                run_id: run_id.clone(),
                agent: "perf".to_string(),
                round: 0,
                kind: "finding".to_string(),
                content: "N+1 query".to_string(),
                refs_json: "[]".to_string(),
                confidence: 0.8,
                tokens_in: 80,
                tokens_out: 40,
            },
        )
        .unwrap();

        let entries = get_board_entries(&db, &run_id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].agent, "security");
        assert_eq!(entries[1].agent, "perf");
    }

    #[test]
    fn test_insert_and_get_ring_contributions() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("agent-a", 0.01)).unwrap();
        let run_id = {
            let conn = db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM runs LIMIT 1").unwrap();
            stmt.query_row([], |row| row.get::<_, String>(0)).unwrap()
        };

        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: run_id.clone(),
                pattern: "ring".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 1,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();

        insert_ring_contribution(
            &db,
            RingContributionRecord {
                run_id: run_id.clone(),
                agent: "initiator".to_string(),
                lap: 0,
                position_in_lap: 0,
                action: "propose".to_string(),
                content: "Use Rust".to_string(),
                reactions_json: "[]".to_string(),
                tokens_in: 100,
                tokens_out: 200,
            },
        )
        .unwrap();

        let contribs = get_ring_contributions(&db, &run_id).unwrap();
        assert_eq!(contribs.len(), 1);
        assert_eq!(contribs[0].agent, "initiator");
        assert_eq!(contribs[0].action, "propose");
    }

    #[test]
    fn test_insert_and_get_ring_votes() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("agent-a", 0.01)).unwrap();
        let run_id = {
            let conn = db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM runs LIMIT 1").unwrap();
            stmt.query_row([], |row| row.get::<_, String>(0)).unwrap()
        };

        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: run_id.clone(),
                pattern: "ring".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 2,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();

        insert_ring_vote(
            &db,
            RingVoteRecord {
                run_id: run_id.clone(),
                agent: "agent-a".to_string(),
                position: "Use Rust".to_string(),
                confidence: 0.9,
                supports: "[0, 2]".to_string(),
                concerns: "[]".to_string(),
            },
        )
        .unwrap();

        insert_ring_vote(
            &db,
            RingVoteRecord {
                run_id: run_id.clone(),
                agent: "agent-b".to_string(),
                position: "Use Go".to_string(),
                confidence: 0.7,
                supports: "[1]".to_string(),
                concerns: "[\"recruiting\"]".to_string(),
            },
        )
        .unwrap();

        let votes = get_ring_votes(&db, &run_id).unwrap();
        assert_eq!(votes.len(), 2);
    }

    #[test]
    fn test_get_orchestration_run_not_found() {
        let db = init_embedded().unwrap();
        let result = get_orchestration_run(&db, "nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_board_entries_empty() {
        let db = init_embedded().unwrap();
        let entries = get_board_entries(&db, "nonexistent").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_get_ring_contributions_empty() {
        let db = init_embedded().unwrap();
        let contribs = get_ring_contributions(&db, "nonexistent").unwrap();
        assert!(contribs.is_empty());
    }

    #[test]
    fn test_get_ring_votes_empty() {
        let db = init_embedded().unwrap();
        let votes = get_ring_votes(&db, "nonexistent").unwrap();
        assert!(votes.is_empty());
    }

    #[test]
    fn test_orchestration_run_parent_and_children() {
        let db = init_embedded().unwrap();
        // parent hierarchical run
        insert_run(&db, sample_run("coord", 0.0)).unwrap();
        let parent_id = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT id FROM runs LIMIT 1", [], |r| r.get::<_, String>(0))
                .unwrap()
        };
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: parent_id.clone(),
                pattern: "hierarchical".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 0,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();
        // child blackboard run linked to the parent
        insert_run(&db, sample_run("searcher", 0.0)).unwrap();
        let child_id = {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT id FROM runs WHERE agent='searcher' LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
        };
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: child_id.clone(),
                pattern: "blackboard".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 2,
                halt_reason: None,
                parent_run_id: Some(parent_id.clone()),
            },
        )
        .unwrap();

        let got = get_orchestration_run(&db, &parent_id).unwrap().unwrap();
        assert_eq!(got.parent_run_id, None);
        let children = get_child_orchestration_runs(&db, &parent_id).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].run_id, child_id);
        assert_eq!(
            children[0].parent_run_id.as_deref(),
            Some(parent_id.as_str())
        );
    }

    #[test]
    fn test_delegation_events_roundtrip() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("coord", 0.0)).unwrap();
        let run_id = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT id FROM runs LIMIT 1", [], |r| r.get::<_, String>(0))
                .unwrap()
        };
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: run_id.clone(),
                pattern: "hierarchical".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 0,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();
        insert_delegation_event(
            &db,
            DelegationEventRecord {
                run_id: run_id.clone(),
                seq: 1,
                from_agent: "coord".into(),
                to_agent: "lead".into(),
                message: "do X".into(),
                depth: 1,
            },
        )
        .unwrap();
        insert_delegation_event(
            &db,
            DelegationEventRecord {
                run_id: run_id.clone(),
                seq: 0,
                from_agent: "user".into(),
                to_agent: "coord".into(),
                message: "start".into(),
                depth: 0,
            },
        )
        .unwrap();

        let events = get_delegation_events(&db, &run_id).unwrap();
        assert_eq!(events.len(), 2);
        // ordered by seq ascending
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[0].to_agent, "coord");
        assert_eq!(events[1].seq, 1);
        assert_eq!(events[1].to_agent, "lead");
    }
}
