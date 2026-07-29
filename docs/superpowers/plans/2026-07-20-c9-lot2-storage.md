# C9 Lot 2 — Storage (persistance hierarchical + sous-runs) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Persister le run hierarchical complet (run + delegation events) et ses sous-runs blackboard/ring imbriqués (liés par `parent_run_id`), via un vrai mécanisme de migration de schéma SQLite.

**Architecture:** (1) `schema.rs` gagne un versioning `PRAGMA user_version` + une migration v1 qui relâche la contrainte CHECK et ajoute `parent_run_id` + la table `delegation_events`. (2) `queries.rs` gagne le champ `parent_run_id` et les records/inserts/gets pour delegation events + enfants. (3) le moteur (`hierarchical.rs`) expose les sous-runs dans `OrchestrationResult.nested_runs` (types core `Board`/`RingToken`). (4) `cli/run.rs` persiste le run hierarchical + delegation events + chaque sous-run avec `parent_run_id`, en réutilisant les fonctions `record_*` existantes enrichies d'un paramètre `parent_run_id`.

**Tech Stack:** Rust edition 2024, rusqlite (bundled SQLite), serde_json.

## Global Constraints

- Base = `origin/release/1.0.0` (@ `b89e408`, après C9 Lot 1). Branche `feat/c9-lot2-storage`, PR vers `release/1.0.0`.
- Le module `storage` est gated `storage`. Vérifier clippy CI standard `--no-default-features --features tui -- -D warnings` ET `--no-default-features --features tui,providers-api -- -D warnings` (ne doivent pas régresser), PLUS **`cargo clippy --no-default-features --features tui,storage -- -D warnings`** et **`cargo clippy --no-default-features --features tui,storage,providers-api -- -D warnings`**. `cargo fmt -- --check`.
- Tests storage : `cargo test --no-default-features --features tui,storage -p armadai storage` (+ tests moteur : `cargo test --no-default-features --features tui,providers-api -p armadai orchestration`).
- **Rétro-compat impérative** : une base SQLite existante (schéma pré-v1, ancien CHECK `('direct','blackboard','ring')`, sans `parent_run_id`) doit migrer **sans perte de données**. Une base neuve doit être en v1 directement. La migration doit être **idempotente** (ré-`apply()` ne casse rien).
- Réutiliser le code existant : `record_orchestration_blackboard`/`_ring` (cli/run.rs) — les enrichir, pas les dupliquer. Le mapping contributions/votes/entries existe déjà, ne pas le réécrire.
- Le moteur (core) ne fait PAS de storage : il expose des données ; `cli/run.rs` (gated `storage`) persiste.

---

### Task 1: Mécanisme de migration `user_version` + schéma v1

**Files:**
- Modify: `src/storage/schema.rs`

**Interfaces:**
- Produces : `apply(conn)` inchangé en signature ; nouvelle constante `SCHEMA_VERSION = 1` ; schéma cible = `orchestration_runs` avec `CHECK (pattern IN ('direct','blackboard','ring','hierarchical'))` + colonne `parent_run_id TEXT`, table `delegation_events`, index `idx_orch_parent` et `idx_delegation_events_run`.

- [ ] **Step 1: Write the failing tests**

Ajouter un module test en bas de `schema.rs` :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
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

    #[test]
    fn fresh_db_is_at_schema_version_and_has_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
        assert!(has_column(&conn, "orchestration_runs", "parent_run_id"));
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
    fn apply_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        apply(&conn).unwrap(); // must not error or duplicate
        assert_eq!(user_version(&conn), SCHEMA_VERSION);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --no-default-features --features tui,storage -p armadai schema::`
Expected: FAIL (`SCHEMA_VERSION` undefined; fresh DB `parent_run_id` absent; `hierarchical` insert violates CHECK).

- [ ] **Step 3: Rewrite `schema.rs` — new base schema + migration**

Remplacer intégralement le contenu de `apply` et ajouter les fonctions, en gardant les tables `runs`/`board_entries`/`ring_contributions`/`ring_votes` telles quelles et en donnant à `orchestration_runs` le schéma cible + `delegation_events` :

```rust
use rusqlite::Connection;

/// Current schema version. Bumped whenever a migration is added.
pub const SCHEMA_VERSION: i64 = 1;

