# OH3 — Mode headless CI-first pour `armadai run`

> **Statut** : design validé (brainstorm 2026-07-17)
> **Cible** : beta.3 (feature 1/2 ; OH4 routeur dynamique = spec séparée)
> **Origine** : enseignement OH3 de l'étude OpenHands (`docs/proposals/etude-openhands.md`)

## 1. Objectif

Rendre `armadai run` utilisable en CI/CD et par des outils tiers : exécution non-interactive, sortie structurée JSONL parsable, exit codes exploitables. Contrainte transverse : **économie de tokens**, sur deux axes — la sortie (souvent relue par un autre agent/LLM) et la consommation du run lui-même.

## 2. Surface CLI

Deux flags orthogonaux sur `run` (couvrant aussi `--pipe` et `--orchestrate`) :

| Flag | Effet |
|---|---|
| `--headless` | Désactive toute interactivité (aucun prompt `model_updater`) ; active les exit codes CI. Modèle déprécié → `warning` + alias résolu + continue. |
| `--json` | Émet le flux JSONL sur **stdout**. La sortie humaine (spinner, texte) bascule sur **stderr** pour ne pas polluer le flux. |
| `--quiet` | (avec `--json`) N'émet que l'événement `result` final (+ `error`). Pour ré-injection économe dans un autre agent. |
| `--max-content <n>` | Tronque le champ `content` des événements **intermédiaires** (`agent_end`) à `n` caractères. Le `result` final reste complet. |

`--json` et `--headless` sont découplés : `--headless` sans `--json` = run non-interactif à sortie texte ; `--json` implique la non-interactivité de fait (pas de prompt possible pendant l'émission machine).

## 3. Contrat d'événements JSONL

Une ligne = un objet JSON. Champ `t` = type. **Clés courtes** pour l'économie de tokens en aval. Schéma **versionné** (`v` dans `run_start`) pour un contrat CI stable.

```jsonl
{"t":"run_start","v":1,"agents":["dev-lead"],"prov":"claude","model":"claude-…","in_chars":412}
{"t":"agent_start","agent":"dev-lead","prov":"claude","model":"claude-…"}
{"t":"warning","code":"deprecated_model","from":"…","to":"…"}
{"t":"agent_end","agent":"dev-lead","tin":1200,"tout":830,"cost":0.014,"content":"…"}
{"t":"result","content":"…","tin":3100,"tout":1900,"cost":0.041,"agents":3}
{"t":"error","code":"budget_exceeded","msg":"…"}
```

### Types d'événements

| `t` | Champs | Émis quand |
|---|---|---|
| `run_start` | `v`, `agents[]`, `prov`, `model`, `in_chars` | Une fois, au début. |
| `agent_start` | `agent`, `prov`, `model` | Avant l'exécution de chaque agent mobilisé. |
| `agent_end` | `agent`, `tin`, `tout`, `cost`, `content` | À la fin de chaque agent (`content` tronçable via `--max-content`). |
| `warning` | `code`, + champs contextuels (`from`/`to`…) | Situation non bloquante (ex. `deprecated_model`). |
| `result` | `content`, `tin`, `tout`, `cost`, `agents` (nombre) | Une fois, à la fin (métriques agrégées, `content` complet). |
| `error` | `code`, `msg` | En cas d'échec (précède un exit non-zéro). |

Codes `warning`/`error` (chaîne stable) : `deprecated_model`, `budget_exceeded`, `provider_unavailable`, `agent_failed`, `usage_error`.

### Périmètre d'exécution

Couvre l'**agent simple**, le chaînage `--pipe`, et l'**orchestration** (`--orchestrate` hierarchical/ring/blackboard). L'orchestration émet au **niveau agent** (un `agent_start`/`agent_end` par agent mobilisé). L'instrumentation fine (delegate, contributions ring, votes, board entries) est **hors périmètre beta.3** — l'`enum RunEvent` est extensible pour l'ajouter plus tard sans casser le contrat.

## 4. Optimisation des tokens (transverse)

- **Aval (sortie)** : clés courtes ; `--quiet` (result-only) ; `--max-content` (troncature des `agent_end`, `result` final préservé).
- **Run (consommation)** : `tin`/`tout`/`cost` remontés par agent (`agent_end`) et agrégés dans `result`.
  - **Portée de l'exit 3 (budget/coût)** : ne s'applique qu'aux chemins qui **remontent l'épuisement en `Err`** (mappé par `exit_code_for` → `error` `budget_exceeded` + exit `3`). En **orchestration (beta.3)**, les 3 moteurs traitent l'épuisement `token_budget`/`cost_limit` comme un **halt gracieux** : ils retournent un **résultat partiel** (`Ok`, exit `0`), donc pas d'exit 3 sur ces chemins. Faire remonter l'épuisement orchestration en `error`+exit 3 est une **extension future** (nécessite que les moteurs signalent l'épuisement en `Err` plutôt qu'en outcome de succès).

## 5. Architecture

- **`core/events.rs`** (nouveau module) :
  - `enum RunEvent` (serde `Serialize`) — un variant par type, sérialisé avec les clés courtes ci-dessus (`#[serde(rename)]`).
  - `trait EventSink: Send + Sync { fn emit(&self, ev: &RunEvent); }`.
  - `struct NullSink` — no-op (mode normal, zéro overhead).
  - `struct JsonlSink { writer: Mutex<Box<dyn Write + Send>> }` — sérialise une ligne JSON par `emit`, écrit sur stdout (lock interne pour le dispatch parallèle).
- **`run::execute`** construit le sink selon les flags (`Arc<dyn EventSink>`), et le passe :
  - à l'exécution agent simple (émet `agent_start`/`agent_end` autour de `complete()`/`stream()`),
  - aux 3 moteurs d'orchestration (émettent aux points agent). Le dispatch parallèle (`tokio::spawn`) clone l'`Arc` — thread-safe via le `Mutex` interne du `JsonlSink`.
- Le mode non-headless passe un `NullSink` → aucun changement de comportement, aucun coût.

## 6. Exit codes CI

| Code | Signification |
|---|---|
| `0` | Succès |
| `1` | Erreur d'exécution (agent/provider en échec) |
| `2` | Erreur d'usage (arguments invalides) |
| `3` | Budget / coût dépassé — chemins qui remontent l'épuisement en `Err` (voir §4 : l'orchestration beta.3 halt gracieusement en exit `0`) |
| `4` | Provider indisponible |

## 7. Tests

- **Unitaires** : sérialisation de chaque `RunEvent` (schéma stable, clés courtes exactes) ; `JsonlSink` produit exactement une ligne JSON valide par `emit`, dans l'ordre.
- **Intégration (provider mock)** : bout-en-bout headless — agent simple ET orchestration → séquence d'événements attendue + exit codes ; `--quiet` (result seul) ; `--max-content` (troncature intermédiaires, result complet) ; `budget_exceeded` → exit `3`.
- **Non-régression** : sans `--json`/`--headless`, `NullSink`, sortie humaine et comportement inchangés ; `model_updater` toujours interactif hors `--headless`.

## 8. Hors périmètre

- OH4 (routeur dynamique de modèle) — spec séparée.
- Instrumentation fine des patterns d'orchestration (delegate/vote/board en JSONL).
- Flag `--strict` (transformer les warnings en échec) — reporté (YAGNI).
- Entrée `stream-json` bidirectionnelle (pilotage d'armadai par un flux entrant).
