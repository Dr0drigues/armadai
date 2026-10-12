# OH1 — Lot 3 (blackboard + ring event-sourcés + nested C9) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Réécrire les patterns `blackboard` et `ring` en event-sourcing pur (Decider pur + EffectRunner async + `run_xxx_es` + preuve de replay), **en coexistence** avec les moteurs legacy, puis câbler le sous-run nested C9 (hierarchical → blackboard/ring ES) laissé en frontière au Lot 2.

**Architecture:** Nouveaux sous-modules `src/core/orchestration/es/blackboard.rs` et `es/ring.rs`, calqués sur le gabarit **déjà mergé `es/hierarchical.rs`** (Lot 2). Décision pure/déterministe reconstruite depuis `ExecutionState` ; seul l'appel LLM est async (EffectRunner). Le nested se branche dans `HierarchicalEffectRunner::run_invoke`.

**Tech Stack:** Rust edition 2024, async-trait, socle ES (Lots 1-2).

## Global Constraints

- Base = `origin/release/1.0.0` (@ 0c2022a, Lots 1-2 mergés). Branche `feat/oh1-lot3-blackboard-ring`, PR vers `release/1.0.0`.
- **Coexistence stricte** : ne PAS modifier `blackboard.rs`/`ring.rs`/`hierarchical.rs` (moteurs legacy) SAUF le sous-module `es/` et le socle `es/state.rs`/`es/engine.rs` si explicitement prévu. NE PAS toucher `cli/run.rs`. Les ~896 tests existants restent verts.
- **Gabarit** : imiter la structure de `es/hierarchical.rs` (Decider pur, EffectRunner async, `run_xxx_es`, mocks provider `#[cfg(test)]`, réutilisation `route`/`resolve_model_for_tier`/`context_injection`).
- `Decider`/`apply` **purs/déterministes/sync** (aucune I/O, temps, hasard ; itération BTreeMap/Vec). Exécution **séquentielle** (pas de `tokio::spawn`).
- Réutiliser les parsers purs existants de `llm_agents.rs` : `parse_board_action`, `parse_ring_action`, `parse_vote_confidence`. NE PAS dupliquer.
- Clippy `-D warnings` en `--features tui` ET `--features tui,providers-api` + `cargo fmt`.
- Spec `docs/superpowers/specs/2026-07-21-oh1-event-sourcing-design.md` §5. Carte de portage validée (arbitrages §10 tranchés ci-dessous).

### Décisions d'arbitrage (tranchées)
- **`routed_tiers` promu** de `HierState` vers `ExecutionState` (niveau run) — Task 1.
- **blackboard** : snapshot début-de-round = le prompt ne voit que les entries `round < round_courant` ; `consecutive_convergence` reconstruit purement ; `kind: EntryKind` → `kind:String` + cibles encodées dans `BoardEntryRec.refs`.
- **ring** : phase (circulation/vote/résolution) dérivée par `ring_phase(state,config)` pure, partagée decider+effect ; `resolve_votes` + tie-break (`max_by` = dernier max, ordre BTreeMap) reproduits à l'identique ; `ContribRec` sans `refs` → cibles Enrich/Contest/Endorse **non préservées** (affichage-only, acceptable).
- **échec d'agent** : `run_invoke` capture l'erreur provider et renvoie un event dégradé (contribution `Pass` / entry vide) + n'avorte jamais le run.
- **nested** : `HierarchicalEffectRunner::run_invoke` court-circuite un lead à `pattern` en lançant `run_blackboard_es`/`run_ring_es` sur un log/run_id enfant, remonte l'outcome+métriques en `AgentObserved` ; `NestedEnded` émis en différé par le `HierarchicalDecider`.
- Chemins morts ignorés (ProposeHalt/Annotate/reactions/priority).

---

### Task 1: Promouvoir `routed_tiers` de `HierState` vers `ExecutionState`

