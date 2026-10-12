# Suppression des moteurs d'orchestration legacy — Plan

**But :** retirer les moteurs d'orchestration legacy (hiérarchique/blackboard/ring impératifs) devenus du code mort en prod depuis la bascule du chemin `run` sur les moteurs event-sourcés (`src/core/orchestration/es/*`), sans changement de comportement.

**Nature :** suppression de code mort, guidée par le compilateur (retirer un symbole encore utilisé = échec de compilation ; casser l'ES = échec des tests unit/e2e). Aucun changement comportemental attendu.

**Gate après CHAQUE étape :** `cargo build --no-default-features --features tui,providers-api` OK, puis `cargo test --no-default-features --features tui,providers-api` vert. Gate final : clippy 2 modes (`tui` et `tui,providers-api`) + `cargo fmt --check` + suite complète + e2e.

## Inventaire

### À SUPPRIMER (legacy, aucun caller PROD hors legacy)
- `hierarchical.rs` : `HierarchicalEngine`, `EngineState`, `EngineContext`, `run_nested_team`, `NestedRun` (enum), champ `OrchestrationResult.nested_runs`, tests inline associés.
- `blackboard.rs` : `run_blackboard` (497-632), `Board` (struct 17-, impl 181-), trait `BoardAgent` (336), tests inline.
- `ring.rs` : `run_ring` (475-), `RingToken` (struct 19-, impl 174-), trait `RingAgent`, tests inline.
- `llm_agents.rs` : `LlmBoardAgent` (273-), `LlmRingAgent` (437-), leurs impls de trait.
- `cli/run.rs` : fonctions `record_*` mortes, boucle `for nested in &result.nested_runs` (~2046), tests `#[cfg(test)]` référençant `HierarchicalEngine`/`NestedRun` (2117+, 2179+, 2509+).
- Fichiers de tests legacy : `e2e_tests.rs`, `gemini_integration_tests.rs` (à confirmer : testent-ils uniquement le legacy ?).

### À GARDER (consommé par le chemin ES — vérifié)
- `OrchestrationResult` (sauf `nested_runs`), `DelegationEvent` — utilisés par `es/bridge.rs::to_orchestration_result` + `run.rs`.
- `llm_agents.rs` : `RoutingCtx`, `parse_board_action`, `parse_ring_action`, `parse_vote_confidence`, `BOARD_ACTION_INSTRUCTIONS`, `RING_ACTION_INSTRUCTIONS` — importés par `es/blackboard.rs` et `es/ring.rs`.
- `BlackboardConfig`, `RingConfig`, `EntryKind`, `entry_kind_name`, `ContributionAction`, `context_injection`, `protocol`, `agent_selection`, `classifier`.

## Étapes (ordre imposé par les dépendances)

1. **Retirer `nested_runs`/`NestedRun`** (prérequis débloquant) : champ struct, enum, `es/bridge.rs:325` (`nested_runs: vec![]`) + asserts tests bridge (823/865), boucle `run.rs:2046`, tests run.rs (1995/2386/2409/2453). Gate.
2. **Retirer `run_nested_team`** + la machinerie C9 legacy dans `hierarchical.rs` (le nested délègue déjà vers l'ES en prod). Gate.
3. **Retirer `HierarchicalEngine`/`EngineState`/`EngineContext`** + tests directs (dont run.rs 2509+, e2e_tests.rs, gemini_integration_tests.rs). Gate.
4. **Retirer `run_blackboard` + `Board` + trait `BoardAgent`** + tests inline blackboard.rs. Gate.
5. **Retirer `run_ring` + `RingToken` + trait `RingAgent`** + tests inline ring.rs. Gate.
6. **Retirer `LlmBoardAgent`/`LlmRingAgent`** de llm_agents.rs (garder RoutingCtx/parse_*/INSTRUCTIONS). Gate.
7. **Retirer les `record_*` mortes** dans run.rs. Gate.
8. **Nettoyer les doc-comments** de `es/*.rs` référençant des types supprimés (mentions en backticks → reformuler ; aucun lien intra-doc `[Type]` détecté). Gate + `cargo doc` si pertinent.
9. **Gate final** : clippy 2 modes + fmt + suite complète + e2e (8 cas). Vérifier qu'aucune couverture ES n'a été perdue.

## Notes
- Si une suppression casse la compilation via un symbole GARDÉ, c'est que la carte est incomplète → réévaluer, ne pas forcer.
- Revue indépendante de toute la branche avant merge (exigence process release). Merge sur vert (non comportemental).
