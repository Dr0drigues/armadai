# OH1 — Lot 2 (hierarchical event-sourcé) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Réécrire le pattern `hierarchical` en event-sourcing pur — un `HierarchicalDecider` (décision pure) + un `HierarchicalEffectRunner` (appel LLM) branchés sur la boucle `run_event_sourced` du socle — **en coexistence** avec le moteur `HierarchicalEngine` existant (aucune bascule du chemin `run`, c'est le Lot 4).

**Architecture:** Nouveau sous-module `src/core/orchestration/es/hierarchical.rs`. La décision (quel agent invoquer, parsing des délégations, gardes depth/budget, routing `latest:auto`) est PURE et vit dans le `Decider` ; le seul effet (`provider.complete`) vit dans l'`EffectRunner`. La boucle est séquentielle → déterministe → rejouable. Réutilise tel quel `protocol.rs` (parsing pur) et `routing.rs` (routing pur).

**Tech Stack:** Rust edition 2024, async-trait, le socle ES (Lot 1, mergé).

## Global Constraints

- Base = `origin/release/1.0.0` (@ 950c64e, socle ES mergé). Branche `feat/oh1-lot2-hierarchical`, PR vers `release/1.0.0`.
- **Coexistence stricte** : ne PAS modifier `hierarchical.rs` (moteur existant), ni `cli/run.rs`, ni la persistance. Ajouts uniquement (nouveau `es/hierarchical.rs` + petite extension de `es/state.rs`/`es/event.rs` si un event doit être appliqué). Les ~869 tests existants restent verts.
- `Decider`/`apply` **purs/déterministes/sync** — aucune I/O, aucun `Instant::now`/hasard. Seul `EffectRunner::run_invoke` est async/impur.
- Exécution **séquentielle** (pas de `tokio::spawn`) → replay fidèle. Le parallélisme éventuel est une décision du Lot 4.
- Clippy `-D warnings` en `--features tui`, `--features tui,providers-api` (les mocks provider ont besoin de la couche providers) + `cargo fmt`.
- Spec : `docs/superpowers/specs/2026-07-21-oh1-event-sourcing-design.md` §5. Carte de portage validée (arbitrages §10 tranchés — voir Décisions ci-dessous).

### Décisions d'arbitrage (tranchées, à respecter)
- **Routing `latest:auto`** : résolu DANS le decider (pur) via `route()` + `resolve_model_for_tier()` ; émis en `Action::Emit(ExecutionEvent::ModelRouted{agent,tier,reason})` AVANT l'`Action::Invoke`. Le `Decider` porte donc les infos de topologie/routing (config, agents, routing_rules) en champs immuables.
- **Trace** : étendre `apply` pour que `AskedPeer` et `Escalated` poussent aussi dans `HierState.trace` (comme `Delegated`), afin de préserver l'équivalence avec `OrchestrationResult.trace`.
- **Budget/cost partiel** : dépassement → `Action::Emit(Warned{code})` puis `Action::Complete{content: <texte partiel reconstruit>}` (contenu non vide). PAS de `Halted` (run.rs attend un content).
- **Anti-boucle synthèse** : le decider compte les tours par agent (occurrences dans `conversations`) ; au-delà de 2 tours pour l'agent racine sans `FinalAnswer`, il force `Complete` avec la dernière narrative.
- **Nested C9** : hors périmètre Lot 2 sauf la frontière — quand l'agent à invoquer est lead d'une team à `pattern`, émettre `NestedStarted{team_lead,pattern}` et déléguer au `EffectRunner` (qui, au Lot 3, lancera un sous-run ES). Au Lot 2 : test que le decider émet bien `NestedStarted`, sans exécuter le sous-run.
- **Compteurs** (`iteration_count`/`invocation_count`) : dérivés par le decider en comptant les tours dans l'état, pas de champ ES ajouté.

---

### Task 1: Transducteur pur `réponse → intentions planifiées`

**Files:** Create `src/core/orchestration/es/hierarchical.rs`; modify `es/mod.rs` (`pub mod hierarchical;`).

