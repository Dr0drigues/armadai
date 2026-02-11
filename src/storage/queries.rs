use serde::{Deserialize, Serialize};

use super::Database;

#[derive(Debug, Serialize, Deserialize)]
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

/// Insert a new execution record.
pub async fn insert_run(db: &Database, run: RunRecord) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    db.create::<Option<RunRecord>>(("runs", id.as_str()))
        .content(run)
        .await?;
    Ok(())
}

/// Get execution history, optionally filtered by agent name.
pub async fn get_history(
    db: &Database,
    agent: Option<&str>,
    limit: u32,
) -> anyhow::Result<Vec<RunRecord>> {
    let query = match agent {
        Some(name) => {
            format!(
                "SELECT * FROM runs WHERE agent = '{name}' ORDER BY created_at DESC LIMIT {limit}"
            )
        }
        None => {
            format!("SELECT * FROM runs ORDER BY created_at DESC LIMIT {limit}")
        }
    };
    let mut result = db.query(&query).await?;
    let records: Vec<RunRecord> = result.take(0)?;
    Ok(records)
}

/// Get total cost for an agent.
pub async fn get_agent_cost(db: &Database, agent: &str) -> anyhow::Result<f64> {
    let mut result = db
        .query(format!(
            "SELECT math::sum(cost) AS total FROM runs WHERE agent = '{agent}'"
        ))
        .await?;
    let total: Option<f64> = result.take("total")?;
    Ok(total.unwrap_or(0.0))
}
