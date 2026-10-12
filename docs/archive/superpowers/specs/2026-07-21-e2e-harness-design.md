# Harnais e2e déterministe (fake `claude` + case files + rapport) — Design

> **Statut** : design validé (brainstorm 2026-07-21/22 avec Dimitri). Prêt pour writing-plans après relecture.
> **Motivation** : la validation manuelle du Lot 4 (bascule `run`→moteurs event-sourcés) a révélé 2 bugs (ordre de circulation ring alphabétique ; halt budget silencieux) que ~1019 tests unitaires/intégration n'avaient PAS vus — parce qu'ils utilisent des mocks courts/déterministes qui n'exercent jamais le vrai binaire `armadai run` de bout en bout. Il manque un **banc e2e déterministe** qui lance le CLI réel contre un provider stub et **assert le flux JSONL + les codes de sortie + le storage**.

---

## 1. Décisions verrouillées (brainstorm)

- **Sandbox léger, PAS de Nix** : tests d'intégration Rust (`assert_cmd` + `tempfile`), dans le job CI existant (`--features tui,storage`), zéro dépendance externe.
- **Fake `claude` = moteur de règles piloté par scénarios déclaratifs** : moteur écrit UNE fois ; chaque cas = des données. Aucune condition en dur.
- **Case file unique = tout le test en déclaratif** : setup **+** réponses du fake **+** assertions attendues **+** poids. Un runner Rust générique découvre, exécute, assert, et produit le rapport. → un test = **un fichier YAML** (agent-authorable).
- **Schema (schemars)** : les structs Rust du case file dérivent `JsonSchema` → schéma unique de vérité + validation serde au chargement (sécurise le YAML généré, y compris par un agent). Pas d'outil externe à installer.
- **Rapport HTML pondéré** habillé du **ArmadAI Design System** (tokens self-hosted), avec diff attendu-vs-obtenu, uploadé comme artefact CI.
- **TDD red-first** : les Bugs A/B/`--quiet` du Lot 4 deviennent des scénarios 🔴→🟢. Baselines 🟢 verrouillent le contrat.
- **Intégré à l'agentique** : ownership **qa-specialist** ; **génération de case files par un agent** depuis une spec NL (contrainte par le schema) ; **record→replay depuis le log OH1** (mode d'authoring en plus).
- **Cible provider = le CLI `claude`** (shell-out) via un fake sur le `PATH`. Les providers HTTP ont leurs propres tests.

---

## 2. Architecture

```
tests/e2e/
  cases/*.yaml            # les cas (données : setup + fake + expect + weight)
  harness.rs             # runner générique : discover → run → assert → report
  fixtures/agents/*.md   # gabarits d'agents de test (portent FAKE_AGENT_ID)
src/bin/fake-claude.rs    # (ou [[bin]] gated) moteur de règles → émet le format claude
target/e2e-report.{json,html}   # rapport (uploadé en CI)
```
Flux d'un cas : le runner monte un `tempdir` (project `armadai.yaml` + agents), écrit le scénario, préfixe `PATH` avec `fake-claude`, lance `armadai run … --json` via `assert_cmd`, capture stdout(JSONL)/stderr/exit, parse les events, vérifie le bloc `expect`, agrège dans le rapport.

---

## 3. Le fake `claude` (moteur de règles)

Binaire Rust `fake-claude` (un `[[bin]]`, idéalement `required-features` de test pour ne pas alourdir `cargo build` par défaut). `assert_cmd` récupère son chemin via `env!("CARGO_BIN_EXE_fake-claude")` et préfixe le `PATH` du sous-process `armadai`, pour que le shell-out `claude` l'atteigne. Zéro dépendance runtime externe.

À chaque invocation :
1. Charge le scénario `FAKE_SCENARIO=<path>` (bloc `fake.rules` du case file, ou fichier dédié).
2. Lit le prompt reçu (**stdin et/ou args — à confirmer**, cf. §11) et en extrait l'identité agent via le marqueur `FAKE_AGENT_ID: <id>` présent dans le system prompt (on contrôle les agents de test → fiable, pas de matching fragile).
3. Évalue les `rules` dans l'ordre (première qui matche gagne).
4. Avance un compteur d'appel par agent (fichier d'état sous `FAKE_STATE_DIR`), pour distinguer les tours (round/lap/synthèse).
5. Émet la réponse **au format stdout attendu par `providers/cli.rs`** (probablement stream-json avec `usage` — cf. §11), applique `latency_ms` (défaut 0), sort avec `exit_code` (défaut 0).

**Prédicats `match`** (tous optionnels, AND) : `agent`, `call` (Nᵉ appel), `prompt_contains`. `{}` = catch-all.
**Réponse** : texte brut avec les marqueurs protocole voulus (`@délégation`, `ACTION:/TARGET:/CONFIDENCE:/CONTENT:`, `CONFIDENCE:` de vote). Champs optionnels : `tokens_in/out`, `cost`, `latency_ms`, `exit_code`.

