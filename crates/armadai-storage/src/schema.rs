use rusqlite::Connection;

/// Current schema version. Bumped whenever a migration is added.
#[allow(dead_code)] // not yet consumed outside tests; will back future migration tooling (Lot 2+)
pub const SCHEMA_VERSION: i64 = 3;

/// Apply the database schema: create base tables (target schema) then run migrations.
pub fn apply(conn: &Connection) -> anyhow::Result<()> {
    // NOTE: SQLite foreign keys ARE enforced by default in this build
    // (`PRAGMA foreign_keys` is ON). `migrate_to_v1` below temporarily
    // disables enforcement around its `orchestration_runs` table rebuild
    // (DROP + RENAME), since child rows in `board_entries`,
    // `ring_contributions`, and `ring_votes` reference `orchestration_runs`
    // and would otherwise trip a FOREIGN KEY constraint failure.
    //
    // Base tables. `orchestration_runs` here carries the v1 target schema
    // (relaxed CHECK + parent_run_id); an EXISTING pre-v1 database keeps its
    // old table (IF NOT EXISTS is a no-op) and is upgraded by `migrate`.
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
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            project TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_runs_agent ON runs(agent);
        CREATE INDEX IF NOT EXISTS idx_runs_created ON runs(created_at);

        CREATE TABLE IF NOT EXISTS orchestration_runs (
            run_id        TEXT PRIMARY KEY REFERENCES runs(id),
            pattern       TEXT NOT NULL CHECK (pattern IN ('direct', 'blackboard', 'ring', 'hierarchical')),
            config_json   TEXT NOT NULL,
            outcome_json  TEXT,
            rounds        INTEGER NOT NULL DEFAULT 0,
            halt_reason   TEXT,
            parent_run_id TEXT,
            created_at    TEXT NOT NULL DEFAULT (datetime('now')),
            finished_at   TEXT
        );

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

        CREATE TABLE IF NOT EXISTS ring_votes (
            run_id      TEXT NOT NULL REFERENCES orchestration_runs(run_id),
            agent       TEXT NOT NULL,
            position    TEXT NOT NULL,
            confidence  REAL NOT NULL,
            supports    TEXT NOT NULL DEFAULT '[]',
            concerns    TEXT NOT NULL DEFAULT '[]',
            PRIMARY KEY (run_id, agent)
        );

        CREATE TABLE IF NOT EXISTS delegation_events (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id     TEXT NOT NULL REFERENCES orchestration_runs(run_id),
            seq        INTEGER NOT NULL,
            from_agent TEXT NOT NULL,
            to_agent   TEXT NOT NULL,
            message    TEXT NOT NULL,
            depth      INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_delegation_events_run ON delegation_events(run_id, seq);

        CREATE TABLE IF NOT EXISTS execution_events (
            run_id      TEXT NOT NULL,
            seq         INTEGER NOT NULL,
            ts          TEXT NOT NULL DEFAULT (datetime('now')),
            kind        TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            PRIMARY KEY (run_id, seq)
        );

        CREATE INDEX IF NOT EXISTS idx_execution_events_run ON execution_events(run_id, seq);
        ",
    )?;

    migrate(conn)?;

    // `idx_orch_parent` must be created only after the `parent_run_id` column
    // is guaranteed to exist. For a fresh database it already does (base
    // schema above); for a legacy database `migrate_to_v1` just added it via
    // table rebuild. Creating it here (rather than in the base batch above)
    // avoids referencing a column that doesn't exist yet on an un-migrated
    // legacy `orchestration_runs` table (whose `CREATE TABLE IF NOT EXISTS`
    // above is a no-op).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_orch_parent ON orchestration_runs(parent_run_id);",
    )?;

    Ok(())
}

/// Apply pending migrations based on `PRAGMA user_version`.
fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        migrate_to_v1(conn)?;
        conn.execute_batch("PRAGMA user_version = 1;")?;
    }
    if version < 2 {
        migrate_to_v2(conn)?;
        conn.execute_batch("PRAGMA user_version = 2;")?;
    }
    if version < 3 {
        migrate_to_v3(conn)?;
        conn.execute_batch("PRAGMA user_version = 3;")?;
    }
    Ok(())
}