**Files:** Modify `src/core/orchestration/es/state.rs`, `src/core/orchestration/es/hierarchical.rs` (lecteur dans l'EffectRunner).

**Interfaces:**
- Déplace `routed_tiers: BTreeMap<String,String>` de `HierState` vers `ExecutionState` (niveau run). Le bras `apply` de `ModelRouted` écrit désormais `state.routed_tiers.insert(...)`. `HierarchicalEffectRunner` lit `state.routed_tiers.get(agent)` (au lieu de `state.hier.routed_tiers`).

- [ ] **Step 1: Update the failing test** — adapte le test `es::state` existant qui vérifie la projection `ModelRouted` pour lire `state.routed_tiers` (au lieu de `state.hier.routed_tiers`). Lance-le → FAIL (champ déplacé pas encore).
- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::state` → FAIL de compilation/assertion.
- [ ] **Step 3: Implement** — bouge le champ, mets à jour `apply` (bras `ModelRouted`) + le lecteur dans `es/hierarchical.rs` (`run_invoke`, résolution `latest:auto`). Vérifie qu'aucun autre lecteur n'existe (`grep routed_tiers`).
- [ ] **Step 4: Run** — `cargo test --no-default-features --features tui,providers-api -p armadai es::` → tous verts (dont les tests hierarchical `latest:auto`). Clippy 2 modes + fmt.
- [ ] **Step 5: Commit** `git commit -m "refactor(es): promote routed_tiers to run-level ExecutionState"`

---

### Task 2: Helpers purs blackboard

**Files:** Create `src/core/orchestration/es/blackboard.rs`; modify `es/mod.rs` (`pub mod blackboard;`).

**Interfaces:**
- Consumes: `es::state::{ExecutionState, BoardState, BoardEntryRec}`, `crate::core::agent::Agent`, la config blackboard (`max_rounds`, `consensus_threshold`, `convergence_rounds` — vérifie les champs réels dans le legacy/config).
- Produces (privés, purs) :
  - `fn eligible_agents(state, agents, config) -> Vec<String>` — reproduit `can_contribute` (bornes de round + kinds requis/exclus présents sur le board), ordonné par le roster (`state.agents`).
  - `fn check_convergence(state, config) -> Option<f32>` — ratio confirmations/entrées du dernier round vs `consensus_threshold` (réplique pure de la logique legacy, SANS le side-effect `tracing`).
  - `fn consecutive_convergence(state, config) -> u32` — compte les rounds finaux consécutifs convergents (reconstruit depuis `board.entries` groupées par round).
  - `fn entry_kind_to_string(kind) -> (String, Vec<usize>)` / mapping cibles → refs (documente la convention : `challenge`+refs=[target], `synthesis`+refs=sources, `confirmation`+refs=[target], `answer`, `finding`).

- [ ] **Step 1: Write the failing tests** — rejoue les cas de convergence du legacy (`blackboard.rs` tests) : board à N entries → `check_convergence`/`consecutive_convergence` attendus ; `eligible_agents` sur un board donné → sous-ensemble ordonné attendu ; mapping kind↔refs. (Assertions concrètes, pas de façade.)
- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::blackboard` → FAIL.
- [ ] **Step 3: Implement** — helpers purs, en réutilisant les seuils/logique du legacy `blackboard.rs` (inspecte `should_halt`/`check_convergence`/`can_contribute`). Ajoute `pub mod blackboard;` à `es/mod.rs`.
- [ ] **Step 4: Run** — tests verts, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): blackboard pure helpers (eligibility, convergence, kind mapping)"`

---

### Task 3: `BlackboardDecider`

**Files:** Modify `src/core/orchestration/es/blackboard.rs`.

**Interfaces:**
- Produces: `struct BlackboardDecider { agents: BTreeMap<String,Agent>, agent_order: Vec<String>, input: String, config: <blackboard config>, routing_rules: RoutingRules, max_rounds: u32, token_budget: Option<u32>, cost_limit: Option<f64> }` + `impl Decider`.
- `decide(state)` (pur) :
  1. État vide → `Emit(RoundStarted{round:0})` + batch d'`Invoke` des `eligible_agents` (préfixés d'un `ModelRouted` si `latest:auto`).
  2. Round courant complet (tous les éligibles du round ont produit une entry) → évaluer `should_halt` : `round+1 >= max_rounds` → `Warned{max_rounds}`+`Complete{synthèse}` ; convergence atteinte `convergence_rounds` fois → `Emit(ConsensusReached{score})`+`Complete` ; budget/cost dépassé → `Warned`+`Complete` ; sinon `Emit(RoundStarted{round+1})` + `Invoke` des éligibles du nouveau round.
  3. `Complete{content}` = synthèse déterministe du board (concat ordonnée des entries pertinentes — helper pur `build_board_result`).
- Le prompt (construit côté effect) devra filtrer `round < round_courant` (snapshot) — noté ici pour cohérence, implémenté Task 4.

