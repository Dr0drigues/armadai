# C9 — Pattern mixing (hierarchical → blackboard/ring) + persistance hierarchical

> **Statut** : design validé (brainstorm 2026-07-20)
> **Cible** : axe 2, feature C9. La vue squad TUI event-based + audit UX/UI fait l'objet d'un **spec séparé, après C9** (elle consomme le flux d'événements standardisé ici).
> **Base** : `release/1.0.0`. Le pattern `Auto` (sélection de pattern au démarrage) et le routing modèle `latest:auto` (OH4) existent déjà et ne sont pas retouchés.

## 1. Objectif

Permettre à une **sous-équipe d'un pattern hierarchical** de s'exécuter en **blackboard** ou **ring** (au lieu d'une délégation plate), le résultat agrégé remontant à son lead puis au coordinateur. Persister le run hierarchical complet (aujourd'hui non persisté) et l'exposer dans le trace UI web (C6) et le flux JSONL headless (OH3).

Rétro-compatible : sans `pattern` sur une team, comportement **identique** à aujourd'hui.

## 2. Surface de configuration (déclaratif)

Une team déclare optionnellement son sous-pattern + des overrides, dans `orchestration.teams` (`armadai.yaml`) :

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: coord
  teams:
    - lead: research-lead
      agents: [searcher, analyst, critic]
      pattern: blackboard      # NEW — la team tourne en blackboard
      max_rounds: 4            # NEW — override optionnel (sinon défaut global)
    - lead: build-lead
      agents: [coder, reviewer]
      pattern: ring            # NEW
      max_laps: 2              # NEW
      consensus_threshold: 0.8 # NEW — override optionnel
    - lead: doc-lead
      agents: [writer]         # pas de `pattern` → délégation plate (comportement actuel)
```

- Nouveau type **`NestedPattern`** (enum `{ Blackboard, Ring }`, serde `rename_all = "lowercase"`). L'usage d'un enum dédié (≠ `OrchestrationPattern`) garantit **par construction** qu'on ne peut imbriquer ni `hierarchical`, ni `direct`, ni `auto` → une seule profondeur d'imbrication, pas de récursion.
- `TeamConfig` gagne :
  - `pattern: Option<NestedPattern>` (`#[serde(default)]`) — absent = délégation plate.
  - `max_rounds: Option<u32>`, `max_laps: Option<u32>`, `consensus_threshold: Option<f32>` (`#[serde(default)]`) — overrides par team ; si absents, les valeurs globales de `OrchestrationConfig` s'appliquent.
- **Validation** (`validate_config`) : un `pattern` sur une team n'est autorisé que si `OrchestrationConfig.pattern == Hierarchical`. Sinon nouvelle erreur `NestedPatternRequiresHierarchical(team_lead)`. Une team à sous-pattern doit avoir un `lead` non nul et ≥1 agent (réutiliser les invariants existants).

## 3. Sémantique d'exécution

Quand le coordinateur délègue une tâche à un **lead dont la team a un `pattern`** :

1. Le lead **ne fait pas** la boucle LLM+délégation libre habituelle.
2. Les agents de la team exécutent le sous-pattern (`run_blackboard` / `run_ring`) sur la tâche reçue, avec le **budget restant** partagé (voir §3.1).
3. **Arbitrage du lead** : une fois le sous-run terminé (consensus blackboard / votes ring), le lead reçoit le résultat agrégé (synthèse + éléments saillants : consensus, votes, confiances) et produit la **réponse finale de la team** via un appel LLM d'arbitrage — il peut **accepter, surcharger ou synthétiser**. Le lead reste « au-dessus » (il ne participe pas au sous-run lui-même) mais a le **droit de trancher**.
4. Cette réponse remonte au coordinateur comme réponse du lead. Le coordinateur conserve son propre arbitrage au niveau supérieur (comportement hierarchical existant).

### 3.1 Budget, limites, profondeur (partagés)

- Le sous-run reçoit le **budget restant** (`token_budget`/`cost_limit` moins ce qui est déjà consommé par l'`EngineState` hierarchical).
- À la fin du sous-run, ses métriques (`total_tokens_in`, `total_tokens_out`, `total_cost`, `invocation_count`) sont **repliées** dans l'`EngineState` hierarchical.
- La profondeur du sous-run compte dans `max_depth` (le lead est à `depth`, le sous-run s'exécute « à `depth + 1` » conceptuellement — on vérifie `depth + 1 < max_depth` avant de lancer).
- Le tour d'arbitrage du lead incrémente `iteration_count` comme un appel agent normal.

### 3.2 Point d'intégration (code)

Dans `hierarchical.rs::invoke_agent` : avant la boucle LLM standard, tester si `agent_name` est le `lead` d'une team ayant `pattern: Some(_)`. Si oui → brancher sur une nouvelle fonction `run_nested_team(...)` qui :
- résout les agents de la team (providers déjà dans `EngineContext`),
- construit la config du sous-pattern (défauts globaux + overrides team),
- appelle `run_blackboard`/`run_ring` avec le **sink partagé**,
- replie métriques + trace,
- déclenche l'arbitrage du lead,
- renvoie la réponse arbitrée.

Le sous-pattern réutilise **tel quel** les moteurs existants (`blackboard::run_blackboard`, `ring::run_ring`) — pas de duplication de logique.

## 4. Événements (sink partagé)

- Les `RunEvent::Delegate` restent émis (coordinateur → lead).
- Nouveaux événements **`RunEvent::NestedStart { team_lead: String, pattern: String }`** et **`RunEvent::NestedEnd { team_lead: String }`** encadrant le sous-run, pour matérialiser la frontière côté consommateurs JSONL/TUI.
- Les `Board`/`Vote`/`Contribution` du sous-run traversent le **même sink** (déjà le cas dès qu'on passe le sink partagé à `run_blackboard`/`run_ring`).
- Clés courtes JSONL cohérentes avec l'existant (`t:"nested_start"`, `t:"nested_end"`).

## 5. Storage — mécanisme de migration + persistance

### 5.1 Migration (brique manquante)

`schema.rs` n'a aujourd'hui **aucun versioning** (`CREATE TABLE IF NOT EXISTS` uniquement) : la contrainte `CHECK (pattern IN ('direct','blackboard','ring'))` est figée et ne peut être relâchée sur une base existante.

- Introduire un versioning via **`PRAGMA user_version`**. `apply(conn)` : (1) crée les tables si absentes (schéma de base = version courante pour une base neuve), (2) appelle `migrate(conn)` qui applique séquentiellement les steps jusqu'à `SCHEMA_VERSION`.
- **Migration → v1** (pour bases pré-existantes ET intégrée au schéma neuf) :
  - Rebuild de `orchestration_runs` (SQLite ne permet pas d'`ALTER` un CHECK) : créer `orchestration_runs_new` avec `CHECK (pattern IN ('direct','blackboard','ring','hierarchical'))` + colonne `parent_run_id TEXT` (nullable) → copier les données → `DROP` l'ancienne → `RENAME`. Les FK ne sont pas enforced (PRAGMA foreign_keys omis) donc le rebuild est sûr.
  - Créer la table `delegation_events`.
- Index : `CREATE INDEX IF NOT EXISTS idx_orch_parent ON orchestration_runs(parent_run_id)`.

### 5.2 Schéma cible

```sql
-- orchestration_runs : CHECK relâché + parent_run_id
pattern TEXT NOT NULL CHECK (pattern IN ('direct','blackboard','ring','hierarchical'))
parent_run_id TEXT            -- NULL pour un run racine ; sinon run_id du parent hierarchical

-- delegation_events (hierarchical)
CREATE TABLE IF NOT EXISTS delegation_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL REFERENCES orchestration_runs(run_id),
    seq        INTEGER NOT NULL,           -- ordre d'émission
    from_agent TEXT NOT NULL,
    to_agent   TEXT NOT NULL,
    message    TEXT NOT NULL,
    depth      INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_delegation_events_run ON delegation_events(run_id, seq);
```

### 5.3 Enregistrement

- Nouvelle query `record_orchestration_hierarchical(db, result, config, input)` : insère une ligne `runs` + une ligne `orchestration_runs` (pattern `hierarchical`, `parent_run_id = NULL`) + les `delegation_events` (depuis `OrchestrationResult.trace`).
- Chaque **sous-run** blackboard/ring est enregistré via les queries existantes (`record_orchestration_blackboard`/`_ring`) **avec `parent_run_id`** = run_id du run hierarchical. → nécessite d'ajouter un paramètre `parent_run_id: Option<&str>` aux fonctions record existantes (défaut `None` = comportement actuel).
- Le câblage se fait dans `cli/run.rs` (branche `hierarchical`), symétrique aux branches blackboard/ring existantes.

## 6. C6 — trace UI web

- **Liste** (`/api/orchestration/trace`) : inclut désormais les runs `hierarchical`. Ne lister que les runs **racines** (`parent_run_id IS NULL`) pour éviter le bruit ; les sous-runs apparaissent dans le détail du parent.
- **Détail** (`/api/orchestration/trace/{run_id}`) : pour un run hierarchical, renvoyer en plus `delegation_events` + la liste des **sous-runs enfants** (chacun avec ses board_entries/ring_contributions/ring_votes).
- **SPA** (`index.html`) :
  - run hierarchical → diagramme mermaid de l'**arbre de délégation** (à partir des `delegation_events` : `from -->|msg| to`), + sections **sous-runs dépliables** réutilisant le rendu blackboard/ring existant.
  - timeline imbriquée : délégations + (au clic/dépli) contributions du sous-run.
  - Cas vide (run sans délégation, sous-run sans entrée) géré proprement. Thème light/dark réutilisé.

## 7. OH3 — headless JSONL

- `NestedStart`/`NestedEnd` sérialisés dans le flux JSONL (`--headless --json`) avec clés courtes.
- Vérifier que les événements du sous-run (`board`/`vote`/`contribution`) sont bien émis en mode headless (ils le sont dès que le sink partagé est passé).
- Non-régression : sans sous-pattern, le flux JSONL hierarchical est inchangé.

## 8. Découpage en lots

- **Lot 1 — Moteur** : `NestedPattern`, champs `TeamConfig`, validation, `run_nested_team` (exécution imbriquée + budget/profondeur partagés + arbitrage du lead), événements `NestedStart`/`NestedEnd`. Testable isolément (providers mock). *Aucune dépendance storage/web.*
- **Lot 2 — Storage** : mécanisme `user_version` + migration v1 (rebuild CHECK + `parent_run_id` + `delegation_events`), `record_orchestration_hierarchical`, paramètre `parent_run_id` sur les record blackboard/ring, câblage `cli/run.rs`.
- **Lot 3 — Exposition** : endpoint détail hierarchical + arbre + sous-runs (C6 web), rendu SPA, `NestedStart/End` dans le JSONL headless.

Chaque lot = une PR vers `release/1.0.0`, revue indépendante avant merge.

## 9. Tests

- **Lot 1** : désérialisation YAML `pattern`/overrides sur une team ; validation (rejet `pattern` hors hierarchical) ; exécution imbriquée blackboard et ring (providers mock) → résultat agrégé remonté ; arbitrage du lead (le lead peut surcharger le consensus) ; budget partagé (le sous-run reçoit le restant, métriques repliées) ; profondeur (`max_depth` respecté) ; événements `NestedStart`/`NestedEnd` émis ; non-régression délégation plate (team sans `pattern`).
- **Lot 2** : migration d'une base « v0 » (avec l'ancien CHECK) → v1 sans perte de données ; base neuve = v1 directement ; insertion d'un run hierarchical + delegation_events relus ; sous-run avec `parent_run_id` retrouvé par parent ; `user_version` correctement posé (idempotence de `apply`).
- **Lot 3** : endpoint détail hierarchical (run + delegation_events + sous-runs) ; liste ne montre que les racines ; `cargo build --release` (SPA embarquée) ; JSONL contient `nested_start`/`nested_end` en mode headless.

## 10. Contraintes CI (tous lots)

- Clippy 2 modes : `--no-default-features --features tui -- -D warnings` ET `--features tui,providers-api -- -D warnings`.
- Lot 2/3 touchent storage/web : ajouter `--no-default-features --features tui,web,storage -- -D warnings` + `cargo build --release`.
- `cargo fmt -- --check` ; `cargo test` dans les modes pertinents (storage : `--features tui,storage` ; web : `--features tui,web,storage`).

## 11. Hors scope (C9)

- **Vue squad TUI event-based + audit UX/UI** → spec séparé, après C9.
- Imbrication récursive (blackboard-dans-blackboard, hierarchical-dans-hierarchical) — interdite par construction (`NestedPattern`).
- Routing dynamique d'agents (C8) — feature distincte.
- Persistance des runs `direct` déjà couverte par l'existant ; pas de refonte.
