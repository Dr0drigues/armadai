use rusqlite::Connection;

/// Apply the database schema.
pub fn apply(conn: &Connection) -> anyhow::Result<()> {
    // NOTE: PRAGMA foreign_keys is intentionally omitted here.  The FK
    // constraints in the schema below exist for documentation and external
    // tooling only; enforcing them globally could break existing code paths
    // that insert into `runs` without a matching orchestration record.
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY,
            agent TEXT NOT NULL,
            input TEXT NOT NULL,
            output TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            tokens_in INTEGER NOT NULL DEFAULT 0,
            tokens_out INTEGER NOT NULL DEFAULT 0,
            cost REAL NOT NULL DEFAULT 0.0,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'success',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_runs_agent ON runs(agent);
        CREATE INDEX IF NOT EXISTS idx_runs_created ON runs(created_at);

        -- Orchestration runs (extends runs table)
        CREATE TABLE IF NOT EXISTS orchestration_runs (
            run_id       TEXT PRIMARY KEY REFERENCES runs(id),
            pattern      TEXT NOT NULL CHECK (pattern IN ('direct', 'blackboard', 'ring')),
            config_json  TEXT NOT NULL,
            outcome_json TEXT,
            rounds       INTEGER NOT NULL DEFAULT 0,
            halt_reason  TEXT,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            finished_at  TEXT
        );

        -- Blackboard entries
        CREATE TABLE IF NOT EXISTS board_entries (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id      TEXT NOT NULL REFERENCES orchestration_runs(run_id),
            agent       TEXT NOT NULL,
            round       INTEGER NOT NULL,
            kind        TEXT NOT NULL,
            content     TEXT NOT NULL,
            refs_json   TEXT NOT NULL DEFAULT '[]',
            confidence  REAL NOT NULL DEFAULT 0.5,
            tokens_in   INTEGER NOT NULL DEFAULT 0,
            tokens_out  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_board_entries_run ON board_entries(run_id, round);

        -- Ring contributions
        CREATE TABLE IF NOT EXISTS ring_contributions (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id          TEXT NOT NULL REFERENCES orchestration_runs(run_id),
            agent           TEXT NOT NULL,
            lap             INTEGER NOT NULL,
            position_in_lap INTEGER NOT NULL,
            action          TEXT NOT NULL,
            content         TEXT NOT NULL,
            reactions_json  TEXT NOT NULL DEFAULT '[]',
            tokens_in       INTEGER NOT NULL DEFAULT 0,
            tokens_out      INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_ring_contributions_run ON ring_contributions(run_id, lap);

        -- Ring votes
        CREATE TABLE IF NOT EXISTS ring_votes (
            run_id      TEXT NOT NULL REFERENCES orchestration_runs(run_id),
            agent       TEXT NOT NULL,
            position    TEXT NOT NULL,
            confidence  REAL NOT NULL,
            supports    TEXT NOT NULL DEFAULT '[]',
            concerns    TEXT NOT NULL DEFAULT '[]',
            PRIMARY KEY (run_id, agent)
        );
        ",
    )?;
    Ok(())
}