/// v0 → v1: relax the `orchestration_runs` CHECK and add `parent_run_id`.
///
/// SQLite cannot ALTER a CHECK constraint, so the table is rebuilt. A fresh
/// database already has the v1 schema from `apply` (detected via the
/// `parent_run_id` column), so the rebuild is skipped there.
fn migrate_to_v1(conn: &Connection) -> anyhow::Result<()> {
    let has_parent: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('orchestration_runs') WHERE name = 'parent_run_id'",
        [],
        |r| r.get::<_, i64>(0),
    )? > 0;
    if !has_parent {
        // `PRAGMA foreign_keys` is a no-op inside a transaction; `apply()`
        // runs no explicit transaction around this call, so toggling it here
        // takes effect for the DROP/RENAME below. Without this, the DROP
        // TABLE fails with SQLite error 787 (FOREIGN KEY constraint failed)
        // whenever `board_entries`, `ring_contributions`, or `ring_votes`
        // hold rows referencing `orchestration_runs`.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        conn.execute_batch(
            "
            CREATE TABLE orchestration_runs_new (
                run_id        TEXT PRIMARY KEY REFERENCES runs(id),
                pattern       TEXT NOT NULL CHECK (pattern IN ('direct', 'blackboard', 'ring', 'hierarchical')),
                config_json   TEXT NOT NULL,
                outcome_json  TEXT,
                rounds        INTEGER NOT NULL DEFAULT 0,
                halt_reason   TEXT,
                parent_run_id TEXT,
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at   TEXT
            );
            INSERT INTO orchestration_runs_new
                (run_id, pattern, config_json, outcome_json, rounds, halt_reason, created_at, finished_at)
                SELECT run_id, pattern, config_json, outcome_json, rounds, halt_reason, created_at, finished_at
                FROM orchestration_runs;
            DROP TABLE orchestration_runs;
            ALTER TABLE orchestration_runs_new RENAME TO orchestration_runs;
            CREATE INDEX IF NOT EXISTS idx_orch_parent ON orchestration_runs(parent_run_id);
            ",
        )?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    Ok(())
}

/// v1 → v2: add `runs.project` (the project root path a run was executed
/// from), so History/Costs can attribute runs to a project. A plain `ALTER
/// TABLE ADD COLUMN` suffices (unlike v1, no CHECK constraint is involved),
/// so no table rebuild is needed. A fresh database already has the column
/// from `apply`'s base `CREATE TABLE IF NOT EXISTS runs` above, so this is a
/// no-op there.
fn migrate_to_v2(conn: &Connection) -> anyhow::Result<()> {
    let has_project: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('runs') WHERE name = 'project'",
        [],
        |r| r.get::<_, i64>(0),
    )? > 0;
    if !has_project {
        conn.execute_batch("ALTER TABLE runs ADD COLUMN project TEXT;")?;
    }
    Ok(())
}