**Interfaces:**
- Consumes: `crate::core::orchestration::protocol::{parse_delegations, DelegationAction, classify_relationship}` (parsing pur existant), `es::event::ExecutionEvent`, `es::engine::Action`.
- Produces:
  - `enum PlannedStep { Invoke { agent: String, task: String, event: ExecutionEvent }, Complete { content: String } }` (interne au module).
  - `fn plan_from_response(response: &str, sender: &str, config: &OrchestrationConfig, depth: u32) -> Vec<PlannedStep>` — pur. Utilise `parse_delegations(response, sender, config)` (le config complet est requis pour classer Peer/Superior/Subordinate via la topologie `teams`) ; mappe `Delegate{target,task}`→`Invoke`+`ExecutionEvent::Delegated{from:sender,to:target,task,depth:depth+1}`, `AskPeer`→`Invoke`+`AskedPeer`, `Escalate`→`Invoke`+`Escalated`, `FinalAnswer{content}`→`Complete{content}`.

- [ ] **Step 1: Write the failing tests** (`es/hierarchical.rs` `#[cfg(test)]`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_answer_becomes_complete() {
        let steps = plan_from_response("Voici la réponse finale.", "dev-lead", "dev-lead", 0);
        assert_eq!(steps.len(), 1);
        matches!(&steps[0], PlannedStep::Complete { content } if content.contains("finale"))
            .then_some(()).expect("expected Complete");
    }

    #[test]
    fn delegation_lines_become_invokes_with_events() {
        let resp = "@core-specialist: implémente X\n@qa-specialist: teste X";
        let steps = plan_from_response(resp, "dev-lead", "dev-lead", 0);
        assert_eq!(steps.len(), 2);
        // ordre = ordre des lignes (déterminisme)
        match &steps[0] {
            PlannedStep::Invoke { agent, event, .. } => {
                assert_eq!(agent, "core-specialist");
                assert!(matches!(event, ExecutionEvent::Delegated { to, depth, .. } if to == "core-specialist" && *depth == 1));
            }
            _ => panic!("expected Invoke"),
        }
        match &steps[1] {
            PlannedStep::Invoke { agent, .. } => assert_eq!(agent, "qa-specialist"),
            _ => panic!("expected Invoke"),
        }
    }
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::hierarchical` → FAIL (module/fn absents).

- [ ] **Step 3: Implement** `plan_from_response` + `PlannedStep` in `es/hierarchical.rs`, reusing `parse_delegations`. Add `pub mod hierarchical;` to `es/mod.rs` (garde le style `#[allow(unused_imports)]` sur d'éventuels réexports non encore consommés). Inspecte la vraie forme de `DelegationAction` dans `protocol.rs` et adapte le mapping.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean.

- [ ] **Step 5: Commit** `git commit -m "feat(es): hierarchical response→intentions pure transducer"`

---

### Task 2: Extension `apply` — trace des AskedPeer/Escalated

**Files:** Modify `src/core/orchestration/es/state.rs` (bras `apply` pour `AskedPeer`/`Escalated`), + tests.

**Interfaces:**
- Consumes: `ExecutionEvent::{AskedPeer, Escalated}`, `HierState.trace: Vec<(String,String,String,u32)>`.
- Produces: `apply` pousse dans `hier.trace` un tuple `(from, to, message/question, depth)` pour `AskedPeer` et `Escalated` (comme déjà fait pour `Delegated`). Aucune signature ne change.

- [ ] **Step 1: Write the failing test** (`es/state.rs` tests)

```rust
#[test]
fn asked_peer_and_escalated_are_traced() {
    let events = vec![
        ExecutionEvent::RunStarted { run_id: "r".into(), pattern: "hierarchical".into(),
            agents: vec!["a".into(), "b".into()], input: "x".into(), project: None },
        ExecutionEvent::AskedPeer { from: "a".into(), to: "b".into(), question: "q?".into() },
        ExecutionEvent::Escalated { from: "b".into(), to: "a".into(), message: "up".into() },
    ];
    let st = fold(&events);
    assert_eq!(st.hier.trace.len(), 2);
    assert_eq!(st.hier.trace[0].1, "b"); // to
    assert_eq!(st.hier.trace[1].2, "up"); // message
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::state::tests::asked_peer` → FAIL (actuellement no-op).

- [ ] **Step 3: Implement** — dans `apply`, remplacer les bras no-op `AskedPeer`/`Escalated` par un push dans `state.hier.trace` (depth = profondeur courante ; si non portée par l'event, utiliser la longueur de trace ou 0 — inspecte le champ `depth` réel ; `AskedPeer`/`Escalated` n'ont pas de `depth` dans l'enum → pousser `0` ou dériver ; documente le choix). Ne casse aucun test existant de `state.rs`.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean (relance TOUTE la suite `es::state` pour non-régression).

- [ ] **Step 5: Commit** `git commit -m "feat(es): trace AskedPeer/Escalated in HierState"`

---

### Task 3: `HierarchicalDecider` (décision pure)

**Files:** Modify `src/core/orchestration/es/hierarchical.rs`; tests.

**Interfaces:**
- Consumes: `es::engine::{Action, Decider}`, `ExecutionState`, `plan_from_response` (Task 1), `crate::core::orchestration::classifier`/config types (`OrchestrationConfig`, `TeamConfig`), `crate::core::routing::{route, resolve_model_for_tier}` (routing pur), `crate::core::agent::Agent`.
- Produces:
  - `struct HierarchicalDecider { coordinator: String, input: String, config: OrchestrationConfig, agents: BTreeMap<String, Agent>, routing_rules: RoutingRules, max_depth: u32, max_iterations: u32, token_budget: Option<u32>, cost_limit: Option<f64> }` (tous immuables). Constructeur `new(...)`.
  - `impl Decider for HierarchicalDecider { fn decide(&self, state: &ExecutionState) -> Vec<Action> }`.
- Logique `decide` (pure) :
  1. Si `state` vide (aucun tour) → `Emit(ModelRouted{coordinator,tier,reason})` (si `latest:auto`) puis `Invoke{coordinator, input}`.
  2. Gardes : profondeur courante (max de `hier.trace.depth`) ≥ `max_depth`, ou nb d'`Invoke` ≥ `max_iterations`, ou budget/cost dépassé → `Emit(Warned{code})` + `Complete{content: reconstruire le partiel}` (helper `build_partial_content(state)` pur, itère `state.conversations` — BTreeMap → ordre stable).
  3. Sinon, prendre la **dernière réponse assistant non traitée** (le dernier `AgentObserved` dont les délégations n'ont pas encore été émises — dérivable en comparant le nombre de réponses au nombre d'invocations enfant) → `plan_from_response` → pour chaque `Invoke`, préfixer d'un `ModelRouted` si l'agent cible route en `latest:auto` ; un `Complete` termine.
  4. Anti-boucle : si le coordinator a déjà eu ≥ 2 tours sans `FinalAnswer`, `Complete` avec la dernière narrative.
  5. Frontière nested : si l'agent à invoquer est lead d'une team à `pattern` → `Emit(NestedStarted{team_lead,pattern})` avant l'`Invoke` (le sous-run réel = Lot 3, exécuté côté effect).

- [ ] **Step 1: Write the failing tests** — construire des `ExecutionState` via `fold(events)` et asserter `decide(&state)`. Cas : (a) état vide → premier `Invoke(coordinator)` ; (b) après une réponse `FinalAnswer` du coordinator → `Complete` ; (c) après une réponse avec 2 délégations → 2 `Invoke` (+ `Delegated`) dans l'ordre ; (d) profondeur ≥ max_depth → `Warned`+`Complete` ; (e) agent lead à pattern → `NestedStarted` émis. Utilise des `Agent`/`OrchestrationConfig` minimaux (helpers de test).

```rust
#[test]
fn empty_state_invokes_coordinator_first() {
    let dec = test_decider(/* coordinator="dev-lead", agents, config */);
    let state = fold(&[ExecutionEvent::RunStarted {
        run_id: "r".into(), pattern: "hierarchical".into(),
        agents: vec!["dev-lead".into()], input: "build X".into(), project: None }]);
    let actions = dec.decide(&state);
    assert!(actions.iter().any(|a| matches!(a, Action::Invoke { agent, .. } if agent == "dev-lead")));
}
```
(Écris aussi les cas b–e ; le brief attend des assertions concrètes pour chacun.)

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::hierarchical::tests::decide` → FAIL.

- [ ] **Step 3: Implement** `HierarchicalDecider` + helpers purs (`current_depth`, `invocation_count`, `build_partial_content`, `pending_response`, `resolve_tier_for`). Vérifie les vraies signatures de `route`/`resolve_model_for_tier`/`OrchestrationConfig`/`RoutingRules` dans le code et adapte. AUCUN async, AUCUNE I/O.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean.

- [ ] **Step 5: Commit** `git commit -m "feat(es): HierarchicalDecider — pure hierarchical decision fn"`

---

### Task 4: `HierarchicalEffectRunner` (appel LLM)

**Files:** Modify `src/core/orchestration/es/hierarchical.rs`; tests.

**Interfaces:**
- Consumes: `es::engine::EffectRunner`, `ExecutionState`, `crate::providers::traits::{Provider, CompletionRequest, ChatMessage}`, `crate::core::orchestration::context_injection::build_enriched_prompt` (ou équivalent), `crate::core::agent::Agent`.
- Produces:
  - `struct HierarchicalEffectRunner { agents: BTreeMap<String, Agent>, providers: BTreeMap<String, Arc<dyn Provider>>, config: OrchestrationConfig }`.
  - `#[async_trait] impl EffectRunner`: `run_invoke(&self, agent, input, state)` — reconstruit `agents_info` depuis `agents` (description = 1re ligne non vide du system_prompt), construit le prompt enrichi + la conversation depuis `state.conversations[agent]`, choisit le modèle déjà résolu (le `ModelRouted` émis par le decider fixe le tier ; l'effect lit le dernier `ModelRouted` pour cet agent dans l'état OU re-résout de façon identique — documente), appelle `provider.complete`, renvoie `ExecutionEvent::AgentObserved{agent, content, tokens_in, tokens_out, cost, model}`.

- [ ] **Step 1: Write the failing test** — réutilise un `MockProvider`/`CapturingProvider` (voir `hierarchical.rs` tests existants pour le patron) renvoyant une réponse fixe + des métriques. Vérifie que `run_invoke` renvoie un `AgentObserved` avec le bon `agent`, le contenu du mock, et les tokens/cost/model attendus.

```rust
#[tokio::test]
async fn effect_runner_invokes_provider_and_returns_observed() {
    let runner = test_runner_with_mock(/* agent "a" → provider renvoyant "resp", tokens 3/4 */);
    let state = fold(&[ExecutionEvent::RunStarted {
        run_id: "r".into(), pattern: "hierarchical".into(),
        agents: vec!["a".into()], input: "go".into(), project: None }]);
    let ev = runner.run_invoke("a", "go", &state).await.unwrap();
    match ev {
        ExecutionEvent::AgentObserved { agent, content, tokens_in, tokens_out, .. } => {
            assert_eq!(agent, "a"); assert_eq!(content, "resp");
            assert_eq!(tokens_in, 3); assert_eq!(tokens_out, 4);
        }
        _ => panic!("expected AgentObserved"),
    }
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui,providers-api -p armadai es::hierarchical::tests::effect` → FAIL.

- [ ] **Step 3: Implement** `HierarchicalEffectRunner`. Inspecte les vraies signatures de `Provider::complete`/`CompletionRequest`/`CompletionResponse` et de la construction de prompt (`context_injection`). Le mock provider peut être un helper de test local si aucun réutilisable n'est accessible depuis ce module.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean.

- [ ] **Step 5: Commit** `git commit -m "feat(es): HierarchicalEffectRunner — LLM effect for hierarchical"`

---

### Task 5: Câblage bout-en-bout `run_hierarchical_es` + replay

**Files:** Modify `src/core/orchestration/es/hierarchical.rs`; tests.

**Interfaces:**
- Consumes: `es::engine::{run_event_sourced, replay}`, `es::log::InMemoryLog`, `HierarchicalDecider` (T3), `HierarchicalEffectRunner` (T4).
- Produces:
  - `pub async fn run_hierarchical_es(coordinator: &str, input: &str, config: OrchestrationConfig, agents: BTreeMap<String, Agent>, providers: BTreeMap<String, Arc<dyn Provider>>, routing_rules: RoutingRules, log: &mut impl EventLog) -> anyhow::Result<ExecutionState>` — assemble le `RunStarted` initial, construit decider+effect, appelle `run_event_sourced`. **Non branché dans `run.rs`** (coexistence).

- [ ] **Step 1: Write the failing tests** — scénarios équivalents aux tests du moteur existant, via mocks :
  - `es_single_delegation` : coordinator délègue à un agent qui répond `FinalAnswer` ; état final `Completed`, trace contient la délégation, content = réponse finale.
  - `es_multiple_delegations` : 2 délégations séquentielles, synthèse, `Completed`.
  - `es_max_depth_halts_gracefully` : chaîne de délégations dépassant `max_depth` → `Completed` avec `Warned` budget/depth, content non vide.
  - `es_replay_reconstructs_state` : après un run, `replay(run_id, &log)` == état retourné (déterminisme prouvé).

```rust
#[tokio::test]
async fn es_single_delegation_completes() {
    let mut log = InMemoryLog::default();
    let (config, agents, providers, rules) = fixture_single_delegation();
    let st = run_hierarchical_es("dev-lead", "build X", config, agents, providers, rules, &mut log).await.unwrap();
    assert_eq!(st.status, RunStatus::Completed);
    assert!(!st.hier.trace.is_empty());
    let replayed = replay("...run_id...", &log).unwrap();
    assert_eq!(format!("{st:?}"), format!("{replayed:?}"));
}
```
(Le `run_id` : `run_hierarchical_es` doit le générer/l'accepter de façon déterministe pour le test — passe-le en paramètre ou expose-le dans le retour ; adapte l'assertion.)

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui,providers-api -p armadai es::hierarchical::tests::es_` → FAIL.

- [ ] **Step 3: Implement** `run_hierarchical_es` + fixtures de test (mocks provider scriptés par agent). Assure un `run_id` déterministe pour les tests (paramètre).

- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean. Relance la suite complète `--features tui,providers-api` pour non-régression (~863 tests).

- [ ] **Step 5: Commit** `git commit -m "feat(es): run_hierarchical_es end-to-end + replay determinism test"`

---

### Task 6: Frontière nested (émission `NestedStarted`, sans exécution)

**Files:** Modify `src/core/orchestration/es/hierarchical.rs`; tests.

**Interfaces:**
- Consumes: `HierarchicalDecider` (T3), `ExecutionState`, `TeamConfig` (topologie des teams dans `OrchestrationConfig`).
- Produces: confirmation testée que `decide` émet `Action::Emit(ExecutionEvent::NestedStarted{team_lead,pattern})` quand l'agent à invoquer est lead d'une team à `pattern` — SANS lancer de sous-run (le sous-run réel = Lot 3). Documente en commentaire le hook `EffectRunner` prévu pour le Lot 3 (option A : sous-run ES séparé avec `parent_run_id`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn decider_emits_nested_started_for_team_lead_with_pattern() {
    // config: agent "team-lead" est lead d'une team pattern="blackboard"
    let dec = test_decider_with_nested_team("team-lead", "blackboard");
    // état où le coordinator a délégué à "team-lead"
    let state = fold(&nested_delegation_events());
    let actions = dec.decide(&state);
    assert!(actions.iter().any(|a| matches!(a,
        Action::Emit(ExecutionEvent::NestedStarted { team_lead, pattern })
            if team_lead == "team-lead" && pattern == "blackboard")));
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai es::hierarchical::tests::decider_emits_nested` → FAIL.

- [ ] **Step 3: Implement** — brancher la détection lead-à-pattern dans `decide` (émission `NestedStarted` avant l'`Invoke` du lead). Vérifie comment `OrchestrationConfig` porte les teams/pattern (`TeamConfig` — voir C9). AUCUNE exécution de sous-run.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt** → PASS/clean.

- [ ] **Step 5: Commit** `git commit -m "feat(es): hierarchical decider emits NestedStarted boundary (C9 Lot 3 hook)"`

---

## Notes pour l'implémenteur
- **Coexistence** : `hierarchical.rs` (moteur existant) reste intact et branché dans `run.rs`. Tu construis un chemin ES PARALLÈLE dans `es/hierarchical.rs`. Rien ne bascule en prod dans ce lot.
- Réutilise au MAXIMUM le pur existant : `protocol::parse_delegations`, `routing::route`/`resolve_model_for_tier`, `context_injection` pour le prompt. Ne les duplique pas.
- Séquentiel obligatoire (pas de `tokio::spawn`) — le parallélisme est une décision Lot 4.
- Déterminisme : `conversations`/`build_partial_content` doivent itérer des structures ordonnées (BTreeMap déjà en place). Aucun temps/hasard dans `decide`.
- Si une signature de la carte de portage ne correspond pas au code réel (formes de `DelegationAction`, `OrchestrationConfig`, `Provider::complete`, `RoutingRules`), adapte au réel en conservant l'intention et les assertions — et note l'écart dans le rapport de task.
- Les ~869 tests existants + les tests ES du Lot 1 doivent rester verts à chaque task.
