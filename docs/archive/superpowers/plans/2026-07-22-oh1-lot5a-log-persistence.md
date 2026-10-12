# OH1 Lot 5a — Persistance du log event-sourcé Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persister le log `ExecutionEvent` du chemin `run` dans `execution_events` au fil de l'eau (sous `storage`), avec un `run_id` unifié entre le log et les tables plates.

**Architecture:** Le socle est déjà là (table `execution_events` + schema v3, `SqliteLog`, `InMemoryLog`, trait `EventLog`, `SinkProjectingLog<L: EventLog>` générique). Deux changements : (1) les fonctions `record_*_es[_into]` reçoivent un `run_id` du dispatch au lieu d'en générer un ; (2) chaque `dispatch_*_es` exécute la boucle ES via un helper générique sur `L: EventLog`, appelé avec un `SqliteLog` persistant sous `storage` (append transactionnel par event) et un `InMemoryLog` éphémère sinon. En 5a les tables plates restent écrites par `record_*_es` (transitionnel) ; le projecteur qui les dérivera du log est 5b.

**Tech Stack:** Rust edition 2024, rusqlite (feature `storage`), tokio, serde.

## Global Constraints

- CI clippy **3 modes** doit passer : `--no-default-features --features tui`, `--features tui,providers-api`, `--features tui,web,storage`. Chacun avec `-D warnings`.
- `cargo fmt -- --check` propre.
- Les **~899 tests unitaires + 30 e2e** actuels restent verts (aucune régression).
- Persistance du log **gated `storage`** ; sans `storage`, exécution en mémoire (log éphémère), aucune écriture DB. Le cœur (log/reducer/moteurs) compile toujours (edition 2024).
- `run_id(log) == run_id(tables plates)` pour un run donné (prérequis du projecteur 5b).
- Append du log **transactionnel par event** (durabilité : un crash laisse un log cohérent jusqu'au dernier event commité). `SqliteLog::append` fait déjà un INSERT par event — le préserver.
- Changement user-facing (persistance) → PR + revue indépendante, **validation Dimitri avant merge**.

---

### Task 1: Unifier le `run_id` — les record_*_es reçoivent le run_id du dispatch

**Files:**
- Modify: `src/cli/run_es_record.rs` — `record_blackboard_es_into` (~136), `record_ring_es_into` (~212) : remplacer `let run_id = uuid::Uuid::new_v4().to_string();` par un paramètre `run_id: &str`.
- Modify: `src/cli/run.rs` — `record_hierarchical_into` (~1735) : idem (paramètre `run_id`), et son appelant `record_orchestration_hierarchical` (~1830) ; wrappers `record_blackboard_es`/`record_ring_es` (~1631/1660) : ajouter `run_id: &str` et le forwarder ; sites d'appel dans les dispatch (`record_blackboard_es(&state, ...)` ~1256, `record_ring_es(...)` ~1320, `record_orchestration_hierarchical(...)` ~1385) : passer le `run_id` du dispatch.
- Test: `src/cli/run_es_record.rs` (module de tests existant, ~490+).

**Interfaces:**
- Consumes: `ExecutionState`, `BlackboardConfig`, `RingConfig`, `crate::storage::Database`.
- Produces (nouvelles signatures — utilisées par Task 2) :
  - `record_blackboard_es_into(db: &Database, run_id: &str, state: &ExecutionState, config: &BlackboardConfig, input: &str, parent_run_id: Option<&str>, project: Option<&str>) -> anyhow::Result<String>` (retourne `run_id` inchangé).
  - `record_ring_es_into(db: &Database, run_id: &str, state: &ExecutionState, config: &RingConfig, input: &str, parent_run_id: Option<&str>, project: Option<&str>) -> anyhow::Result<String>`.
  - `record_hierarchical_into(db: &Database, run_id: &str, result: &OrchestrationResult, config: &OrchestrationConfig, input: &str, project: Option<&str>) -> anyhow::Result<String>`.
  - Wrappers : `record_blackboard_es(run_id: &str, state, config, input, project)`, `record_ring_es(run_id: &str, state, config, input, project)`, `record_orchestration_hierarchical(run_id: &str, result, config, input, project)`.

- [ ] **Step 1: Write the failing test** — le run_id passé est celui persisté.

Dans le module `#[cfg(all(test, feature = "storage"))] mod tests` de `run_es_record.rs`, ajouter :

```rust
#[test]
fn record_blackboard_es_into_uses_caller_run_id() {
    let db = crate::storage::init_embedded().unwrap();
    // Un état blackboard minimal complété (réutiliser le helper de construction
    // d'état des tests voisins de ce module — même idiome que
    // `record_blackboard_es_into_persists_run_and_entries`).
    let state = sample_blackboard_state();
    let cfg = BlackboardConfig::default();
    let returned = record_blackboard_es_into(
        &db,
        "fixed-run-id-123",
        &state,
        &cfg,
        "task",
        None,
        None,
    )
    .unwrap();
    assert_eq!(returned, "fixed-run-id-123");
    // La ligne persistée porte bien ce run_id.
    let run = crate::storage::queries::get_orchestration_run(&db, "fixed-run-id-123")
        .unwrap()
        .unwrap();
    assert_eq!(run.run_id, "fixed-run-id-123");
}
```

Note : si aucun helper `sample_blackboard_state()` n'existe, construire l'état inline via `fold(&[...])` comme le font les tests voisins de ce module (regarder `record_blackboard_es_into_persists_run_and_entries` et copier son montage d'état).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api,storage record_blackboard_es_into_uses_caller_run_id`
Expected: FAIL — la signature actuelle n'accepte pas de `run_id` (erreur de compilation : arité).

- [ ] **Step 3: Implement** — ajouter `run_id: &str` en 2ᵉ position, supprimer la génération interne.

Dans `record_blackboard_es_into` : supprimer `let run_id = uuid::Uuid::new_v4().to_string();` et ajouter le paramètre `run_id: &str` juste après `db`. Idem `record_ring_es_into` et `record_hierarchical_into`. Mettre à jour les wrappers pour prendre/forwarder `run_id`, et tous les autres tests existants du module qui appellent ces fonctions (leur passer un run_id littéral, ex. `"test-run"`).

- [ ] **Step 4: Run tests** — le nouveau + tous les tests du module.

Run: `cargo test --no-default-features --features tui,providers-api,storage record_`
Expected: PASS (tous verts).

- [ ] **Step 5: Verify les 3 modes clippy + fmt** (le changement de signature peut casser des appelants gated).

Run:
```
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo fmt -- --check
```
Expected: 0 warning, fmt propre.

- [ ] **Step 6: Commit**

```bash
git add src/cli/run_es_record.rs src/cli/run.rs
git commit -m "refactor(run): thread caller run_id into record_*_es (unify log/table run_id)"
```

---

### Task 2: Persister le log au fil de l'eau sous `storage` (SqliteLog dans le dispatch)

**Files:**
- Modify: `src/cli/run.rs` — les 4 `dispatch_*_es` (`dispatch_direct_es` ~624, `dispatch_blackboard_es` ~1501, `dispatch_ring_es` ~1542, `dispatch_hierarchical_es` ~1589) et `run_single_agent_es` (~699). Introduire un helper générique qui exécute la boucle ES sur un `L: EventLog` fourni, appelé avec `SqliteLog` sous `storage` sinon `InMemoryLog`.
- Test: `src/cli/run.rs` module de tests d'intégration ES (le module documenté « Integration-style tests for OH1 Lot 5 »).

**Interfaces:**
- Consumes: `SinkProjectingLog<L>`, `InMemoryLog`, `SqliteLog` (`#[cfg(feature="storage")]`), `EventLog`, `run_id` (Task 1), les fonctions record de Task 1.
- Produces: comportement — sous `storage`, après un run, `execution_events[run_id]` contient les events du run (folde vers le même `ExecutionState` que le run) ; le `run_id` du log == celui des tables plates.

- [ ] **Step 1: Write the failing test** — un run blackboard persiste son log.

Dans le module de tests d'intégration ES de `run.rs` (là où vivent `blackboard_es_state_is_recorded_via_record_blackboard_es_into` etc.), ajouter, gated `#[cfg(feature = "storage")]` :

```rust
#[tokio::test]
async fn blackboard_es_run_persists_event_log() {
    // Même montage que `blackboard_es_state_is_recorded_via_record_blackboard_es_into` :
    // providers scriptés + config, mais on assure que le dispatch utilise un
    // SqliteLog persistant et on relit execution_events ensuite.
    let db = crate::storage::init_embedded().unwrap();
    let run_id = "it-bb-log-1";
    // Construire le SinkProjectingLog<SqliteLog> comme le fait le dispatch sous storage,
    // exécuter run_blackboard_es (boucle), puis :
    let log = crate::core::orchestration::es::log::SqliteLog::new(db.clone());
    let events = <_ as crate::core::orchestration::es::log::EventLog>::events(&log, run_id).unwrap();
    assert!(!events.is_empty(), "le run doit avoir persisté ses events");
    // Fold → state cohérent (au moins un RunStarted en tête).
    use crate::core::orchestration::es::event::ExecutionEvent;
    assert!(matches!(events[0], ExecutionEvent::RunStarted { .. }));
}
```

Note : suivre exactement l'idiome du test voisin `blackboard_es_state_is_recorded_via_record_blackboard_es_into` pour construire providers/config/état et invoquer la boucle — ne pas inventer une API. Le test doit démontrer que, quand le dispatch tourne sous `storage`, les events atterrissent dans `execution_events`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --no-default-features --features tui,providers-api,storage blackboard_es_run_persists_event_log`
Expected: FAIL — aujourd'hui le dispatch utilise `InMemoryLog`, donc `execution_events` reste vide (assert `!events.is_empty()` échoue).

- [ ] **Step 3: Implement** — helper générique + branche storage.

Extraire la boucle commune en un helper générique (nom suggéré `run_es_loop`) paramétré par `L: EventLog`, qui prend le `SinkProjectingLog<L>`, le `run_id`, les inputs du pattern, exécute `run_event_sourced`/`run_*_es`, et renvoie l'`ExecutionState` (via `log.events(run_id)` + fold, comme aujourd'hui). Puis dans chaque `dispatch_*_es`, remplacer la construction en dur :

```rust
// AVANT :
let mut log = SinkProjectingLog::with_meta(InMemoryLog::default(), &filtered_sink, agent_meta);

// APRÈS (esquisse) :
#[cfg(feature = "storage")]
let db = crate::storage::init_db().ok();
#[cfg(feature = "storage")]
let state = if let Some(db) = db.as_ref() {
    let log = SinkProjectingLog::with_meta(
        crate::core::orchestration::es::log::SqliteLog::new(db.clone()),
        &filtered_sink,
        agent_meta.clone(),
    );
    run_es_loop(log, &run_id, /* inputs du pattern */).await?
} else {
    let log = SinkProjectingLog::with_meta(InMemoryLog::default(), &filtered_sink, agent_meta.clone());
    run_es_loop(log, &run_id, /* inputs */).await?
};
#[cfg(not(feature = "storage"))]
let state = {
    let log = SinkProjectingLog::with_meta(InMemoryLog::default(), &filtered_sink, agent_meta);
    run_es_loop(log, &run_id, /* inputs */).await?
};
```

Puis appeler la fonction record (Task 1) avec le **même** `db` et le **même** `run_id` (sous `storage`), au lieu du wrapper qui ré-init la DB — de sorte que log et tables plates partagent connexion et run_id. Adapter les 4 dispatch + `run_single_agent_es`. `Database` doit être clonable (c'est un handle `Arc<Mutex<Connection>>` — vérifier `.clone()`).

- [ ] **Step 4: Run tests** — le nouveau + toute la suite (non-régression).

Run:
```
cargo test --no-default-features --features tui,providers-api,storage
cargo test --no-default-features --features tui,providers-api
```
Expected: tout vert, y compris `blackboard_es_run_persists_event_log`.

- [ ] **Step 5: Verify les 3 modes clippy + fmt**

Run:
```
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo fmt -- --check
```
Expected: 0 warning, fmt propre.

- [ ] **Step 6: Commit**

```bash
git add src/cli/run.rs
git commit -m "feat(run): persist the ES event log to execution_events under storage"
```

---

## Notes de vérification
- Après un run réel (storage on), `execution_events` doit contenir les events ET la ligne `orchestration_runs` doit porter le même `run_id`.
- Ne PAS supprimer les écritures des tables plates en 5a (elles restent la source de lecture jusqu'à 5b). 5a = ajouter la persistance du log + unifier run_id.
- Piège clippy `tui,web,storage` : ce mode compile web + storage ensemble ; vérifier qu'aucun `#[cfg]` n'oublie une combinaison (les branches storage/non-storage doivent toutes compiler).