/// Apply the database schema: create base tables (target schema) then run migrations.
pub fn apply(conn: &Connection) -> anyhow::Result<()> {
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
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
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

        CREATE INDEX IF NOT EXISTS idx_orch_parent ON orchestration_runs(parent_run_id);

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
        ",
    )?;

    migrate(conn)?;
    Ok(())
}

/// Apply pending migrations based on `PRAGMA user_version`.
fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        migrate_to_v1(conn)?;
        conn.execute_batch("PRAGMA user_version = 1;")?;
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
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --no-default-features --features tui,storage -p armadai schema::`
Expected: PASS (3 tests).

- [ ] **Step 5: Clippy (all relevant modes) + fmt**

Run: `cargo clippy --no-default-features --features tui,storage -- -D warnings && cargo clippy --no-default-features --features tui -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/storage/schema.rs
git commit -m "feat(storage): user_version migrations; relax orchestration CHECK + parent_run_id + delegation_events"
```

---

### Task 2: `queries.rs` — `parent_run_id` + delegation events + enfants

**Files:**
- Modify: `src/storage/queries.rs`

**Interfaces:**
- Consumes (Task 1) : colonnes `parent_run_id`, table `delegation_events`.
- Produces :
  - `OrchestrationRunRecord` gagne `pub parent_run_id: Option<String>`.
  - `pub struct DelegationEventRecord { run_id, seq, from_agent, to_agent, message, depth }` (types : `String`/`i64`).
  - `pub fn insert_delegation_event(db, record) -> Result<()>`.
  - `pub fn get_delegation_events(db, run_id) -> Result<Vec<DelegationEventRecord>>` (triés par `seq`).
  - `pub fn get_child_orchestration_runs(db, parent_run_id) -> Result<Vec<OrchestrationRunRecord>>`.

- [ ] **Step 1: Write the failing tests**

Dans le module `tests` de `queries.rs`, ajouter :

```rust
    #[test]
    fn test_orchestration_run_parent_and_children() {
        let db = init_embedded().unwrap();
        // parent hierarchical run
        insert_run(&db, sample_run("coord", 0.0)).unwrap();
        let parent_id = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT id FROM runs LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap()
        };
        insert_orchestration_run(&db, OrchestrationRunRecord {
            run_id: parent_id.clone(),
            pattern: "hierarchical".to_string(),
            config_json: "{}".to_string(),
            outcome_json: None,
            rounds: 0,
            halt_reason: None,
            parent_run_id: None,
        }).unwrap();
        // child blackboard run linked to the parent
        insert_run(&db, sample_run("searcher", 0.0)).unwrap();
        let child_id = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT id FROM runs WHERE agent='searcher' LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap()
        };
        insert_orchestration_run(&db, OrchestrationRunRecord {
            run_id: child_id.clone(),
            pattern: "blackboard".to_string(),
            config_json: "{}".to_string(),
            outcome_json: None,
            rounds: 2,
            halt_reason: None,
            parent_run_id: Some(parent_id.clone()),
        }).unwrap();

        let got = get_orchestration_run(&db, &parent_id).unwrap().unwrap();
        assert_eq!(got.parent_run_id, None);
        let children = get_child_orchestration_runs(&db, &parent_id).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].run_id, child_id);
        assert_eq!(children[0].parent_run_id.as_deref(), Some(parent_id.as_str()));
    }

    #[test]
    fn test_delegation_events_roundtrip() {
        let db = init_embedded().unwrap();
        insert_run(&db, sample_run("coord", 0.0)).unwrap();
        let run_id = {
            let conn = db.lock().unwrap();
            conn.query_row("SELECT id FROM runs LIMIT 1", [], |r| r.get::<_, String>(0)).unwrap()
        };
        insert_orchestration_run(&db, OrchestrationRunRecord {
            run_id: run_id.clone(),
            pattern: "hierarchical".to_string(),
            config_json: "{}".to_string(),
            outcome_json: None,
            rounds: 0,
            halt_reason: None,
            parent_run_id: None,
        }).unwrap();
        insert_delegation_event(&db, DelegationEventRecord {
            run_id: run_id.clone(), seq: 1, from_agent: "coord".into(), to_agent: "lead".into(),
            message: "do X".into(), depth: 1,
        }).unwrap();
        insert_delegation_event(&db, DelegationEventRecord {
            run_id: run_id.clone(), seq: 0, from_agent: "user".into(), to_agent: "coord".into(),
            message: "start".into(), depth: 0,
        }).unwrap();

        let events = get_delegation_events(&db, &run_id).unwrap();
        assert_eq!(events.len(), 2);
        // ordered by seq ascending
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[0].to_agent, "coord");
        assert_eq!(events[1].seq, 1);
        assert_eq!(events[1].to_agent, "lead");
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui,storage -p armadai queries::tests::test_orchestration_run_parent queries::tests::test_delegation_events`
Expected: FAIL (`parent_run_id` field missing; `DelegationEventRecord`/functions undefined).

- [ ] **Step 3: Add `parent_run_id` to `OrchestrationRunRecord` + update SQL**

Dans `OrchestrationRunRecord` (≈ ligne 171), ajouter le champ :

```rust
    pub parent_run_id: Option<String>,
