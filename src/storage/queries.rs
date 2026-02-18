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
    let conn = db.lock().unwrap();
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
    let conn = db.lock().unwrap();
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
    let conn = db.lock().unwrap();
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
}