/// v2 → v3: add `execution_events`, the append-only event-sourcing log for
/// orchestration runs (OH1 Lot 1 socle — `core::orchestration::es`).
/// `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS` are both
/// idempotent, so this is safe to run unconditionally (a fresh database
/// already has the table from `apply`'s base batch above, in which case this
/// is a no-op).
fn migrate_to_v3(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS execution_events (
            run_id      TEXT NOT NULL,
            seq         INTEGER NOT NULL,
            ts          TEXT NOT NULL DEFAULT (datetime('now')),
            kind        TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            PRIMARY KEY (run_id, seq)
        );
        CREATE INDEX IF NOT EXISTS idx_execution_events_run ON execution_events(run_id, seq);
        ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }
    fn has_column(conn: &Connection, table: &str, col: &str) -> bool {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
            [col],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }
    fn has_table(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
            [table],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    #[test]
    fn fresh_db_is_at_schema_version_and_has_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(has_column(&conn, "orchestration_runs", "parent_run_id"));
        assert!(has_column(&conn, "runs", "project"));
        assert!(has_table(&conn, "execution_events"));
        // delegation_events table exists
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='delegation_events'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        // hierarchical is now an accepted pattern (CHECK relaxed)
        conn.execute("INSERT INTO runs (id, agent, input, output, provider, model) VALUES ('r1','a','i','o','p','m')", []).unwrap();
        conn.execute(
            "INSERT INTO orchestration_runs (run_id, pattern, config_json) VALUES ('r1','hierarchical','{}')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn legacy_db_migrates_without_data_loss() {
        let conn = Connection::open_in_memory().unwrap();
        // Recreate the PRE-v1 schema (old CHECK, no parent_run_id), user_version 0.
        conn.execute_batch(
            "
            CREATE TABLE runs (id TEXT PRIMARY KEY, agent TEXT NOT NULL, input TEXT NOT NULL,
                output TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0,
                cost REAL NOT NULL DEFAULT 0.0, duration_ms INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'success', created_at TEXT NOT NULL DEFAULT (datetime('now')));
            CREATE TABLE orchestration_runs (
                run_id TEXT PRIMARY KEY REFERENCES runs(id),
                pattern TEXT NOT NULL CHECK (pattern IN ('direct','blackboard','ring')),
                config_json TEXT NOT NULL, outcome_json TEXT,
                rounds INTEGER NOT NULL DEFAULT 0, halt_reason TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')), finished_at TEXT);
            INSERT INTO runs (id, agent, input, output, provider, model) VALUES ('old1','a','i','o','p','m');
            INSERT INTO orchestration_runs (run_id, pattern, config_json, rounds) VALUES ('old1','blackboard','{}',3);
            ",
        )
        .unwrap();
        assert_eq!(user_version(&conn), 0);
        assert!(!has_column(&conn, "orchestration_runs", "parent_run_id"));

        apply(&conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(has_column(&conn, "orchestration_runs", "parent_run_id"));
        // Existing row preserved.
        let (pat, rounds): (String, i64) = conn
            .query_row(
                "SELECT pattern, rounds FROM orchestration_runs WHERE run_id='old1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pat, "blackboard");
        assert_eq!(rounds, 3);
        // hierarchical now accepted.
        conn.execute("INSERT INTO runs (id, agent, input, output, provider, model) VALUES ('h1','a','i','o','p','m')", []).unwrap();
        conn.execute("INSERT INTO orchestration_runs (run_id, pattern, config_json) VALUES ('h1','hierarchical','{}')", []).unwrap();
    }

    #[test]
    fn legacy_db_with_child_rows_migrates() {
        let conn = Connection::open_in_memory().unwrap();
        // Recreate the PRE-v1 schema (old CHECK, no parent_run_id), user_version 0,
        // plus the legacy `board_entries` table (not created by the fixture above)
        // with a row referencing `old1` via its FK on `orchestration_runs(run_id)`.
        conn.execute_batch(
            "
            CREATE TABLE runs (id TEXT PRIMARY KEY, agent TEXT NOT NULL, input TEXT NOT NULL,
                output TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0,
                cost REAL NOT NULL DEFAULT 0.0, duration_ms INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'success', created_at TEXT NOT NULL DEFAULT (datetime('now')));
            CREATE TABLE orchestration_runs (
                run_id TEXT PRIMARY KEY REFERENCES runs(id),
                pattern TEXT NOT NULL CHECK (pattern IN ('direct','blackboard','ring')),
                config_json TEXT NOT NULL, outcome_json TEXT,
                rounds INTEGER NOT NULL DEFAULT 0, halt_reason TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')), finished_at TEXT);
            CREATE TABLE board_entries (
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
            INSERT INTO runs (id, agent, input, output, provider, model) VALUES ('old1','a','i','o','p','m');
            INSERT INTO orchestration_runs (run_id, pattern, config_json, rounds) VALUES ('old1','blackboard','{}',3);
            INSERT INTO board_entries (run_id, agent, round, kind, content) VALUES ('old1','a',1,'note','hello');
            ",
        )
        .unwrap();
        assert_eq!(user_version(&conn), 0);
        assert!(!has_column(&conn, "orchestration_runs", "parent_run_id"));

        // This is the assertion that fails without the FK-toggle fix: the
        // table rebuild's DROP TABLE orchestration_runs trips SQLite error 787
        // (FOREIGN KEY constraint failed) because `board_entries` still
        // references 'old1'.
        apply(&conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(has_column(&conn, "orchestration_runs", "parent_run_id"));

        // Child row survived the rebuild (rowid-preserving INSERT...SELECT + RENAME).
        let board_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM board_entries WHERE run_id = 'old1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(board_count, 1);

        // Parent row survived too.
        let (pat, rounds): (String, i64) = conn
            .query_row(
                "SELECT pattern, rounds FROM orchestration_runs WHERE run_id='old1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(pat, "blackboard");
        assert_eq!(rounds, 3);
    }

    #[test]
    fn apply_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        apply(&conn).unwrap(); // must not error or duplicate
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
    }

    /// A v1 database (already migrated past the CHECK-relaxation, i.e. it has
    /// `parent_run_id`, but predates `runs.project`) migrates to v2 by adding
    /// the column in place — no table rebuild, no data loss.
    #[test]
    fn v1_db_migrates_to_v2_adding_project_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE runs (id TEXT PRIMARY KEY, agent TEXT NOT NULL, input TEXT NOT NULL,
                output TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0,
                cost REAL NOT NULL DEFAULT 0.0, duration_ms INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'success', created_at TEXT NOT NULL DEFAULT (datetime('now')));
            CREATE TABLE orchestration_runs (
                run_id        TEXT PRIMARY KEY REFERENCES runs(id),
                pattern       TEXT NOT NULL CHECK (pattern IN ('direct', 'blackboard', 'ring', 'hierarchical')),
                config_json   TEXT NOT NULL,
                outcome_json  TEXT,
                rounds        INTEGER NOT NULL DEFAULT 0,
                halt_reason   TEXT,
                parent_run_id TEXT,
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at   TEXT
            );
            INSERT INTO runs (id, agent, input, output, provider, model) VALUES ('r1','a','i','o','p','m');
            ",
        )
        .unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();

        assert_eq!(user_version(&conn), 1);
        assert!(has_column(&conn, "orchestration_runs", "parent_run_id"));
        assert!(!has_column(&conn, "runs", "project"));

        apply(&conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(has_column(&conn, "runs", "project"));
        // Existing row preserved (ALTER TABLE ADD COLUMN, not a rebuild).
        let (agent, project): (String, Option<String>) = conn
            .query_row("SELECT agent, project FROM runs WHERE id='r1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(agent, "a");
        assert_eq!(project, None);
    }

    /// A v2 database (has `runs.project`, predates `execution_events`)
    /// migrates to v3 by adding the table in place — no data loss on
    /// existing tables.
    #[test]
    fn v2_db_migrates_to_v3_adding_execution_events_table() {
        let conn = Connection::open_in_memory().unwrap();
        // A v2 database is just the current base schema minus
        // `execution_events` (which migrate_to_v3 adds), at user_version 2.
        conn.execute_batch(
            "
            CREATE TABLE runs (id TEXT PRIMARY KEY, agent TEXT NOT NULL, input TEXT NOT NULL,
                output TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0,
                cost REAL NOT NULL DEFAULT 0.0, duration_ms INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'success', created_at TEXT NOT NULL DEFAULT (datetime('now')),
                project TEXT);
            CREATE TABLE orchestration_runs (
                run_id        TEXT PRIMARY KEY REFERENCES runs(id),
                pattern       TEXT NOT NULL CHECK (pattern IN ('direct', 'blackboard', 'ring', 'hierarchical')),
                config_json   TEXT NOT NULL,
                outcome_json  TEXT,
                rounds        INTEGER NOT NULL DEFAULT 0,
                halt_reason   TEXT,
                parent_run_id TEXT,
                created_at    TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at   TEXT
            );
            INSERT INTO runs (id, agent, input, output, provider, model) VALUES ('r1','a','i','o','p','m');
            ",
        )
        .unwrap();
        conn.execute_batch("PRAGMA user_version = 2;").unwrap();

        assert_eq!(user_version(&conn), 2);
        assert!(!has_table(&conn, "execution_events"));

        apply(&conn).unwrap();

        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(has_table(&conn, "execution_events"));
        // Existing row preserved (table add is additive, no rebuild involved).
        let agent: String = conn
            .query_row("SELECT agent FROM runs WHERE id='r1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(agent, "a");
        // The table is actually usable (PK + insert works).
        conn.execute(
            "INSERT INTO execution_events (run_id, seq, kind, payload_json) VALUES ('r1', 0, 'run_started', '{}')",
            [],
        )
        .unwrap();
    }
}