```

Dans `insert_orchestration_run`, mettre à jour l'INSERT :

```rust
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
```

Dans `get_orchestration_run` ET `get_orchestration_runs`, ajouter `parent_run_id` au SELECT et au mapping. Pour `get_orchestration_run` :

```rust
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
```

Appliquer le même ajout (colonne 6 `parent_run_id`) au `query_map` de `get_orchestration_runs`.

- [ ] **Step 4: Add `DelegationEventRecord` + insert/get + children query**

Ajouter après `RingVoteRecord` :

```rust
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
```

- [ ] **Step 5: Fix existing `OrchestrationRunRecord` literals**

Le nouveau champ casse les littéraux existants. Chercher et corriger (ajouter `parent_run_id: None`) :

Run: `rg -n "OrchestrationRunRecord \{" src/`

Corriger chaque occurrence hors de ce fichier (notamment `src/cli/run.rs` dans `record_orchestration_blackboard`/`_ring`) en ajoutant `parent_run_id: None,`. (Le câblage réel du parent est fait en Task 4 ; ici on met `None` pour compiler.) Vérifier par compilation.

- [ ] **Step 6: Run to verify pass**

Run: `cargo test --no-default-features --features tui,storage -p armadai queries::`
Expected: PASS (nouveaux + existants).

- [ ] **Step 7: Clippy + fmt**

Run: `cargo clippy --no-default-features --features tui,storage -- -D warnings && cargo clippy --no-default-features --features tui,storage,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/storage/queries.rs src/cli/run.rs
git commit -m "feat(storage): parent_run_id on orchestration runs + delegation events records/queries"
```

---

### Task 3: Moteur — exposer les sous-runs dans `OrchestrationResult`

**Files:**
- Modify: `src/core/orchestration/hierarchical.rs`

**Interfaces:**
- Consumes : `run_nested_team` (Lot 1), `Board`, `RingToken`, `BlackboardConfig`, `RingConfig`, `NestedPattern`.
- Produces :
  - `pub enum NestedRun { Blackboard { team_lead: String, task: String, board: Board, config: BlackboardConfig }, Ring { team_lead: String, task: String, token: RingToken, config: RingConfig } }`.
  - `OrchestrationResult` gagne `pub nested_runs: Vec<NestedRun>`.
  - `EngineState` gagne `nested_runs: Vec<NestedRun>` ; `run()` déplace ce vec dans le résultat ; `run_nested_team` y pousse un `NestedRun` par sous-run.

- [ ] **Step 1: Write the failing test**

Dans le module test de `hierarchical.rs`, ajouter (réutilise `nested_blackboard_config`, `FixedProvider`, `make_agent` du Lot 1) :

```rust
#[tokio::test]
async fn test_nested_run_is_surfaced_in_result() {
    let config = nested_blackboard_config();
    let mut agents = HashMap::new();
    agents.insert("coordinator".to_string(), make_agent("coordinator", "Coordinate."));
    agents.insert("research-lead".to_string(), make_agent("research-lead", "Lead."));
    agents.insert("searcher".to_string(), make_agent("searcher", "Search."));
    agents.insert("analyst".to_string(), make_agent("analyst", "Analyze."));

    let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    providers.insert("coordinator".to_string(), Arc::new(FixedProvider::new("@research-lead: analyze the topic")));
    providers.insert("research-lead".to_string(), Arc::new(FixedProvider::new("lead verdict")));
    let board_action = "ACTION: FINDING\nCONFIDENCE: 0.9\nCONTENT: a finding";
    providers.insert("searcher".to_string(), Arc::new(FixedProvider::new(board_action)));
    providers.insert("analyst".to_string(), Arc::new(FixedProvider::new(board_action)));

    let mut engine = HierarchicalEngine::new(config, agents, providers, Arc::new(NullSink));
    let result = engine.run("Do research").await.unwrap();

    assert_eq!(result.nested_runs.len(), 1, "one nested sub-run should be surfaced");
    match &result.nested_runs[0] {
        NestedRun::Blackboard { team_lead, board, .. } => {
            assert_eq!(team_lead, "research-lead");
            assert!(!board.entries().is_empty(), "board should have entries");
        }
        NestedRun::Ring { .. } => panic!("expected a blackboard nested run"),
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui,providers-api -p armadai test_nested_run_is_surfaced_in_result`
Expected: FAIL (`NestedRun` undefined; `result.nested_runs` missing).

- [ ] **Step 3: Add `NestedRun` + result/state fields**

Dans `hierarchical.rs`, ajouter le type après `DelegationEvent` (≈ ligne 47) :

```rust
/// A nested sub-run produced by a team running a blackboard/ring sub-pattern (C9).
/// Held so the CLI layer can persist it (with `parent_run_id`) after the run.
pub enum NestedRun {
    Blackboard {
        team_lead: String,
        task: String,
        board: super::blackboard::Board,
        config: super::blackboard::BlackboardConfig,
    },
    Ring {
        team_lead: String,
        task: String,
        token: super::ring::RingToken,
        config: super::ring::RingConfig,
    },
}
```

Ajouter à `OrchestrationResult` (≈ ligne 28) le champ :

```rust
    /// Nested blackboard/ring sub-runs (C9), for downstream persistence.
    pub nested_runs: Vec<NestedRun>,
```

Ajouter à `EngineState` (≈ ligne 67) :

```rust
    nested_runs: Vec<NestedRun>,
```

Initialiser `nested_runs: Vec::new()` dans le `EngineState { ... }` de `with_routing_rules` (≈ ligne 148).

Dans `run()`, à la construction de `OrchestrationResult` (≈ ligne 186), ajouter :

```rust
            nested_runs: std::mem::take(&mut state.nested_runs),
```

- [ ] **Step 4: Push a `NestedRun` in `run_nested_team`**

Dans `run_nested_team`, la fonction construit `board`/`token` et `config` localement. Après avoir calculé `outcome_text` et **avant** de replier les métriques (les métriques lisent `board.entries()`/`token.contributions` par référence, donc extraire les métriques d'abord, puis déplacer `board`/`token` dans le `NestedRun`).

Restructurer les deux bras : au lieu de laisser `board`/`token` être droppés, les retourner du `match` avec les métriques, puis après le fold pousser le `NestedRun`. Concrètement, changer le `match nested { ... }` pour qu'il renvoie aussi le `NestedRun` :

Pour le bras Blackboard, remplacer la fin par :

```rust
            let text = board
                .entries()
                .iter()
                .map(|e| format!("[{}] {}", e.agent, e.content))
                .collect::<Vec<_>>()
                .join("\n");
            let nested = NestedRun::Blackboard {
                team_lead: team_lead.to_string(),
                task: task.to_string(),
                board,
                config,
            };
            (text, ti, to, cost, nested)
```

(le fold `(ti,to,cost)` lit `board.entries()` AVANT cette construction — garder cet ordre : fold d'abord, puis `let text`, puis move `board`).

Pour le bras Ring, symétriquement, après le fold et le calcul de `text` :

```rust
            let nested = NestedRun::Ring {
                team_lead: team_lead.to_string(),
                task: task.to_string(),
                token,
                config,
            };
            (text, ti, to, cost, nested)
```

Adapter le binding : `let (outcome_text, folded_in, folded_out, folded_cost, nested) = match nested { ... };`
(⚠️ renommer la variable locale `nested` du `match` — elle entre en collision avec le paramètre `nested: NestedPattern`. Utiliser `nested_run` pour la valeur produite : `let (outcome_text, folded_in, folded_out, folded_cost, nested_run) = match nested { ... };`, et dans les bras `(text, ti, to, cost, nested_run)`.)

Après le bloc de fold des métriques (et avant/après l'émission de `NestedEnd`, peu importe), pousser dans l'état :

```rust
    {
        let mut s = state.lock().unwrap_or_else(|e| {
            tracing::warn!("Mutex poisoned pushing nested run: {:?}", e);
            e.into_inner()
        });
        s.nested_runs.push(nested_run);
    }
```

- [ ] **Step 5: Fix other `OrchestrationResult` construction sites if any**

Run: `rg -n "OrchestrationResult \{" src/`
Le seul site de construction est `run()` (les tests lisent le résultat mais ne le construisent pas). Si un test/bench construit un `OrchestrationResult` littéral, ajouter `nested_runs: Vec::new()`. Vérifier par compilation.

- [ ] **Step 6: Run tests (nested surface + all Lot 1 regression)**

Run: `cargo test --no-default-features --features tui,providers-api -p armadai orchestration`
Expected: PASS (nouveau `test_nested_run_is_surfaced_in_result` + tous les tests Lot 1 : blackboard fold, ring, arbitrage, flat delegation).

- [ ] **Step 7: Clippy 2 modes + fmt**

Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/core/orchestration/hierarchical.rs
git commit -m "feat(orchestration): surface nested sub-runs in OrchestrationResult"
```

---

### Task 4: `cli/run.rs` — persistance hierarchical + sous-runs

**Files:**
- Modify: `src/cli/run.rs`

**Interfaces:**
- Consumes : `insert_delegation_event`, `DelegationEventRecord`, `OrchestrationRunRecord.parent_run_id` (Task 2) ; `OrchestrationResult.{trace,nested_runs}`, `NestedRun` (Task 3) ; `record_orchestration_blackboard`/`_ring` existants.
- Produces :
  - `record_orchestration_blackboard`/`_ring` gagnent un paramètre `parent_run_id: Option<&str>` (les appels top-level passent `None`) et retournent le `run_id` généré (`String`) pour permettre le lien parent→enfant. Refactor : extraire le corps en `*_into(db, ...)` prenant `&Database`, et garder le wrapper public appelant `init_db()`.
  - `record_orchestration_hierarchical(result: &OrchestrationResult, config: &OrchestrationConfig, input: &str)` : persiste le run hierarchical (pattern `hierarchical`, `parent_run_id = None`) + ses `delegation_events` (depuis `result.trace`) + chaque `NestedRun` via les fonctions `record_*_into` avec `parent_run_id = Some(hierarchical_run_id)`.
  - Appel de `record_orchestration_hierarchical(&result, &orch_config, input)` dans la branche `"hierarchical"` de `run_orchestrated` (gated `#[cfg(feature = "storage")]`, symétrique aux branches blackboard/ring).

- [ ] **Step 1: Write the failing test**

Le corps de persistance doit être testable avec une DB en mémoire. Extraire un helper `record_hierarchical_into(db, result, config, input) -> anyhow::Result<String>` (retourne le run_id du hierarchical). Test dans le module `#[cfg(all(test, feature = "storage"))]` de `cli/run.rs` (créer le module s'il n'existe pas) :

```rust
#[cfg(all(test, feature = "storage"))]
mod storage_tests {
    use super::*;
    use crate::core::orchestration::OrchestrationConfig;
    use crate::core::orchestration::hierarchical::{DelegationEvent, NestedRun, OrchestrationResult};
    use crate::core::orchestration::blackboard::{Board, BlackboardConfig};
    use crate::storage::{init_embedded, queries};

    #[test]
    fn hierarchical_run_and_nested_children_are_persisted() {
        let db = init_embedded().unwrap();

        // A hierarchical result with one delegation event and one nested board.
        let mut board = Board::new("subtask".to_string(), 50_000);
        // (empty board is fine; we only assert the run + linkage persists)
        let result = OrchestrationResult {
            content: "final".to_string(),
            trace: vec![DelegationEvent {
                from: "coordinator".to_string(),
                to: "research-lead".to_string(),
                message: "analyze".to_string(),
                depth: 1,
            }],
            total_tokens_in: 30,
            total_tokens_out: 40,
            total_cost: 0.01,
            invocation_count: 3,
            nested_runs: vec![NestedRun::Blackboard {
                team_lead: "research-lead".to_string(),
                task: "subtask".to_string(),
                board,
                config: BlackboardConfig::default(),
            }],
        };
        let config = OrchestrationConfig::default();

        let parent_id = record_hierarchical_into(&db, &result, &config, "do research").unwrap();

        // Parent persisted as hierarchical with no parent.
        let parent = queries::get_orchestration_run(&db, &parent_id).unwrap().unwrap();
        assert_eq!(parent.pattern, "hierarchical");
        assert_eq!(parent.parent_run_id, None);
        // Delegation event persisted.
        let events = queries::get_delegation_events(&db, &parent_id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to_agent, "research-lead");
        // Nested child persisted and linked.
        let children = queries::get_child_orchestration_runs(&db, &parent_id).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].pattern, "blackboard");
        assert_eq!(children[0].parent_run_id.as_deref(), Some(parent_id.as_str()));
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui,storage -p armadai storage_tests`
Expected: FAIL (`record_hierarchical_into` undefined; `OrchestrationResult` has `nested_runs` from Task 3 so it compiles once the fn exists).

- [ ] **Step 3: Refactor `record_orchestration_blackboard`/`_ring` into `*_into` with parent**

Pour chacune : renommer le corps existant en une fonction `fn record_orchestration_blackboard_into(db: &crate::storage::Database, board: &Board, config: &BlackboardConfig, input: &str, parent_run_id: Option<&str>) -> String` qui :
- génère `run_id`, insère le parent `RunRecord` via `insert_run_with_id`,
- insère l'`OrchestrationRunRecord` en passant `parent_run_id: parent_run_id.map(|s| s.to_string())`,
- insère les board entries,
- retourne `run_id`.

Garder le wrapper public existant :

```rust
fn record_orchestration_blackboard(
    board: &crate::core::orchestration::blackboard::Board,
    config: &crate::core::orchestration::blackboard::BlackboardConfig,
    input: &str,
) {
    let db = match crate::storage::init_db() {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to init storage: {e}");
            return;
        }
    };
    let _ = record_orchestration_blackboard_into(&db, board, config, input, None);
}
```

Faire de même pour `_ring` → `record_orchestration_ring_into(db, token, config, input, parent_run_id) -> String`.
Mettre `parent_run_id: <passé>` dans l'`OrchestrationRunRecord` (remplace le `parent_run_id: None` mis en Task 2 Step 5).
Les erreurs internes restent en `tracing::warn!` + continue (comportement actuel), mais la fonction `*_into` renvoie toujours le `run_id` généré (même en cas d'échec partiel d'insertion d'entrées, cohérent avec le fire-and-forget existant).

- [ ] **Step 4: Implement `record_hierarchical_into` + public wrapper**

```rust
#[cfg(feature = "storage")]
fn record_hierarchical_into(
    db: &crate::storage::Database,
    result: &crate::core::orchestration::hierarchical::OrchestrationResult,
    config: &crate::core::orchestration::OrchestrationConfig,
    input: &str,
) -> anyhow::Result<String> {
    use crate::core::orchestration::hierarchical::NestedRun;
    use crate::storage::queries;

    let run_id = uuid::Uuid::new_v4().to_string();

    // 1. Parent run row.
    let parent = queries::RunRecord {
        agent: "orchestration:hierarchical".to_string(),
        input: input.to_string(),
        output: result.content.clone(),
        provider: "orchestration".to_string(),
        model: String::new(),
        tokens_in: result.total_tokens_in as i64,
        tokens_out: result.total_tokens_out as i64,
        cost: result.total_cost,
        duration_ms: 0,
        status: "success".to_string(),
    };
    queries::insert_run_with_id(db, &run_id, parent)?;

    // 2. Orchestration metadata (hierarchical, no parent).
    queries::insert_orchestration_run(db, queries::OrchestrationRunRecord {
        run_id: run_id.clone(),
        pattern: "hierarchical".to_string(),
        config_json: serde_json::to_string(config).unwrap_or_default(),
        outcome_json: None,
        rounds: result.invocation_count as i64,
        halt_reason: None,
        parent_run_id: None,
    })?;

    // 3. Delegation events (seq = order in trace).
    for (seq, ev) in result.trace.iter().enumerate() {
        let rec = queries::DelegationEventRecord {
            run_id: run_id.clone(),
            seq: seq as i64,
            from_agent: ev.from.clone(),
            to_agent: ev.to.clone(),
            message: ev.message.clone(),
            depth: ev.depth as i64,
        };
        if let Err(e) = queries::insert_delegation_event(db, rec) {
            tracing::warn!("Failed to record delegation event: {e}");
        }
    }

    // 4. Nested sub-runs, linked to the hierarchical parent.
    for nested in &result.nested_runs {
        match nested {
            NestedRun::Blackboard { task, board, config, .. } => {
                let _ = record_orchestration_blackboard_into(db, board, config, task, Some(&run_id));
            }
            NestedRun::Ring { task, token, config, .. } => {
                let _ = record_orchestration_ring_into(db, token, config, task, Some(&run_id));
            }
        }
    }

    Ok(run_id)
}

#[cfg(feature = "storage")]
fn record_orchestration_hierarchical(
    result: &crate::core::orchestration::hierarchical::OrchestrationResult,
    config: &crate::core::orchestration::OrchestrationConfig,
    input: &str,
) {
    let db = match crate::storage::init_db() {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!("Failed to init storage: {e}");
            return;
        }
    };
    if let Err(e) = record_hierarchical_into(&db, result, config, input) {
        tracing::warn!("Failed to record hierarchical run: {e}");
    }
}
```

- [ ] **Step 5: Wire the call in the hierarchical branch**

Dans `run_orchestrated`, branche `"hierarchical"`, après `let result = engine.run(input).await?;` et avant l'affichage/`emit_agent_ends`, ajouter :

```rust
            #[cfg(feature = "storage")]
            record_orchestration_hierarchical(&result, &orch_config, input);
```

(`orch_config` est le nom de la variable de config dans cette branche — vérifier le nom exact et l'utiliser ; c'est le `OrchestrationConfig` déjà construit et validé plus haut dans la branche.)

- [ ] **Step 6: Run the storage test + full suites**

Run: `cargo test --no-default-features --features tui,storage -p armadai storage_tests`
Expected: PASS.
Run: `cargo test --no-default-features --features tui,providers-api -p armadai orchestration`
Expected: PASS (moteur non régressé).

- [ ] **Step 7: Clippy (all modes) + fmt + build**

Run: `cargo clippy --no-default-features --features tui -- -D warnings && cargo clippy --no-default-features --features tui,providers-api -- -D warnings && cargo clippy --no-default-features --features tui,storage -- -D warnings && cargo clippy --no-default-features --features tui,storage,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/cli/run.rs
git commit -m "feat(cli): persist hierarchical run + delegation events + nested sub-runs (parent_run_id)"
```

---

## Notes pour l'implémenteur

- Ne PAS toucher web/tui (Lot 3). Ce lot s'arrête à la persistance.
- La migration doit rester **idempotente** : `apply()` est appelé à chaque `init_db()` ; grâce à `user_version`, la migration v1 ne s'exécute qu'une fois.
- Sur une base existante, la reconstruction de `orchestration_runs` ne recrée PAS les tables enfants (board_entries/…) — leurs FK (non enforced, cf. commentaire dans `schema.rs`) référencent `orchestration_runs(run_id)` par valeur ; les données restent cohérentes.
- Les fonctions `record_*` restent **fire-and-forget** (warn + continue) sauf `record_hierarchical_into` qui renvoie `Result` pour être testable ; le wrapper public avale l'erreur en `warn!` comme les autres.
- Collision de nom à éviter en Task 3 : le paramètre `nested: NestedPattern` de `run_nested_team` vs la valeur produite par le `match` → nommer cette dernière `nested_run`.
- Vérifier le nom exact de la variable de config dans la branche `"hierarchical"` de `run_orchestrated` (probablement `orch_config`) avant de câbler l'appel Step 5.
