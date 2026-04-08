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
        "INSERT INTO runs (id, agent, input, output, provider, model, tokens_in, tokens_out, cost, duration_ms, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, run.agent, run.input, run.output, run.provider, run.model,
                run.tokens_in, run.tokens_out, run.cost, run.duration_ms, run.status],
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
                "SELECT agent, input, output, provider, model, tokens_in, tokens_out, cost, duration_ms, status
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
                })
            })?;
            for row in rows {
                records.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT agent, input, output, provider, model, tokens_in, tokens_out, cost, duration_ms, status
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

/// Insert an orchestration run record (finished_at populated automatically).
pub fn insert_orchestration_run(
    db: &Database,
    record: OrchestrationRunRecord,
) -> anyhow::Result<()> {
    let conn = db
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
    conn.execute(
        "INSERT INTO orchestration_runs (run_id, pattern, config_json, outcome_json, rounds, halt_reason, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
        params![
            record.run_id,
            record.pattern,
            record.config_json,
            record.outcome_json,
            record.rounds,
            record.halt_reason
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
        "SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason
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
        "SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason
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
        };
        insert_orchestration_run(&db, orch).unwrap();

        let result = get_orchestration_run(&db, &run_id).unwrap();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.pattern, "blackboard");
        assert_eq!(r.rounds, 3);
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
}
