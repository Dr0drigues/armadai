# OH3/OH4 Consolidation (orchestration) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Étendre OH3/OH4 à l'orchestration : événements JSONL fins (delegate/vote/board) et routage `latest:auto` + budget (downgrade) dans les moteurs.

**Architecture:** Nouveaux variants `RunEvent` (events.rs) ; `sink: &Arc<dyn EventSink>` passé aux 3 moteurs (`run_blackboard`/`run_ring`/`run_hierarchical`) et à `agent_model` ; `route()` câblé dans `llm_agents` avec `BudgetState` dérivé du budget restant du board.

**Tech Stack:** Rust edition 2024. Réutilise OH3 (`core/events.rs`, EventSink) et OH4 (`core/routing.rs`, `route`, `RouteReason`).

## Global Constraints
- Base = `origin/release/1.0.0` (@ `77a31b7`, version 1.0.0-rc.1). Branche `feat/oh34-consolidation`, PR vers `release/1.0.0`.
- Clippy 2 modes `--all-targets -D warnings` + `cargo fmt -- --check` + `cargo test --no-default-features --features tui,providers-api`.
- **Budget en orchestration = DOWNGRADE du tier via le routeur uniquement** ; PAS d'`exit 3` ; le halt gracieux (exit 0, résultat partiel) décidé en beta.3 est CONSERVÉ.
- Clés JSONL courtes, tag `t`, cohérentes avec les events OH3 existants (run_start/agent_start/agent_end/warning/result/error/route).
- Ne pas casser la non-régression : sans `--json`, `NullSink` (no-op), comportement inchangé.

---

### Task 1: Nouveaux variants RunEvent (delegate/vote/board)
**Files:** Modify `src/core/events.rs` (+ tests dans le module).
**Interfaces produces:** `RunEvent::Delegate { from: String, to: String }`, `RunEvent::Vote { agent: String, conf: f32 }`, `RunEvent::Board { agent: String, kind: String }` (serde tag `t`, snake_case → `delegate`/`vote`/`board`).

- [ ] Écrire les tests de sérialisation (clés exactes : `{"t":"delegate","from":..,"to":..}`, `{"t":"vote","agent":..,"conf":..}`, `{"t":"board","agent":..,"kind":..}`), les faire échouer.
- [ ] Ajouter les 3 variants à l'`enum RunEvent`.
- [ ] Tests passent ; clippy 2 modes + fmt ; commit `feat(core): add delegate/vote/board RunEvent variants`.

---

### Task 2: Émettre les events fins dans les 3 moteurs
**Files:** Modify `src/cli/run.rs` (`run_orchestrated` : passer `&sink` aux moteurs) et `src/core/orchestration/{blackboard.rs,ring.rs,hierarchical.rs}` (signatures `run_*` + points d'émission).
**Interfaces consumes:** `RunEvent` (Task 1), `EventSink`.

- [ ] Ajouter un paramètre `sink: &std::sync::Arc<dyn crate::core::events::EventSink>` aux fns `run_blackboard`/`run_ring`/`run_hierarchical` (et propager depuis `run_orchestrated`, `run.rs`).
- [ ] Émettre : `Board{agent,kind}` quand un agent poste une entrée (blackboard) ; `Vote{agent,conf}` à chaque vote (ring) ; `Delegate{from,to}` à chaque `AgentDelegateAction`/invocation enfant (hierarchical). Aux points existants où ces actions se produisent (chercher les sites de contribution/vote/délégation).
- [ ] Mode non-`--json` : `NullSink` → aucun effet. Adapter les tests existants des moteurs aux nouvelles signatures.
- [ ] clippy 2 modes + fmt + full test ; commit `feat(orchestration): emit delegate/vote/board JSONL events`.

---

### Task 3: Routage latest:auto + budget dans l'orchestration
**Files:** Modify `src/core/orchestration/llm_agents.rs` (`agent_model` + points d'appel), éventuellement passer `RoutingRules` + budget restant.
**Interfaces consumes:** `route`, `RoutingRules`, `RouteReason`, `resolve_model_for_tier` (OH4), `BudgetState`.

- [ ] Là où `agent_model(agent)` calcule le modèle (llm_agents.rs:24) : si `agent.metadata.model == "latest:auto"`, appeler `route(input, &agent.metadata.tags, budget, rules)` → `resolve_model_for_tier`. `input` = le prompt de la contribution ; `rules` = `RoutingRules` chargées depuis la config projet (les passer à `run_orchestrated` → moteurs → agents) ou `Default`.
- [ ] `budget` : dériver un `BudgetState { remaining_ratio }` depuis le `token_budget` restant du board/ring (rapport tokens consommés / budget). Si pas de budget configuré → `None`.
- [ ] Émettre `RunEvent::Route { agent, tier, reason }` sur le chemin routé (via le sink).
- [ ] Modèles concrets / `latest:pro` inchangés (non-régression). PAS d'exit 3 sur épuisement budget (halt gracieux conservé).
- [ ] Tests : `route()` invoqué en orchestration pour `latest:auto` ; budget bas → downgrade. clippy 2 modes + fmt + full test ; commit `feat(orchestration): route latest:auto with effective budget downgrade`.

---

## Notes
- Si passer `input`/`rules`/`budget` jusqu'à `agent_model` alourdit trop la signature, préférer un petit struct de contexte de routage plutôt que 4 params épars.
- Émission `Delegate` en hierarchical : au point où `AgentController` parent instancie/appelle un enfant (chercher `AgentDelegateAction`/invocation dans hierarchical.rs).