- [ ] **Step 1: Write the failing tests** — `decide` sur `ExecutionState` construits via `fold` : (a) état vide → `RoundStarted{0}` + Invokes éligibles ; (b) round complet non convergent < max_rounds → `RoundStarted{1}` + Invokes ; (c) convergence atteinte → `ConsensusReached` + `Complete` ; (d) max_rounds → `Warned{max_rounds}`+`Complete`. Assertions concrètes.
- [ ] **Step 2: Run to verify fail** — `es::blackboard::tests::decide` → FAIL.
- [ ] **Step 3: Implement** `BlackboardDecider` + `build_board_result` pur + réutilise `route` pour `ModelRouted`.
- [ ] **Step 4: Run** — verts, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): BlackboardDecider — pure blackboard decision fn"`

---

### Task 4: `BlackboardEffectRunner`

**Files:** Modify `src/core/orchestration/es/blackboard.rs`.

**Interfaces:**
- Produces: `struct BlackboardEffectRunner { agents, providers, config }` + `#[async_trait] impl EffectRunner`.
- `run_invoke(agent, input, state)` : construit le prompt (system de l'agent + `BOARD_ACTION_INSTRUCTIONS` + **snapshot filtré aux entries `round < state.board.round`**), résout `latest:auto`→modèle concret (via `state.routed_tiers`), appelle `provider.complete`, parse via `parse_board_action` → `BoardEntryAdded{agent,round,kind,content,refs,confidence,tokens_in,tokens_out,cost}`. **Sur erreur provider : renvoie une entry dégradée** (kind="finding", content vide/marqueur, confidence 0, tokens 0) sans avorter — documente.

- [ ] **Step 1: Write the failing test** — mock provider renvoyant une réponse `ACTION:/CONFIDENCE:/CONTENT:` → `run_invoke` renvoie `BoardEntryAdded` avec kind/confidence/content/round attendus + tokens réels. + un test : erreur provider → entry dégradée, pas d'`Err`.
- [ ] **Step 2: Run to verify fail** — `es::blackboard::tests::effect` → FAIL.
- [ ] **Step 3: Implement** — réutilise `parse_board_action` (`llm_agents.rs`) + construction prompt (inspecte `contribute`). Filtre snapshot. Résolution modèle calquée sur `es/hierarchical.rs`.
- [ ] **Step 4: Run** — verts (features tui,providers-api), clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): BlackboardEffectRunner — LLM effect + round snapshot + graceful failure"`

---

### Task 5: `run_blackboard_es` + replay

**Files:** Modify `src/core/orchestration/es/blackboard.rs`.

**Interfaces:**
- Produces: `pub async fn run_blackboard_es(run_id, input, agents: BTreeMap<String,Agent>, providers, config, routing_rules, log: &mut impl EventLog) -> anyhow::Result<ExecutionState>` — monte `RunStarted{pattern:"blackboard",...}` + Decider + Effect, appelle `run_event_sourced`. Non branché dans `run.rs`.

- [ ] **Step 1: Write the failing tests** (mocks scriptés par agent, modèles concrets — pas de `latest:auto` pour rester hermétique) :
  - `es_blackboard_converges_and_completes` : agents produisent des confirmations → convergence → `Completed`, content non vide.
  - `es_blackboard_halts_at_max_rounds` : jamais de convergence → `Warned{max_rounds}` + `Completed`.
  - `es_blackboard_replay_reconstructs_state` : `replay(run_id,&log)` == état final (égalité `Debug`) + baseline appels provider `> 0` avant replay et inchangés après.
- [ ] **Step 2: Run to verify fail** — `es::blackboard::tests::es_` → FAIL.
- [ ] **Step 3: Implement** `run_blackboard_es` + fixtures.
- [ ] **Step 4: Run** — verts + suite complète non régressée, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): run_blackboard_es end-to-end + replay determinism"`

---

### Task 6: Helpers purs ring (`ring_phase`, `resolve_votes`, tie-break)

**Files:** Create `src/core/orchestration/es/ring.rs`; modify `es/mod.rs` (`pub mod ring;`).