---

## 4. Case file (déclaratif — réponses ET assertions)

```yaml
name: hierarchical-delegates-and-synthesizes
weight: 3                          # pondération du score (baseline critique > cas bord)
setup:
  pattern: hierarchical            # génère armadai.yaml + agents de test voulus
  agents: [t-coordinator, t-analyst, t-writer, t-reviewer]
  flags: [--json]                  # flags de `armadai run`
  input: "ship the auth refactor"
fake:                              # règles du stub (réponses)
  rules:
    - match: { agent: t-coordinator, call: 1 }
      respond: "@t-analyst: analyse\n@t-writer: rédige\n@t-reviewer: relis"
    - match: { agent: t-coordinator, call: 2 }
      respond: "Synthèse finale."
    - match: {}
      respond: "ok"
      tokens_in: 10
      tokens_out: 5
expect:                            # assertions, déclaratives
  exit_code: 0
  events:                          # présence + ordre partiel + champs
    - { t: run_start }
    - { t: delegate, from: t-coordinator, to: t-analyst }
    - { t: result }
  event_counts: { agent_start: 4, agent_end: 4 }   # dénombrements
  invariants: [agent_start_end_symmetric, prov_model_non_empty, single_result]
  storage:                         # optionnel (feature storage) : rows attendues
    runs: 1
```
Types Rust dérivant `serde::Deserialize` + `schemars::JsonSchema` → un **JSON Schema** publié (`docs/e2e-case.schema.json`) sert de contrat pour l'écriture manuelle ET la génération par agent ; le chargement serde rejette tout YAML invalide avec une erreur claire. Extensible sans toucher au runner (nouveaux champs `match`/`expect` optionnels).

**Invariants réutilisables** (nommés, évalués par le runner) : `agent_start_end_symmetric`, `prov_model_non_empty`, `single_result`, `no_orphan_events`, etc.

---

## 5. Runner générique (`tests/e2e/harness.rs`)

- Découvre tous les `cases/*.yaml`, désérialise (schema-validé).
- Pour chaque cas : monte le tempdir (project + agents `fixtures/agents` avec `FAKE_AGENT_ID`), écrit le scénario, `PATH`=fake-claude, `FAKE_SCENARIO`/`FAKE_STATE_DIR`, lance `assert_cmd::Command::cargo_bin("armadai").args(...)`, capture stdout/stderr/exit.
- Parse le JSONL en `RunEvent`, évalue `expect` (events présents + ordre partiel + champs + counts + invariants + storage rows).
- Agrège : par cas `{ name, weight, status, expected, observed, diff }` + **score pondéré** global → écrit `target/e2e-report.json` puis `e2e-report.html`.
- Un test `#[test] fn e2e_suite()` fait échouer le job si un cas non-`#[ignore]`/non-`allow_fail` échoue.

---

## 6. Rapport (JSON + HTML, habillé du Design System)

- `e2e-report.json` : machine-readable (score pondéré, par cas : statut/poids/expected/observed/diff).
- `e2e-report.html` : **self-contained**, habillé des **tokens du ArmadAI Design System** (palette pont-de-commandement, IBM Plex + icônes **inlinés self-hosted** — la CSP interdit les CDN), thème clair/sombre. Contenu : jauge de score pondéré + chips (pass/fail/scénarios/patterns), table des scénarios (poids + badge signal), et par échec un **diff attendu-vs-obtenu** (events JSONL côte à côte, divergences/manquants surlignés) + cause. C'est le rendu déjà maquetté (à re-skinner sur les vrais tokens ; la maquette jetable est remplacée).
- CI : uploadé comme **artefact du job** (consultable après le run). Local : fichier ouvrable.
- Réutilise le même moteur de rendu que `armadai view` (§CLI) et l'History quand pertinent.

---

## 7. Stratégie TDD & matrice

**Red-first — Bugs Lot 4 (le case file encode le comportement VOULU) :**
| Case | Assertion | État |
|---|---|---|
| `ring-order` | ordre de circulation = ordre chaîne (t-analyst,t-writer,t-reviewer), tous circulent, `vote` présent, `outcome_resolved` | 🔴 Bug A |
| `budget-halt-visible` | budget bas → `warning{code:token_budget}` **présent** + `result` partiel + exit 0 | 🔴 Bug B |
| `quiet-orchestrated` | blackboard `--quiet` → **`result` seul** | 🔴 (quiet scoped direct) |

**Green baselines (verrouillent le contrat) :** direct (prov/model réels, quiet, max-content) · hierarchical (delegate + synthèse) · blackboard (rounds→consensus) · ring (circulation ordre chaîne→vote→outcome) · nested-c9 (nested_start/end) · invariants transverses (symétrie start/end, prov/model, single result).

