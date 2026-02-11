use super::Database;

/// Apply the database schema.
pub async fn apply(db: &Database) -> anyhow::Result<()> {
    db.query(
        "
        DEFINE TABLE IF NOT EXISTS runs SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS agent      ON TABLE runs TYPE string;
        DEFINE FIELD IF NOT EXISTS input      ON TABLE runs TYPE string;
        DEFINE FIELD IF NOT EXISTS output     ON TABLE runs TYPE string;
        DEFINE FIELD IF NOT EXISTS provider   ON TABLE runs TYPE string;
        DEFINE FIELD IF NOT EXISTS model      ON TABLE runs TYPE string;
        DEFINE FIELD IF NOT EXISTS tokens_in  ON TABLE runs TYPE int;
        DEFINE FIELD IF NOT EXISTS tokens_out ON TABLE runs TYPE int;
        DEFINE FIELD IF NOT EXISTS cost       ON TABLE runs TYPE float;
        DEFINE FIELD IF NOT EXISTS duration_ms ON TABLE runs TYPE int;
        DEFINE FIELD IF NOT EXISTS status     ON TABLE runs TYPE string;
        DEFINE FIELD IF NOT EXISTS created_at ON TABLE runs TYPE datetime DEFAULT time::now();

        DEFINE TABLE IF NOT EXISTS agent_stats SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS agent       ON TABLE agent_stats TYPE string;
        DEFINE FIELD IF NOT EXISTS total_runs  ON TABLE agent_stats TYPE int;
        DEFINE FIELD IF NOT EXISTS total_cost  ON TABLE agent_stats TYPE float;
        DEFINE FIELD IF NOT EXISTS avg_duration_ms ON TABLE agent_stats TYPE int;
        DEFINE FIELD IF NOT EXISTS last_run    ON TABLE agent_stats TYPE datetime;
        ",
    )
    .await?;

    Ok(())
}
