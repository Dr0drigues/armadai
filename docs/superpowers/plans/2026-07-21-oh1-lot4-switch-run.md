# OH1 — Lot 4 (bascule du chemin `run` sur les moteurs ES) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Basculer le chemin d'exécution `run` sur les moteurs event-sourcés (direct/hierarchical/blackboard/ring), **en séquentiel**, en préservant l'observabilité (`RunEvent`), l'affichage, le storage et les codes de sortie — sans supprimer le legacy (retiré en PR séparée après validation manuelle).

**Architecture:** Un wrapper `SinkProjectingLog<L>` mappe chaque `ExecutionEvent` appendé → `RunEvent` émis au sink (préserve l'observabilité live + anticipe la projection JSONL du Lot 5). Un extracteur `ExecutionState(+log) → OrchestrationResult` + des fonctions `record_*_es` natives alimentent affichage/storage. `run.rs` appelle `run_{direct,hierarchical,blackboard,ring}_es` au lieu des moteurs legacy.

**Tech Stack:** Rust edition 2024, socle ES (Lots 1-3, mergés).

## Global Constraints

- Base = `origin/release/1.0.0` (@ 6792bdf, Lots 1-3 mergés). Branche `feat/oh1-lot4-switch-run`, PR vers `release/1.0.0`.
- **Bascule SÉQUENTIELLE** (pas de `tokio::spawn` ; le parallélisme est un lot ultérieur — décision Dimitri 2026-07-21).
- **Legacy NON supprimé dans ce lot** (garde compilable pour rollback trivial ; suppression = PR séparée T7 après validation manuelle). Les moteurs ES restent la source ; legacy devient mort mais présent.
- **★ Lot user-facing** : change le comportement de `run` → **validation manuelle Dimitri AVANT merge** (checkpoint après T5).
- Keying des `BTreeMap<String,Agent>` passés aux `run_*_es` = **roster key** (slug fichier, comme `run.rs:1011`), jamais H1 ni `a.name()`.
- Codes de sortie inchangés : halt budget/cost → `Ok` → exit 0 (halt gracieux, OH3) ; aucune `Err` ES contenant "budget" ne doit fuir vers `exit_code_for`.
- Clippy `-D warnings` en `--features tui` ET `--features tui,providers-api` (+ `tui,storage` pour les record) + fmt. Suite complète verte à chaque task.
- Carte de portage validée ; arbitrages tranchés (voir Décisions).

### Décisions d'arbitrage (tranchées)
- **Observabilité** : wrapper `SinkProjectingLog` (ExecutionEvent→RunEvent), moteurs ES inchangés.
- **Extracteur** : `to_orchestration_result` (hierarchical) ; `record_blackboard_es_into`/`record_ring_es_into` natives (blackboard/ring). `nested_runs = []` au Lot 4 (documenté).
- **max_depth** : garde à la source dans le decider hierarchical (pas d'`Invoke` enfant à `depth+1 > max_depth`).
- **snapshot blackboard** : réintroduire `take(10)`.
- **Régressions acceptées** (montrées au checkpoint) : nested C9 non persistés en sous-tables ; tokens par-entrée blackboard/ring = 0 ; colonne `status` blackboard `completed` au lieu de `halted`.
- **`--pipe`** (multi-agent séquentiel hors orchestration) : inchangé, reste hors ES.
- **cost_limit** blackboard/ring standalone : threadé via nouveau param des `run_*_es` SI le legacy l'appliquait (à vérifier en T3) ; sinon documenté comme non applicable.

---

### Task 1: `SinkProjectingLog` (ExecutionEvent → RunEvent) + extracteur `to_orchestration_result`

**Files:** Create `src/core/orchestration/es/bridge.rs` (ou `es/result.rs`); modify `es/mod.rs`.

**Interfaces:**
- Consumes: `es::log::EventLog`, `es::event::ExecutionEvent`, `es::state::{ExecutionState, fold}`, `crate::core::events::{RunEvent, EventSink}` (vérifie les vrais chemins/variants de RunEvent — OH3), `crate::core::orchestration::hierarchical::{OrchestrationResult, DelegationEvent}` (types legacy réutilisés pour l'affichage/storage).
- Produces:
  - `struct SinkProjectingLog<'s, L: EventLog> { inner: L, sink: &'s dyn EventSink }` + `impl EventLog` : `append` appelle `inner.append` PUIS `map_execution_to_run_events(event)` → émet chaque `RunEvent` via `sink`. `events()` délègue à `inner`.
  - `fn map_execution_to_run_events(e: &ExecutionEvent) -> Vec<RunEvent>` (pur) : `AgentInvoked`→`AgentStart` ; `AgentObserved`→`AgentEnd` ; `Delegated`→`Delegate` ; `BoardEntryAdded`→`Board` ; `VoteCast`→`Vote` ; `NestedStarted`→`NestedStart` ; `NestedEnded`→`NestedEnd` ; `ModelRouted`→`Route` ; `Completed`→`Result`(ou géré par run.rs) ; autres→[]. Mappe les champs fidèlement aux variants RunEvent existants.
  - `fn to_orchestration_result(state: &ExecutionState, events: &[ExecutionEvent]) -> OrchestrationResult` : content = dernier `Completed` ; total_tokens_in/out = `budget_*` (u64→u32 saturant) ; total_cost ; trace = `hier.trace`→`Vec<DelegationEvent>` ; invocation_count = nb `AgentInvoked` ; `nested_runs = vec![]`.

- [ ] **Step 1: Write the failing tests** — (a) `map_execution_to_run_events` sur chaque variant → RunEvent attendu ; (b) `SinkProjectingLog` : un sink de capture (Vec) reçoit les bons RunEvent dans l'ordre d'append, et `events()` renvoie bien les ExecutionEvent ; (c) `to_orchestration_result` sur un log hierarchical synthétique → content/tokens/trace/invocation_count attendus, nested_runs vide.
- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::bridge` → FAIL.
- [ ] **Step 3: Implement** — inspecte `crate::core::events` (RunEvent variants + EventSink trait réels) et adapte le mapping. Ajoute `pub mod bridge;` (ou `result`) à `es/mod.rs`.
- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean.
- [ ] **Step 5: Commit** `git commit -m "feat(es): SinkProjectingLog (ExecutionEvent->RunEvent) + to_orchestration_result extractor"`

---

### Task 2: `run_direct_es` + `DirectDecider`

**Files:** Create `src/core/orchestration/es/direct.rs`; modify `es/mod.rs`.

**Interfaces:**
- Produces:
  - `struct DirectDecider { agent: String, input: String, routing_rules: RoutingRules, agents: BTreeMap<String,Agent> }` + `impl Decider` : état vide → (`Emit(ModelRouted)` si `latest:auto`) + `Invoke{agent, input}` ; agent observé → `Complete{content: dernier AgentObserved}`.
  - Réutilise un EffectRunner : soit `HierarchicalEffectRunner` (si utilisable pour 1 agent), soit un `DirectEffectRunner` minimal (provider.complete + routing `latest:auto`/fallback repris de `run_single_agent`).
  - `pub async fn run_direct_es(run_id, agent: &str, input, agents, providers, routing_rules, log) -> anyhow::Result<ExecutionState>`.
- [ ] **Step 1: Write the failing test** — mock provider ; `run_direct_es` → log `RunStarted{pattern:"direct"}`→`AgentInvoked`→`AgentObserved`→`Completed`, status Completed, content = réponse.
- [ ] **Step 2: Run to verify fail** — `es::direct` → FAIL.
- [ ] **Step 3: Implement** — DirectDecider + effect + run_direct_es. Hermétique (modèles concrets en test).
- [ ] **Step 4: Run + clippy 2 modes + fmt** → PASS/clean.
- [ ] **Step 5: Commit** `git commit -m "feat(es): run_direct_es + DirectDecider (single-agent)"`

---

### Task 3: Réconciliation des divergences (max_depth, snapshot cap, cost_limit)

**Files:** Modify `es/hierarchical.rs` (garde max_depth à la source), `es/blackboard.rs` (cap snapshot 10 + éventuel cost_limit), `es/ring.rs` (éventuel cost_limit).

**Interfaces:**
- max_depth : dans le `HierarchicalDecider`, filtrer les délégations dont `depth+1 > max_depth` AVANT d'émettre `Delegated`+`Invoke` (garde à la source) ; le run halte alors sans invoquer l'enfant trop profond (fidèle legacy). Ajuste/ajoute les tests concernés (le test `es_max_depth_halts_gracefully` doit refléter que l'enfant à la profondeur limite n'est PAS invoqué).
- snapshot : dans `BlackboardEffectRunner::build_prompt`, tronquer aux 10 entries les plus récentes (`round < current`) — `rev().take(10)` comme le legacy (`llm_agents.rs:377`).
- cost_limit : VÉRIFIE d'abord dans le legacy `run_blackboard`/`run_ring` si un cost_limit est appliqué (source de `RingOutcome::CostLimitExceeded`). Si oui, ajoute un param `cost_limit: Option<f64>` aux `run_blackboard_es`/`run_ring_es` + garde dans le decider. Si non, documente que non applicable.

- [ ] **Step 1: Write/adjust the failing tests** — (a) max_depth : chaîne dépassant max_depth → l'agent à `depth == max_depth+? ` n'est PAS invoqué (assert call_count=0) et le run Complete avec Warned ; (b) snapshot > 10 entries → le prompt ne contient que les 10 plus récentes ; (c) cost_limit (si applicable) → breach → Warned+Complete.
- [ ] **Step 2: Run to verify fail** — cibles → FAIL.
- [ ] **Step 3: Implement** les 3 réconciliations. Attention à ne pas casser les tests existants (ajuste ceux qui encodaient l'ancien comportement max_depth/+1, en documentant le changement voulu).
- [ ] **Step 4: Run suite complète + clippy 2 modes + fmt** → PASS/clean.
- [ ] **Step 5: Commit** `git commit -m "feat(es): reconcile max_depth (guard-at-source), blackboard snapshot cap, cost_limit"`

---

### Task 4: Fonctions `record_*_es` natives (storage) + affichage depuis `ExecutionState`

**Files:** Modify `src/cli/run.rs` (nouvelles fn record ES-native + helpers d'affichage), gated `#[cfg(feature="storage")]` pour les record.

**Interfaces:**
- Consumes: `ExecutionState`, extracteur (T1), les fn storage bas-niveau existantes (`insert_run_with_id`, `insert_orchestration_run`, `insert_board_entry`, `insert_ring_contribution`, `insert_ring_vote`, `insert_delegation_event` — vérifie noms réels dans `queries.rs`/`run.rs`).
- Produces:
  - `record_blackboard_es_into(state, config, project, run_id, ...) ` : écrit `runs` (tokens = budget, cost, status) + `orchestration_runs` meta + `board_entries` depuis `state.board.entries` (tokens par-entrée = 0, documenté ; kind depuis `BoardEntryRec.kind`+refs).
  - `record_ring_es_into(state, config, ...)` : `runs` + meta + `ring_contributions` depuis `state.ring.contributions` (tokens 0) + `ring_votes` depuis `state.ring.votes`.
  - Helpers d'affichage : `blackboard_display(state) -> String` (concat `[agent] content` des entries) et `ring_display(state) -> String` (depuis `OutcomeResolved`/`Completed`), reproduisant l'affichage legacy.
- [ ] **Step 1: Write the failing tests** — (storage) un `ExecutionState` blackboard/ring synthétique → `record_*_es_into` écrit les bonnes lignes (query de vérif) ; affichage → chaîne attendue.
- [ ] **Step 2: Run to verify fail** — `cargo test --features tui,storage ... record_.*_es` → FAIL.
- [ ] **Step 3: Implement** — réutilise les fn `insert_*` bas-niveau existantes. Ne PAS reconstruire `Board`/`RingToken`.
- [ ] **Step 4: Run tests (tui,storage) + clippy 3 modes + fmt** → PASS/clean.
- [ ] **Step 5: Commit** `git commit -m "feat(cli): ES-native record + display for blackboard/ring runs"`

---

### Task 5: Bascule `run.rs` — les 4 patterns

**Files:** Modify `src/cli/run.rs` (branches direct/hierarchical/blackboard/ring).

**Interfaces:** consomme T1-T4. Chaque branche : construit `BTreeMap<String,Agent>` (roster key) + providers, un `InMemoryLog` enveloppé dans `SinkProjectingLog{inner, sink}`, appelle `run_{pattern}_es(...)`, puis extracteur/record/affichage/`RunEvent::Result`/exit code.
- T5a **direct** (`run_inner` chemin séquentiel `run.rs:177-241` / `run_single_agent`) → `run_direct_es` + extracteur + `record_run` + `Result`.
- T5b **hierarchical** (`run.rs:976`) → `run_hierarchical_es` + `to_orchestration_result` + `record_orchestration_hierarchical` (nested_runs vide) + `Result`.
- T5c **blackboard** (`run.rs:820`) → `run_blackboard_es` + `record_blackboard_es_into` + `blackboard_display` + `Result`.
- T5d **ring** (`run.rs:872`) → `run_ring_es` + `record_ring_es_into` + `ring_display` + `Result`. Keying `agent_order` = roster key (vérifier vs `a.name()` legacy).

- [ ] **Step 1: Write the failing tests** — tests d'intégration `run.rs` (mock providers) par pattern : mode humain (stdout) + `--json` headless (JSONL RunEvent via SinkProjectingLog) + exit codes + storage. Vérifie que les `RunEvent` d'observabilité sont bien émis (AgentStart/End/Delegate/Board/Vote/Nested).
- [ ] **Step 2: Run to verify fail** — FAIL (branches encore legacy).
- [ ] **Step 3: Implement** — bascule chaque branche. Legacy reste dans le fichier (mort) mais n'est plus appelé. Ne PAS supprimer encore.
- [ ] **Step 4: Run suite complète (tui, tui+providers-api, tui+storage) + clippy 3 modes + fmt** → PASS/clean.
- [ ] **Step 5: Commit** `git commit -m "feat(cli): switch run path to event-sourced engines (4 patterns, sequential)"`

---

### ★ CHECKPOINT — Validation manuelle Dimitri (AVANT T6/T7 et AVANT merge)

`armadai run` réel sur les 4 patterns (direct, hierarchical, blackboard, ring), mode humain ET `--json` headless : stdout, JSONL/RunEvent, exit codes, History/Costs (storage). Je remets la branche à Dimitri avec un prompt de test par pattern. Les régressions acceptées (nested non persistés, tokens par-entrée 0, status blackboard) sont signalées. **Ni suppression legacy ni merge avant son feu vert.**

---

### Task 6: (après validation) Nettoyage — retirer le legacy des chemins de test/e2e

**Files:** `e2e_tests.rs`, `gemini_integration_tests.rs`, `llm_agents.rs` (tests appelant les moteurs legacy).
- [ ] Migrer/retirer les tests e2e qui exercent `run_blackboard`/`run_ring`/`HierarchicalEngine` legacy vers les moteurs ES (ou supprimer si redondants avec les tests ES). Suite verte.
- [ ] Commit `test: migrate e2e/integration tests off legacy engines`

---

### Task 7: (PR SÉPARÉE, après validation + T6) Suppression des moteurs legacy

**Files:** `hierarchical.rs`, `blackboard.rs`, `ring.rs` (racine), `llm_agents.rs`, `run.rs` (record functions mortes).
- [ ] Supprimer `HierarchicalEngine`/`EngineState`/`EngineContext` + `run_blackboard`/`Board` runtime/`BoardState`/`HaltReason` + `run_ring`/`RingToken` runtime/`RingOutcome`/`TokenStatus` + `LlmBoardAgent`/`LlmRingAgent` + les `record_orchestration_*`/`NestedRun`-driven morts.
- [ ] **CONSERVER** : `BlackboardConfig`, `RingConfig`, `EntryKind`, `entry_kind_name`, `ContributionAction`, `parse_board_action`/`parse_ring_action`/`parse_vote_confidence`, `BOARD_ACTION_INSTRUCTIONS`/`RING_ACTION_INSTRUCTIONS`, `context_injection`, `protocol`, `agent_selection`, `classifier` (consommés par ES).
- [ ] `cargo build --all-features` + `cargo test` + clippy 3 modes + fmt verts.
- [ ] Commit `refactor(orchestration): remove legacy mutable engines (superseded by event-sourced)` — PR séparée.

---

## Notes pour l'implémenteur
- Le socle ES + les 3 moteurs (Lots 1-3) sont mergés et NE changent pas (sauf T3 réconciliations dans `es/hierarchical.rs`/`es/blackboard.rs`/`es/ring.rs`).
- Vérifie les vrais chemins/formes : `crate::core::events::{RunEvent, EventSink}` (OH3), `OrchestrationResult`/`DelegationEvent`, les fn `insert_*` de `queries.rs`, le keying agent_map (`run.rs:1011`).
- Séquentiel obligatoire. Ne PAS supprimer le legacy avant le checkpoint + PR séparée.
- Observabilité : le `SinkProjectingLog` DOIT émettre les mêmes `RunEvent` que le legacy pour ne pas casser le JSONL headless — c'est le point le plus sensible, teste-le explicitement.
- Si une signature diffère du réel, adapte en gardant l'intention + les assertions ; note l'écart.