**Pérenne** : tout nouveau pattern/feature/bug → nouveau case file 🔴 → impl → 🟢. Suite de non-régression vivante en CI.

---

## 8. Intégration agentique

- **Ownership qa-specialist** : les `tests/e2e/cases/` appartiennent au qa-specialist ; le workflow flotte = dev-lead délègue « ajoute une couverture e2e pour X » → qa-specialist écrit le case file + lance la suite + lit le rapport. (Mettre à jour le scope de l'agent starter + doc du format.)
- **Génération par agent depuis une spec NL** : un agent lit une description de comportement et produit le case file YAML valide (contraint par `e2e-case.schema.json`). Le format déclaratif (réponses + assertions comme données) rend ça possible sans écrire de Rust.
- **Record→replay depuis le log OH1** (mode en plus) : un vrai run event-sourcé est un log d'`ExecutionEvent` dont les `AgentObserved` capturent les réponses exactes. Un convertisseur `log → case file` transforme un run capturé en scénario déterministe (les `AgentObserved` deviennent les `fake.rules`), rejouable en CI. « Capture une fois, rejoue toujours » ; idéal pour figer un run qui a exposé un bug. **Dépend de la persistance du log** (OH1 Lot 5/6) — donc livré après.

---

## 9. Intégration CI

Les cas tournent dans le job `test` en `--features tui,storage`. `fake-claude` construit par cargo. Hermétique : aucun réseau, aucun LLM, DB in-memory (`init_embedded`), tempdirs jetables. Le rapport HTML est uploadé comme artefact.

---

## 10. Séquencement

1. **PR harnais** (vers `release/1.0.0`) : `fake-claude` + case format + schema + runner + rapport + **baselines 🟢** (direct/hier/blackboard/ring/nested). Infra permanente ; couvre rétroactivement les Lots 1-3. Les 3 cas de bug ajoutés en 🔴 marqués `allow_fail`/`#[ignore]` (ou portés dans la PR Lot 4).
2. **Branche Lot 4** (rebasée) : activer les 3 cas de bug + **corrections** — Bug A (ordre chaîne), Bug B (bridge projette `Warned`→`Warning`), `--quiet` orchestré → 🟢. Puis re-validation Dimitri → merge Lot 4 → PR séparée suppression legacy.

---

## 11. Détails à confirmer À L'IMPLÉMENTATION (investiguer en premier)

- **Format stdout attendu du CLI `claude`** par `providers/cli.rs`/`factory.rs` (stream-json + `usage` ? args ?) → le fake doit l'émettre fidèlement (sinon tokens/cost=0 et parsing KO). **Premier pas d'impl.**
- **Passage du prompt** (stdin vs fichier vs args) → où le fake lit le prompt + le marqueur `FAKE_AGENT_ID`.
- **`fake-claude`** : `[[bin]]` toujours compilé (léger) vs `required-features` de test.

---

## 12. Design system : usage

Le ArmadAI Design System (claude.ai/design, id `416749e1-…`) est la **source de vérité visuelle** — mais c'est une **spec (React/JSX + HTML previews)** ; on **ré-implémente** côté ratatui (Rust) et HTML axum. On en **extrait les tokens** (`tokens/*.css`, `terminal-palette.json`) + les specs de composants. Le **rapport HTML e2e** est la première surface concrète à l'utiliser (tokens + IBM Plex + icônes inlinés self-hosted, CSP-safe).

---

## 12bis. Addendum rapport — interpréteur HTML des rulesets (Dimitri 2026-07-22)
Évolution du rapport HTML (post-merge harnais) : un **interpréteur des rulesets** qui rend, par cas ET par règle/assertion, une **pastille succès/erreur** ; au **survol**, un **tooltip** affiche le détail (la règle du case file — `match`/`respond` — et l'assertion `expect` correspondante : attendu vs obtenu). But : comprendre d'un coup d'œil **quelle assertion précise** coince, sans quitter la page ni ouvrir les `.json`/`.err`. Concrètement : décomposer chaque cas en ses assertions (events attendus, event_counts, invariants) → une pastille par assertion + tooltip (attendu/obtenu/diff). Self-contained, thème clair/sombre, tokens « pont de commandement ». À spécifier en tâche dédiée quand on reprendra le rapport.

## 13. Hors périmètre
- Tests de concurrence/latence réelle (viendront avec le lot parallélisme ; `latency_ms` déjà prévu).
- Providers HTTP (anthropic/openai/google) : tests propres ; le harnais cible le provider CLI.
- Re-skin complet TUI/Web sur l'identité : chantiers séparés (le harnais ne fait que le rapport).