**Interfaces:**
- Produces (purs) :
  - `enum RingPhase { Circulate { lap: u32 }, Vote, Resolve, Done }` + `fn ring_phase(state, config) -> RingPhase` — dérive la phase depuis `state.ring` (nb contributions par lap, lap vs max_laps, tous Pass en fin de lap, votes complets). **Partagé decider + effect.**
  - `fn resolve_votes(state, vote_weights: &BTreeMap<String,f32>) -> String` — réplique EXACTE de `ring.rs::resolve_votes` (groupes par similarité de position, `position_similarity`, tie-break `max_by` = dernier max dans l'ordre BTreeMap, representative = premier vote du plus grand groupe).
  - `fn next_ring_agent(state, agent_order) -> Option<String>` — prochain agent à parler dans la circulation (position = index dans le lap).

- [ ] **Step 1: Write the failing tests** — `ring_phase` sur états construits (circulation partielle/complète, tous Pass, phase vote, done) ; `resolve_votes` incluant un **cas d'égalité de poids** vérifiant le tie-break exact (dernier max) ; `next_ring_agent`. Rejoue les cas de `ring.rs` tests.
- [ ] **Step 2: Run to verify fail** — `es::ring` → FAIL.
- [ ] **Step 3: Implement** — réplique fidèle de `resolve_votes`/`position_similarity` (inspecte `ring.rs:362-468`). Ajoute `pub mod ring;`.
- [ ] **Step 4: Run** — verts, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): ring pure helpers (ring_phase, resolve_votes tie-break, rotation)"`

---

### Task 7: `RingDecider` (3 phases)

**Files:** Modify `src/core/orchestration/es/ring.rs`.

**Interfaces:**
- Produces: `struct RingDecider { agents, agent_order: Vec<String>, input, config, routing_rules, max_laps, vote_weights: BTreeMap<String,f32>, token_budget, cost_limit }` + `impl Decider`.
- `decide(state)` via `ring_phase` :
  - `Circulate` → `Invoke{next_ring_agent}` (+ `ModelRouted` si `latest:auto`) ; début de lap → `Emit(LapStarted{lap})`.
  - transition vers `Vote` (lap >= max_laps ou tous Pass) → `Invoke` du 1er votant.
  - `Vote` en cours → `Invoke` du votant suivant.
  - `Resolve` (tous ont voté) → `Emit(OutcomeResolved{outcome: resolve_votes(...)})` + `Complete{content:outcome}`.
  - gardes budget/cost/max_laps → `Warned`+`Complete`.

- [ ] **Step 1: Write the failing tests** — `decide` sur états : début → `LapStarted{0}`+Invoke 1er agent ; circulation → Invoke agent suivant ; fin circulation → Invoke 1er votant ; tous votés → `OutcomeResolved`+`Complete`. Assertions concrètes.
- [ ] **Step 2: Run to verify fail** — `es::ring::tests::decide` → FAIL.
- [ ] **Step 3: Implement** `RingDecider` réutilisant `ring_phase`/`resolve_votes`/`next_ring_agent`.
- [ ] **Step 4: Run** — verts, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): RingDecider — pure 3-phase ring decision fn"`

---

### Task 8: `RingEffectRunner`

**Files:** Modify `src/core/orchestration/es/ring.rs`.

**Interfaces:**
- Produces: `struct RingEffectRunner { agents, providers, config, vote_weights }` + `#[async_trait] impl EffectRunner`.
- `run_invoke(agent, input, state)` : utilise `ring_phase(state,config)` pour choisir :
  - phase `Circulate` → prompt process + `RING_ACTION_INSTRUCTIONS`, `parse_ring_action` → `ContributionAdded{agent,lap,position,action,content,tokens...}`.
  - phase `Vote` → prompt vote, `parse_vote_confidence` → `VoteCast{agent,position,confidence,supports:(0..n),concerns:[]}`.
  - résolution `latest:auto`→modèle concret via `state.routed_tiers`.
  - **erreur provider** : phase Circulate → contribution `Pass` (action="pass", content vide) ; phase Vote → vote neutre/abstention documenté. Pas d'avortement.

- [ ] **Step 1: Write the failing tests** — mock provider : en phase circulation → `ContributionAdded` attendu ; en phase vote (état où circulation finie) → `VoteCast` avec confidence/position ; erreur → Pass. + assertion tokens réels.
- [ ] **Step 2: Run to verify fail** — `es::ring::tests::effect` → FAIL.
- [ ] **Step 3: Implement** — réutilise `parse_ring_action`/`parse_vote_confidence` + prompts (inspecte `process`/`vote` dans `llm_agents.rs`).
- [ ] **Step 4: Run** — verts (tui,providers-api), clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): RingEffectRunner — phase-aware LLM effect + graceful failure"`

---

### Task 9: `run_ring_es` + replay

**Files:** Modify `src/core/orchestration/es/ring.rs`.

**Interfaces:**
- Produces: `pub async fn run_ring_es(run_id, input, agents, providers, config, routing_rules, log) -> anyhow::Result<ExecutionState>`. Non branché dans `run.rs`.

