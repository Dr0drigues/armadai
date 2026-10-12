# OH1 — Orchestration event-sourcée (log = source de vérité)

> **Statut** : design validé (brainstorm 2026-07-21)
> **Cible** : programme 1.0.0 (scope rouvert — cf. [[project_scope_unfrozen_v1]]). Premier gros pari, socle des suivants (moteur déclaratif, OH2, sessions reprenables C7).
> **Base** : `release/1.0.0` (@ rc.4). **Réécriture profonde** de `core/orchestration/` + `storage/`.

## 1. Objectif & décisions

Modéliser l'exécution comme un **event log immuable append-only** qui est **LA source de vérité**. L'état d'exécution est un **fold pur** sur ce log ; la **reprise** = replay du log ; l'**audit/replay** et la transmission inter-process en découlent.

Décisions (brainstorm) :
- **Event-sourcing PUR** : les moteurs deviennent événementiels (pas de dual-write, pas d'état mutable canonique hors du log).
- **Les 4 patterns d'un bloc** : hierarchical, blackboard, ring, **direct** (single-agent) réécrits ensemble.
- **Projections** : les tables plates (`runs`/`orchestration_runs`/`board_entries`/`ring_contributions`/`ring_votes`/`delegation_events`) **ET** le flux JSONL `RunEvent` (OH3) deviennent des **projections dérivées du log** — plus de source de vérité concurrente.

## 2. Concepts

- **`ExecutionEvent`** — enum exhaustif des faits d'exécution (voir §3). Immuable, horodaté, séquencé par run. Sérialisé JSON dans le log.
- **`ExecutionState`** — l'état reconstruit par fold. Contient ce dont les moteurs ont besoin pour décider la suite (agents, conversations, board/ring/hierarchy state, budget consommé, statut). `fn apply(&mut ExecutionState, &ExecutionEvent)` **pur** (aucun effet de bord, déterministe).
- **Effet** — un appel LLM (via `Provider`) : **non déterministe**, donc son **résultat est enregistré comme event** (`AgentObserved`) ; le replay ne ré-exécute jamais un effet.
- **Boucle moteur** (par pattern) = *lire `ExecutionState` → décider la/les prochaine(s) action(s) (déterministe) → exécuter l'effet → produire l'/les event(s) (dont l'observation) → append au log + fold → recommencer* jusqu'à `Halted`/`Completed`.
- **Replay** = fold du log sans exécuter d'effet (reconstruit l'état). **Resume** = replay puis reprise de la boucle depuis l'état reconstruit.

## 3. `ExecutionEvent` (log, source de vérité)

Enum unique couvrant les 4 patterns (clés courtes pour le stockage/JSONL) :

- **Communs** : `RunStarted { run_id, pattern, agents, input, project, ts }`, `AgentInvoked { agent, input_ref }`, `AgentObserved { agent, content, tokens_in, tokens_out, cost, model }`, `ModelRouted { agent, tier, reason }` (OH4), `Warned { code, .. }`, `Halted { reason }`, `Completed { content }`.
- **Hierarchical** : `Delegated { from, to, task, depth }`, `AskedPeer { from, to, question }`, `Escalated { from, to, message }`, `Synthesized { agent, content }`, `NestedStarted { team_lead, pattern }`, `NestedEnded { team_lead }`.
- **Blackboard** : `RoundStarted { round }`, `BoardEntryAdded { agent, round, kind, content, refs, confidence, tokens }`, `ConsensusReached { score }`.
- **Ring** : `LapStarted { lap }`, `ContributionAdded { agent, lap, position, action, content, tokens }`, `VoteCast { agent, position, confidence, supports, concerns }`, `OutcomeResolved { outcome }`.

`AgentObserved` porte le contenu complet de la réponse (indispensable au replay). Volumineux → stocké dans le log, tronqué seulement dans les projections d'affichage.

## 4. `ExecutionState` + reducer

- `ExecutionState { run_id, pattern, agents: Vec<String>, conversations: Map<agent, Vec<ChatMessage>>, hierarchy: HierState, board: BoardState, ring: RingState, budget: BudgetAcc, status: RunStatus }` (les sous-états ne sont peuplés que pour le pattern actif).
- `apply` : match sur l'event, met à jour le sous-état (ex. `BoardEntryAdded` → push dans `board.entries` + `budget += tokens` ; `Delegated` → trace + conversation ; `AgentObserved` → append assistant message + budget). **Pur, total, déterministe.**
- Invariant : `state == events.fold(apply)` à tout instant. Testé (property-ish : rejouer un log connu donne un état attendu).

## 5. Moteurs (réécriture des 4 patterns)

Chaque pattern implémente une **fonction de décision** `fn decide(&ExecutionState) -> Vec<Action>` (déterministe) + l'exécution des `Action` produisant des events :
- `Action::Invoke { agent, input }` → exécute le provider → `AgentInvoked` + `AgentObserved`.
- `Action::Halt { reason }` → `Halted`.
- etc.
La boucle générique `run_event_sourced(engine, state, sink, log)` est commune ; chaque pattern fournit `decide` + le mapping action→effet. **direct** = cas dégénéré (1 agent, invoke → observe → complete).

Réutilise les logiques métier existantes (parse délégations, résolution consensus ring, triggers blackboard, arbitrage nested C9, routing `latest:auto` OH4) — déplacées dans les `decide`/reducers, pas jetées.

## 6. Storage — le log + migration

- Nouvelle table **`execution_events`** (append-only) : `run_id TEXT, seq INTEGER, ts TEXT, kind TEXT, payload_json TEXT` (PK `(run_id, seq)`, index `run_id, seq`). Source de vérité.
- **Migration schema v3** (via le mécanisme `user_version` posé en C9 Lot 2) : créer `execution_events`. Les tables plates existantes sont **conservées comme projections** (voir §7) — pas supprimées (rétro-compat lecture + moindre risque), mais **remplies par le projecteur**, plus par les moteurs.
- Append **transactionnel** par event (durabilité : un crash laisse un log cohérent jusqu'au dernier event commité).

## 7. Projections (dérivées du log)

Un **projecteur** `project(events) -> {runs row, orchestration_runs row, board_entries, ring_*, delegation_events}` (fold dédié). Deux modes possibles (à trancher au plan) :
- **Matérialisées au fil de l'eau** : à chaque event appendé, mettre à jour la/les ligne(s) de projection (perf lecture History/Costs/C6 inchangée). Le log reste la vérité ; les tables = cache reconstructible.
- **Reconstruites à la demande** : les lectures foldent le log. Plus pur, mais coûteux pour History/Costs.
Recommandation : **matérialisées** (perf), reconstructibles depuis le log (commande `armadai projections rebuild` optionnelle).
- **JSONL `RunEvent`** (OH3/headless) = **projection en direct** : mapper chaque `ExecutionEvent` → 0..n `RunEvent` et l'émettre via le sink existant. `RunEvent` reste l'API d'observabilité ; il n'est plus produit indépendamment.

## 8. Reprise (C7) & audit/replay

- `armadai run --resume <run_id>` : charge `execution_events[run_id]`, fold → `ExecutionState`, puis reprend la boucle du pattern depuis l'état (statut ≠ Completed/Halted-terminal). Aucun effet re-exécuté (les observations sont dans le log).
- `armadai run --replay <run_id>` (audit) : fold + rejoue les events vers le sink d'affichage sans exécuter d'effet (visualisation déterministe).
- Transmission inter-process : le log est autosuffisant (un autre process peut reprendre/auditer).

## 9. Feature flags / CI

- Le log + reducer + moteurs = cœur, **toujours compilés** (edition 2024). La **persistance** du log est gated `storage` (comme les projections). Sans `storage`, l'exécution event-sourcée tourne **en mémoire** (log éphémère → pas de resume, mais même moteur) ; avec `storage`, log persisté + resume.
- Clippy CI 2 modes (`tui`, `tui,providers-api`) + `tui,web,storage` (projections web/C6). fmt. Tests.

## 10. Découpage (programme, une PR par lot, revue indépendante)

1. **Lot 1 — socle ES** : `ExecutionEvent`, `ExecutionState` + `apply` (reducer pur), la boucle générique `run_event_sourced`, un `EventLog` trait (InMemory + Sqlite gated storage), tests du fold/replay. Aucun moteur encore réécrit (le socle est prouvé isolément).
2. **Lot 2 — hierarchical event-sourcé** : `decide` hierarchical + arbitrage nested + `latest:auto`, câblé sur la boucle. Le pattern le plus complexe valide le socle.
3. **Lot 3 — blackboard + ring event-sourcés**.
4. **Lot 4 — direct event-sourcé** + bascule du chemin `run` (single + orchestration) sur les moteurs ES ; suppression de l'ancien état mutable.
5. **Lot 5 — projections** : projecteur → tables plates matérialisées + JSONL `RunEvent` dérivé ; brancher History/Costs/C6/headless sur les projections ; migration v3.
6. **Lot 6 — resume/replay CLI** : `--resume`/`--replay` + tests bout-en-bout (crash mid-run → resume).

## 11. Risques (à re-signaler à l'impl)
- **Surface de régression énorme** sur `core/orchestration/` + `storage/` + tous les chemins de lecture (History/Costs/C6/headless). Revue indépendante systématique + tests de non-régression exhaustifs (les ~850 tests actuels doivent rester verts à chaque lot).
- **Déterminisme du `decide`** : toute source non déterministe (hasard, temps, ordre parallèle) doit passer par un event enregistré, sinon le replay diverge. Les délégations parallèles (hierarchical) doivent enregistrer un ordre stable.
- **Volume du log** (`AgentObserved` = réponses complètes) : acceptable (SQLite), tronqué seulement en projection d'affichage.

## 12. Hors périmètre (autres chantiers du programme)
OH7 (extraction lib cœur) — s'articulera après/pendant selon l'appétit ; moteur déclaratif ; OH2/5/6 ; C4/stream-json. OH1 pose le state model dont ils dépendent.
