# OH1 Lot 4 — Réconciliation ES (via harnais e2e) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Rendre la suite e2e (harnais mergé) **entièrement verte sur la branche Lot 4** (moteurs event-sourcés), en corrigeant les vrais écarts ES et en re-baselinant les comportements ES voulus, jusqu'à ce que le chemin `run` ES soit prêt pour la validation manuelle + merge.

**Architecture:** TDD via le harnais e2e (`tests/e2e/`, `cargo test --features tui,storage --test e2e`). Chaque task rend un ou plusieurs case files verts, soit par un fix moteur/bridge ES, soit par un re-baseline documenté.

**Tech Stack:** Rust edition 2024, moteurs ES (`src/core/orchestration/es/`), bridge (`es/bridge.rs`), CLI (`src/cli/run.rs`), harnais e2e.

## Global Constraints

- Base = branche `feat/oh1-lot4-switch-run` (@ c8a00f1, rebasée sur `release/1.0.0` = harnais e2e + #217). Travailler dessus (PAS une nouvelle branche).
- **Contrat d'observabilité acté** : flux `run --json` **per-tour** (un `agent_start`/`agent_end` par invocation). Les divergences de count blackboard/hierarchical = **re-baseline** (comportement ES correct), pas des bugs.
- Legacy NON supprimé (chemin parallèle mort). Ne pas toucher aux moteurs legacy.
- Après chaque task : `cargo test --no-default-features --features tui,storage --test e2e` + suite complète + clippy `-D warnings` (`tui`, `tui,providers-api`, `tui,storage`) + fmt.
- Diagnostics : cf. investigations (nested_start = ordre d'émission ; ring = ordre alphabétique + budget ; budget halt = `Warned` jeté par le bridge ; quiet = scoped direct).
- **Lot user-facing** : à la fin, la branche va en **validation manuelle Dimitri** avant merge (change `run`).

---

### Task 1: Fix ordre `Delegated`→`NestedStarted` + re-baseline `nested`

**Files:** Modify `src/core/orchestration/es/hierarchical.rs` (`invoke_actions`); `tests/e2e/cases/nested.yaml`.

**Problème:** `invoke_actions` (`es/hierarchical.rs` ~567-589) émet `NestedStarted` AVANT `Delegated` ; le legacy et le case attendent `delegate` PUIS `nested_start`. + le case attend `agent_start:4` alors que l'ES en produit 3 (expose le lead t-lead, masque les membres du sous-run isolé — comportement ES voulu).

- [ ] **Step 1: Run the e2e nested case to see it red** — `cargo test --no-default-features --features tui,storage --test e2e -- e2e_suite` (nested échoue : nested_start "not found in order" + counts 4≠3). Note le diff.
- [ ] **Step 2: Fix l'ordre d'émission** — dans `invoke_actions`, déplace le push du `delegation_event` (`Delegated`) AVANT le `nested_started_event` (`NestedStarted`). `model_routed` reste en tête. (Swap neutre : `Delegated`→`hier.trace`, `NestedStarted`→`open_nested`, indépendants.)
- [ ] **Step 3: Re-baseline `nested.yaml`** — mets l'ordre attendu `delegate` puis `nested_start` ; ajuste `event_counts` `agent_start`/`agent_end` de 4 à **3** ; ajoute un commentaire expliquant : per-tour ES + isolation du sous-run (membres non exposés au sink, lead exposé). Retire `allow_fail` si présent sur nested.
- [ ] **Step 4: Run e2e** — nested vert. Suite complète + clippy 3 modes + fmt.
- [ ] **Step 5: Commit** `git commit -m "fix(es): emit Delegated before NestedStarted (restore delegate->nested_start order) + rebaseline nested"`

---

### Task 2: Re-baseline `blackboard` + `hierarchical` (contrat per-tour)

**Files:** Modify `tests/e2e/cases/blackboard.yaml`, `tests/e2e/cases/hierarchical.yaml`.

**Problème:** counts `agent_start`/`agent_end` attendus 2 (blackboard) / 3 (hierarchical), l'ES en produit 4 / 4 (per-tour : rounds pour blackboard ; coordinateur delegate + subordonnés + coordinateur synthèse pour hierarchical). Comportement ES **voulu**.

- [ ] **Step 1: Run e2e** — blackboard/hierarchical rouges (counts). Note les valeurs ES réelles observées (via le rapport `target/e2e-report/e2e-report.json`).
- [ ] **Step 2: Re-baseline** — mets les `event_counts` attendus aux valeurs ES réelles (blackboard 4, hierarchical 4 — confirme via le rapport, n'invente pas). Ajoute un commentaire dans chaque case : « contrat per-tour ES (un start/end par invocation) — cf. décision 2026-07-22 ». Vérifie que les invariants (symétrie, single_result) restent asserts.
- [ ] **Step 3: Run e2e** — blackboard + hierarchical verts. (nested de T1 aussi.) Suite + clippy + fmt.
- [ ] **Step 4: Commit** `git commit -m "test(e2e): rebaseline blackboard/hierarchical event counts to ES per-turn contract"`

---

### Task 3: Bug A — ordre de circulation ring = ordre chaîne (+ vote/outcome)

**Files:** Modify `src/core/orchestration/es/ring.rs` (`run_ring_es`) et `src/cli/run.rs` (`dispatch_ring_es`) si besoin de plomber l'ordre ; `tests/e2e/cases/ring.yaml`.

**Problème:** `run_ring_es` fait `agent_order = agents.keys().collect()` (BTreeMap → **alphabétique**) au lieu de l'ordre de la chaîne `--pipe`. La circulation diverge de l'intention. (`es/ring.rs` ~1007.)

- [ ] **Step 1: Écris le case ring VOULU** — `ring.yaml` : agents `[t-a, t-b, t-c]` (ordre chaîne), fake scripté (contributions PROPOSE puis votes CONFIDENCE) → `expect` : circulation dans l'**ordre chaîne** (assert l'ordre des `agent_start` = t-a, t-b, t-c via `events` ordonnés), présence de `vote`, `outcome_resolved`/`result`. **Retire `allow_fail`**. Lance → rouge (ordre alpha).
- [ ] **Step 2: Fix l'ordre** — `run_ring_es` doit utiliser l'ordre de la chaîne pour `agent_order` ET `RunStarted.agents`, pas `agents.keys()`. Comme `agents: BTreeMap` perd l'ordre, ajoute un paramètre `agent_order: &[String]` (ou `Vec<String>`) à `run_ring_es` (l'ordre du roster/chaîne), passé par `dispatch_ring_es` (`run.rs`, qui a la chaîne avant de construire la BTreeMap). Le `RingDecider.agent_order` reçoit cet ordre. (Vérifie que blackboard/hierarchical n'ont pas le même besoin d'ordre sémantique — sinon note-le ; blackboard = ordre des éligibles, hierarchical = coordinator-driven, moins critiques ; applique le même plombage si trivial.)
- [ ] **Step 3: Run e2e** — ring vert (ordre chaîne + vote + outcome). Suite + clippy + fmt.
- [ ] **Step 4: Commit** `git commit -m "fix(es): ring circulates in chain order (not BTreeMap alphabetical)"`

---

### Task 4: Bug B — halt budget/cost visible (`Warned` → `RunEvent::Warning`)

**Files:** Modify `src/core/orchestration/es/bridge.rs` (`map_execution_to_run_events`); create `tests/e2e/cases/budget-halt-visible.yaml`.

**Problème:** le bridge mappe `Warned{code}` → `[]` (jeté) → un halt budget/cost est **invisible** en `--json`. (`es/bridge.rs` ~179.)

- [ ] **Step 1: Écris le case `budget-halt-visible.yaml`** — un pattern (ex. ring ou blackboard) avec un `token_budget` bas (via `setup`/orchestration config ou des `tokens_in`/`tokens_out` élevés dans les `fake.rules` pour dépasser vite) → `expect` : présence d'un event `warning` (avec `code`/champ correspondant), `result` (partiel) non vide, `exit_code: 0` (halt gracieux). Lance → rouge (pas de `warning`).
   - Vérifie le vrai variant `RunEvent::Warning` (dans `src/core/events.rs`) : son type `t` (`warning` ?) et ses champs (`code`/`msg`), pour écrire l'assertion + le mapping.
   - Vérifie comment injecter un budget bas dans le case : si `Setup` ne le permet pas, ajoute un champ optionnel `token_budget`/`cost_limit` à `Setup` (serde default) + génère-le dans `armadai.yaml` (`orchestration.token_budget`/`cost_limit`), OU scripte des tokens élevés dans le fake.
- [ ] **Step 2: Fix le bridge** — `map_execution_to_run_events` mappe `ExecutionEvent::Warned{code}` → `vec![RunEvent::Warning{...}]` (champs réels). Vérifie que ça n'introduit pas de doublon ni ne casse d'autres cases (relance toute la suite e2e).
- [ ] **Step 3: Run e2e** — budget-halt-visible vert + non-régression des autres cases. Suite + clippy + fmt.
- [ ] **Step 4: Commit** `git commit -m "fix(es): bridge projects Warned as RunEvent::Warning (visible budget/cost halt)"`

---

### Task 5: `--quiet` (+ `--max-content`) sur les chemins orchestrés

**Files:** Modify `src/cli/run.rs` (`dispatch_{hierarchical,blackboard,ring}_es`); create `tests/e2e/cases/quiet-orchestrated.yaml`.

**Problème:** `QuietMaxContentSink` (introduit au Lot 4) est scoped au chemin direct ; sur l'orchestré, `--quiet`/`--max-content` sont ignorés.

- [ ] **Step 1: Écris `quiet-orchestrated.yaml`** — un run blackboard (ou hierarchical) avec `flags: ["--json", "--quiet"]` → `expect` : **seul l'event `result`** (pas de `agent_start`/`agent_end`/`board`…). Lance → rouge (events intermédiaires présents).
   - (Confirme la sémantique voulue de `--quiet` : « n'émettre que `result` » — cf. l'aide CLI `src/cli/mod.rs`. Le `QuietMaxContentSink` du direct droppe `agent_end` ; pour « result seul » il faut dropper tous les events sauf `result` — ajuste le décorateur en conséquence.)
- [ ] **Step 2: Fix** — enveloppe le sink des 3 `dispatch_*_es` orchestrés avec le `QuietMaxContentSink` (ou une version « result-only ») quand `quiet` / `max_content`. Factorise proprement (le direct l'utilise déjà). Ne touche pas `--pipe`/legacy.
- [ ] **Step 3: Run e2e** — quiet-orchestrated vert + non-régression. Suite + clippy + fmt.
- [ ] **Step 4: Commit** `git commit -m "fix(cli): honor --quiet/--max-content on orchestrated ES run paths"`

---

## Notes pour l'implémenteur
- La branche a DÉJÀ le harnais e2e (`tests/e2e/`) + le fix #217. Ne les re-crée pas.
- Chaque task doit laisser `cargo test --no-default-features --features tui,storage --test e2e` VERT (le test agrégateur `e2e_suite` échoue si un cas non-`allow_fail` échoue). Lis `target/e2e-report/e2e-report.json` pour les diffs.
- Re-baseline = mettre l'attendu au comportement ES RÉEL observé (via le rapport), pas à une valeur devinée ; documente pourquoi c'est le comportement voulu.
- Fixes moteur = strictement dans `es/` + `run.rs`/`bridge.rs` ; ne touche pas au legacy.
- À la fin des 5 tasks, TOUS les cas doivent être verts (idéalement plus aucun `allow_fail`) → prêt pour validation manuelle Dimitri.