- [ ] **Step 1: Write the failing tests** (mocks concrets, hermétiques) :
  - `es_ring_circulates_votes_and_resolves` : agents contribuent puis votent → `OutcomeResolved` + `Completed`, content = outcome.
  - `es_ring_halts_at_max_laps`.
  - `es_ring_replay_reconstructs_state` : égalité `Debug` + baseline appels `> 0`.
- [ ] **Step 2: Run to verify fail** — `es::ring::tests::es_` → FAIL.
- [ ] **Step 3: Implement** `run_ring_es` + fixtures.
- [ ] **Step 4: Run** — verts + suite complète, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): run_ring_es end-to-end + replay determinism"`

---

### Task 10: Câblage nested C9 (hierarchical → sous-run blackboard/ring)

**Files:** Modify `src/core/orchestration/es/hierarchical.rs` (le `HierarchicalEffectRunner` + `HierarchicalDecider` pour `NestedEnded`).

**Interfaces:**
- Consumes: `run_blackboard_es`/`run_ring_es` (Tasks 5/9), la détection lead-à-pattern déjà présente (`nested_started_event`).
- Produces:
  - `HierarchicalEffectRunner::run_invoke` : si `agent` est lead d'une team à `pattern`, court-circuite l'appel LLM plat → lance `run_blackboard_es`/`run_ring_es` sur un **run_id enfant** (`format!("{}::nested::{}", state.run_id, agent)`) + un log enfant (nouvel `InMemoryLog` interne OU le même store — documente), avec agents/providers scopés à `team.agents` et budget restant. Extrait l'outcome (texte) + métriques agrégées (`budget_*` de l'état enfant) → renvoie `AgentObserved{agent, content:outcome, tokens_in/out/cost:agrégés, model}`.
  - `HierarchicalDecider` : émet `NestedEnded{team_lead}` en différé (état où un `NestedStarted{team_lead}` existe, le lead a été observé, et pas de `NestedEnded{team_lead}` postérieur). L'arbitrage du lead = tour normal du parent (déjà géré par la synthèse).

- [ ] **Step 1: Write the failing tests** (calqués sur `test_nested_blackboard_runs_and_folds_metrics`/`test_nested_ring_runs_and_folds_metrics` du legacy) — un run hierarchical ES où le coordinateur délègue à un lead de team `pattern="blackboard"` : le sous-run blackboard s'exécute, l'`AgentObserved` du lead porte l'outcome + les métriques agrégées, `NestedStarted` puis `NestedEnded` présents dans le log, budget du parent inclut le sous-run. Idem un test ring. + replay.
- [ ] **Step 2: Run to verify fail** — `es::hierarchical::tests::nested_` → FAIL.
- [ ] **Step 3: Implement** — court-circuit dans `run_invoke` + `NestedEnded` différé dans `decide`. Attention au budget restant transmis au sous-run.
- [ ] **Step 4: Run** — verts + suite complète, clippy 2 modes, fmt.
- [ ] **Step 5: Commit** `git commit -m "feat(es): wire nested C9 sub-runs (hierarchical -> blackboard/ring ES)"`

---

## Notes pour l'implémenteur
- **Gabarit** : `es/hierarchical.rs` (Lot 2, mergé) est ton modèle pour la structure Decider/EffectRunner/`run_xxx_es`, les mocks provider, la résolution `latest:auto`, les fixtures hermétiques. Imite-le.
- **Coexistence** : les moteurs legacy (`blackboard.rs`/`ring.rs`/`hierarchical.rs`) et `run.rs` ne changent pas (sauf Task 1 qui touche `es/state.rs`+`es/hierarchical.rs`, et Task 10 qui touche `es/hierarchical.rs`). Rien ne bascule en prod.
- **Déterminisme** : itère `state.agents` (Vec ordonné) pour l'ordre de parole ; BTreeMap pour votes/conversations. Reproduis exactement le tie-break de `resolve_votes`.
- **Réutilise** les parsers `llm_agents.rs` (`parse_board_action`/`parse_ring_action`/`parse_vote_confidence`) et la construction de prompt — ne réécris pas.
- **Hermeticité tests** : jamais `latest:auto` dans les fixtures e2e (évite `resolve_model_for_tier` → cache disque non hermétique) ; modèles concrets.
- Si une signature de la carte ne colle pas au réel (champs config, `BoardState`/`RingState`, formes d'enums), adapte au réel en gardant l'intention + les assertions — note l'écart.
- Les ~896 tests existants restent verts à chaque task.
