# OH1 Lot 5b — Projecteur (tables plates dérivées du log) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Les tables plates (`runs`/`orchestration_runs`/`board_entries`/`ring_contributions`/`ring_votes`) deviennent des **projections dérivées du log** `execution_events`, matérialisées via un projecteur idempotent, avec une commande `armadai projections rebuild` pour les reconstruire.

**Architecture:** Après 5a, le log est persisté au fil de l'eau et les tables plates sont encore écrites par les `record_*_es_into(state, config)`. 5b introduit un projecteur `project_run(db, run_id)` qui lit `execution_events[run_id]` → fold → `ExecutionState`, et écrit les tables plates (DELETE-puis-INSERT idempotent). Le chemin `run` appelle désormais `project_run` au lieu de `record_*_es`. Le `config` (paramètre runtime absent du log) est capturé par un nouvel event `ConfigSnapshot { config_json }` émis juste après `RunStarted`, stocké dans `state.config_json`, lu par le projecteur — décision Dimitri validée (nécessaire aussi au resume Lot 6).

**Tech Stack:** Rust edition 2024, rusqlite (`storage`), serde, tokio.

## Global Constraints

- CI clippy **3 modes** (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check`.
- Tous les tests existants restent verts (aucune régression). Feature persistance/projection gated `storage`.
- Le projecteur est **idempotent** : `project_run` deux fois sur le même `run_id` produit exactement les mêmes lignes (pas de doublon) → DELETE des lignes du `run_id` avant réinsertion.
- `run_id(log) == run_id(tables)` (déjà assuré en 5a).
- Changement user-facing (nouvelle commande, modèle de dérivation) → PR + revue indépendante, **validation Dimitri avant merge**.
- `ConfigSnapshot.config_json` = config **déjà sérialisée** par le moteur émetteur (pattern-agnostique : `BlackboardConfig` pour blackboard, `RingConfig` pour ring, `OrchestrationConfig` pour hierarchical). Direct n'a pas de config orchestration → n'émet pas `ConfigSnapshot` (ou `config_json` vide).

---

### Task 1: Event `ConfigSnapshot` + capture du config dans l'état

**Files:**
- Modify: `src/core/orchestration/es/event.rs` — ajouter la variante `ConfigSnapshot { config_json: String }` à `ExecutionEvent` (avec son tag serde court, ex. `t = "config"`).
- Modify: `src/core/orchestration/es/state.rs` — champ `pub config_json: Option<String>` sur `ExecutionState` (défaut `None`) ; bras `apply` pour `ConfigSnapshot` → `state.config_json = Some(config_json.clone())`.
- Modify: `src/core/orchestration/es/blackboard.rs`, `ring.rs`, `hierarchical.rs` — chaque moteur émet `ConfigSnapshot { config_json: serde_json::to_string(&config).unwrap_or_default() }` **immédiatement après** `RunStarted` (même endroit où RunStarted est émis).
- Modify: `src/core/orchestration/es/bridge.rs` — `map_execution_to_run_events` : `ConfigSnapshot` → `vec![]` (aucun `RunEvent` d'observabilité).
- Test: `src/core/orchestration/es/state.rs` (module de tests du fold).

**Interfaces:**
- Produces: `ExecutionEvent::ConfigSnapshot { config_json: String }` ; `ExecutionState.config_json: Option<String>`.

- [ ] **Step 1: Write the failing test**

Dans le module de tests de `state.rs` :

```rust
#[test]
fn config_snapshot_is_captured_in_state() {
    let events = vec![
        ExecutionEvent::RunStarted {
            run_id: "r".into(), pattern: "blackboard".into(),
            agents: vec!["a".into()], input: "x".into(), project: None,
        },
        ExecutionEvent::ConfigSnapshot { config_json: "{\"max_rounds\":5}".into() },
    ];
    let state = fold(&events);
    assert_eq!(state.config_json.as_deref(), Some("{\"max_rounds\":5}"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api config_snapshot_is_captured_in_state`
Expected: FAIL (variante/ champ inexistants → erreur de compilation).

- [ ] **Step 3: Implement** — variante + champ + bras apply + émission dans les 3 moteurs + bras bridge.

Ajouter la variante à l'enum (avec `#[serde(rename = "config")]` cohérent avec le style des autres variantes), le champ `config_json: Option<String>` à `ExecutionState` (+ `Default`), le bras `apply`, l'émission après RunStarted dans blackboard/ring/hierarchical (`Action::Emit(ExecutionEvent::ConfigSnapshot { config_json: serde_json::to_string(&config).unwrap_or_default() })` — adapter au mécanisme d'émission de chaque `decide`/boucle), et le bras `ConfigSnapshot => vec![]` dans `map_execution_to_run_events`.

- [ ] **Step 4: Run tests** — nouveau + toute la suite ES.

Run: `cargo test --no-default-features --features tui,providers-api,storage`
Expected: tout vert.

- [ ] **Step 5: Verify 3 modes clippy + fmt** (voir Global Constraints). Expected: 0 warning, fmt propre.

- [ ] **Step 6: Commit**

```bash
git add src/core/orchestration/es/
git commit -m "feat(es): add ConfigSnapshot event capturing run config in the log"
```

---

### Task 2: Projecteur idempotent `project_run(db, run_id)`

**Files:**
- Modify: `src/cli/run_es_record.rs` — ajouter `pub fn project_run(db: &Database, run_id: &str) -> anyhow::Result<()>` ; ajouter des DELETE idempotents ; réutiliser `record_blackboard_es_into`/`record_ring_es_into`/`record_hierarchical_into` (row-building) sourcés depuis l'état projeté + `state.config_json`.
- Modify: `src/storage/queries.rs` — ajouter les DELETE par `run_id` s'ils n'existent pas : `delete_projection_for_run(db, run_id)` supprimant les lignes de `runs`, `orchestration_runs`, `board_entries`, `ring_contributions`, `ring_votes`, `delegation_events` pour ce `run_id`.
- Test: `src/cli/run_es_record.rs` (module `storage_tests`).

**Interfaces:**
- Consumes: `SqliteLog::events` (ou une requête directe sur `execution_events`), `fold`, `ExecutionState.config_json` (Task 1), `record_*_es_into` (Task 1 de 5a, signature avec `run_id`).
- Produces: `project_run(db: &Database, run_id: &str) -> anyhow::Result<()>` ; `queries::delete_projection_for_run(db: &Database, run_id: &str) -> anyhow::Result<()>`.

- [ ] **Step 1: Write the failing test** — idempotence.

```rust
#[test]
fn project_run_is_idempotent() {
    let db = crate::storage::init_embedded().unwrap();
    // Persister un log blackboard minimal via SqliteLog.
    let mut log = crate::core::orchestration::es::log::SqliteLog::new(db.clone());
    for e in sample_blackboard_events("run-x") { // RunStarted, ConfigSnapshot, entries..., Completed
        <_ as crate::core::orchestration::es::log::EventLog>::append(&mut log, "run-x", &e).unwrap();
    }
    // Projeter deux fois.
    project_run(&db, "run-x").unwrap();
    project_run(&db, "run-x").unwrap();
    // Une seule ligne runs + orchestration_runs, pas de doublon d'entries.
    let run = crate::storage::queries::get_orchestration_run(&db, "run-x").unwrap().unwrap();
    assert_eq!(run.pattern, "blackboard");
    let history = crate::storage::queries::get_history(&db, None, 10).unwrap();
    assert_eq!(history.iter().filter(|r| r.run_id == "run-x").count(), 1);
}
```

Note : construire `sample_blackboard_events` en s'inspirant des events utilisés par les tests voisins (RunStarted + ConfigSnapshot + `ContributionAdded`/`BoardEntry*` selon les events blackboard réels — regarder `es/blackboard.rs` pour les noms exacts d'events). Le `config_json` du ConfigSnapshot doit se retrouver dans `orchestration_runs.config_json`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api,storage project_run_is_idempotent`
Expected: FAIL (`project_run` n'existe pas).

- [ ] **Step 3: Implement**

`project_run` : lit les events (`SqliteLog::new(db.clone()).events(run_id)`), `fold` → state, lit le `pattern` depuis le premier event `RunStarted` (ou un champ d'état), appelle `delete_projection_for_run(db, run_id)` puis le `record_*_es_into` correspondant au pattern avec `(db, run_id, &state, &config, &input, None, project.as_deref())` où `config` est **désérialisé depuis `state.config_json`** (fallback `Default` si `None`/vide) et `input`/`project` viennent de `RunStarted`. Écrire `delete_projection_for_run` dans `queries.rs` (6 `DELETE ... WHERE run_id = ?1`).

- [ ] **Step 4: Run tests** Run: `cargo test --no-default-features --features tui,providers-api,storage project_run`. Expected: PASS.

- [ ] **Step 5: Verify 3 modes clippy + fmt.**

- [ ] **Step 6: Commit**

```bash
git add src/cli/run_es_record.rs src/storage/queries.rs
git commit -m "feat(run): add idempotent project_run deriving flat tables from the event log"
```

---

### Task 3: Brancher le chemin run sur le projecteur

**Files:**
- Modify: `src/cli/run.rs` — dans les branches d'appel post-run (là où `record_blackboard_es`/`record_ring_es`/`record_orchestration_hierarchical` sont appelés, ~1256/1320/1385), remplacer l'appel `record_*` par `let _ = crate::cli::run_es_record::project_run(&db, &run_id);` **sous `#[cfg(feature = "storage")]`**, en réutilisant le handle `db` + `run_id` déjà disponibles. Le run projette désormais depuis le log persisté (les tables ne sont plus écrites directement depuis l'état runtime).
- Test: `src/cli/run.rs` (module `es_switch_tests`).

**Interfaces:**
- Consumes: `project_run` (Task 2).

- [ ] **Step 1: Write the failing test** — un run projette ses tables via le log.

Adapter/ajouter dans `es_switch_tests` (gated storage) un test `blackboard_es_run_projects_tables_from_log` : exécuter un run blackboard sous storage, puis vérifier via `queries::get_orchestration_run(&db, &run_id)` que la ligne existe avec `pattern == "blackboard"` — SANS avoir appelé `record_*` explicitement (la projection se fait dans le dispatch). Réutiliser l'idiome des tests voisins.

- [ ] **Step 2: Run test to verify it fails** (si le wiring n'est pas encore fait, ou si l'ancien record est retiré avant le projet). Run: `cargo test --no-default-features --features tui,providers-api,storage blackboard_es_run_projects_tables_from_log`.

- [ ] **Step 3: Implement** — remplacer les appels `record_*_es`/`record_orchestration_hierarchical` par `project_run(&db, &run_id)` dans les branches post-run des dispatch. Retirer les wrappers `record_blackboard_es`/`record_ring_es`/`record_orchestration_hierarchical` devenus inutilisés (ou les garder si encore appelés ailleurs — vérifier). NE PAS supprimer `record_*_es_into` (utilisés par `project_run`).

- [ ] **Step 4: Run tests** — toute la suite (storage + non-storage). Expected: vert, aucune régression History/Costs.

- [ ] **Step 5: Verify 3 modes clippy + fmt.**

- [ ] **Step 6: Commit**

```bash
git add src/cli/run.rs
git commit -m "feat(run): project flat tables from the event log after each run"
```

---

### Task 4: Commande `armadai projections rebuild`

**Files:**
- Create: `src/cli/projections.rs` — `pub async fn execute(args) -> anyhow::Result<()>` : pour `--run <id>` projette ce run ; pour `--all` (défaut) itère tous les `run_id` distincts de `execution_events` et projette chacun via `project_run`. Affiche un compte.
- Modify: `src/cli/mod.rs` — variante d'enum `Projections { ... }` (sous-commande `rebuild` avec `--run`/`--all`) + handler qui appelle `projections::execute`. Gated `#[cfg(feature = "storage")]`.
- Modify: `src/storage/queries.rs` — `pub fn all_event_log_run_ids(db: &Database) -> anyhow::Result<Vec<String>>` (`SELECT DISTINCT run_id FROM execution_events`).
- Test: `src/cli/projections.rs` (module de tests, gated storage).

**Interfaces:**
- Consumes: `project_run` (Task 2), `all_event_log_run_ids` (nouveau).

- [ ] **Step 1: Write the failing test** — rebuild reconstruit une table effacée.

```rust
#[test]
fn rebuild_reprojects_a_run_from_the_log() {
    let db = crate::storage::init_embedded().unwrap();
    // Persister un log + projeter une fois.
    // ... (log blackboard "run-y" via SqliteLog, project_run une fois)
    // Effacer la projection puis rebuild.
    crate::storage::queries::delete_projection_for_run(&db, "run-y").unwrap();
    assert!(crate::storage::queries::get_orchestration_run(&db, "run-y").unwrap().is_none());
    rebuild_run(&db, "run-y").unwrap(); // helper interne appelé par execute()
    assert!(crate::storage::queries::get_orchestration_run(&db, "run-y").unwrap().is_some());
}
```

- [ ] **Step 2: Run test to verify it fails** Run: `cargo test --no-default-features --features tui,providers-api,storage rebuild_reprojects_a_run_from_the_log`. Expected: FAIL.

- [ ] **Step 3: Implement** — `projections.rs` (exposant un helper `rebuild_run`/`rebuild_all` réutilisant `project_run`), la variante CLI dans `mod.rs`, `all_event_log_run_ids` dans queries. Suivre le pattern d'un fichier CLI existant (ex. `src/cli/history.rs`) pour la structure `execute` + parsing des flags via l'enum clap.

- [ ] **Step 4: Run tests** Run: `cargo test --no-default-features --features tui,providers-api,storage rebuild`. Expected: PASS.

- [ ] **Step 5: Verify 3 modes clippy + fmt** + smoke `cargo run --features tui,providers-api,storage -- projections rebuild --all` (doit s'exécuter sans planter, 0 runs OK).

- [ ] **Step 6: Commit**

```bash
git add src/cli/projections.rs src/cli/mod.rs src/storage/queries.rs
git commit -m "feat(cli): add \`armadai projections rebuild\` to re-derive tables from the log"
```

---

## Self-Review
- Couverture spec §7 : projecteur matérialisé (Tasks 2-3) ✓ ; `projections rebuild` (Task 4) ✓ ; config dans le log pour projection fidèle (Task 1) ✓ ; JSONL RunEvent déjà projection live (5a, hors 5b). Wiring History/Costs/web sur projections = 5c (les tables restent lues telles quelles ; 5b garantit qu'elles sont dérivées du log).
- Idempotence : Task 2 DELETE-avant-INSERT, testée.
- Cohérence types : `project_run(&Database, &str)`, `delete_projection_for_run(&Database, &str)`, `all_event_log_run_ids(&Database) -> Vec<String>`, `ConfigSnapshot { config_json: String }`, `ExecutionState.config_json: Option<String>` — cohérents entre tasks.
