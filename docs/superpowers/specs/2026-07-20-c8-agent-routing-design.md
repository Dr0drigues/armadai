# C8 — Routage déclaratif d'agents (déterministe)

> **Statut** : design validé (brainstorm 2026-07-20)
> **Cible** : axe 2, feature C8, prévue pour la **rc.3** (avec la vue squad TUI, spec séparé).
> **Base** : `release/1.0.0` (@ `9952f9f`, après tag rc.2). Le classifier de pattern (`Auto`) et le routing modèle `latest:auto` (OH4) existent et ne sont pas retouchés.

## 1. Objectif

Sélectionner **quels agents du roster** participent à un run orchestré, de façon **déterministe et auditable**, via deux voies combinables : **routes nommées** et **tags de capacité**. Rétro-compatible : sans option, comportement **identique** (roster complet).

Décision de conception clé (brainstorm) : pas d'inférence floue depuis le texte de la tâche (un hash ne généralise pas ; une extraction sémantique ramène au problème flou). La clé de sélection est **explicite** (route nommée ou tags fournis), ce qui rend le routage déterministe et lisible.

Cible : patterns à **roster plat** — **blackboard / ring** (+ direct). **Hierarchical est hors périmètre** : sa topologie coordinateur+teams *est* déjà le routage.

## 2. Les deux voies (combinables)

### Routes nommées
Table déclarative dans `armadai.yaml` :
```yaml
orchestration:
  routes:
    security-audit: [rust-security, rust-reviewer, qa-specialist]
    frontend-review: [ui-specialist, qa-specialist]
```
Sélection via `armadai run --route security-audit`. Lookup pur.

### Tags de capacité
Les agents portent déjà `tags` et `stacks` en métadata (`AgentMetadata`). La tâche porte des tags explicites : `armadai run --tags security,rust`. On retient les agents du roster dont `tags ∪ stacks` **intersecte** l'ensemble demandé (comparaison insensible à la casse). Set membership déterministe, aucune table séparée à maintenir.

## 3. Précédence

| Options | Sélection |
|---|---|
| aucune | roster complet (comportement actuel) |
| `--route R` seul | les agents listés par la route R |
| `--tags T` seul | roster filtré par intersection de tags |
| `--route R` + `--tags T` | la route R définit le **vivier**, `--tags` **raffine** dedans |

- **Vivier pour les tags = le roster fourni au run** (`--pipe` / config d'orchestration), **jamais** toute la bibliothèque (décision brainstorm : borné, prévisible).
- Les agents d'une route doivent être résolvables (présents dans la bibliothèque projet/user, comme aujourd'hui pour `--pipe`). La route peut nommer des agents hors du roster `--pipe` initial : dans ce cas la route **remplace** le roster (elle est la source explicite).

## 4. Architecture

- Nouveau : `OrchestrationConfig.routes: BTreeMap<String, Vec<String>>` (`#[serde(default)]`, ordre déterministe). Vide = pas de routes.
- Fonction pure `select_agents` dans `core/orchestration/` (nouveau module `agent_selection.rs` ou dans `classifier.rs`) :
  ```rust
  pub struct AgentSelection { pub agents: Vec<String>, pub reason: String }

  pub enum SelectionError {
      UnknownRoute { name: String, known: Vec<String> },
      NoMatch { tags: Vec<String>, roster: Vec<String> },
  }

  /// Deterministic. `roster` = the run's agent names; `route`/`tags` = explicit
  /// selectors; `routes` = the configured table; `agent_tags` maps an agent
  /// name to its (tags ∪ stacks) for tag matching.
  pub fn select_agents(
      roster: &[String],
      route: Option<&str>,
      tags: &[String],
      routes: &BTreeMap<String, Vec<String>>,
      agent_tags: &dyn Fn(&str) -> Vec<String>,
  ) -> Result<AgentSelection, SelectionError>;
  ```
  - route seul → `routes[route]` (erreur `UnknownRoute` si absent).
  - tags seuls → `roster.filter(|a| intersect(agent_tags(a), tags))` (erreur `NoMatch` si vide).
  - route + tags → pool = `routes[route]`, puis filtre tags dedans (erreur `NoMatch` si vide).
  - ni l'un ni l'autre → `roster` inchangé, reason « no routing (full roster) ».
  - `reason` décrit la voie et le résultat (ex. « route 'security-audit' → 3 agents » / « tags [security,rust] matched 2/5 roster agents »).

## 5. Événement

Nouveau `RunEvent::AgentSelect { selected: Vec<String>, reason: String }` (clés courtes JSONL, `t:"agent_select"`), émis une fois après la sélection, avant l'exécution. Transparence côté headless/TUI.

## 6. Garde-fous

- Route inconnue → erreur claire listant les routes connues.
- 0 agent après filtrage → erreur actionnable (tags demandés + roster examiné).
- **blackboard/ring exigent ≥2 agents** : si la sélection tombe à <2, erreur explicite (« le routage a retenu N agent(s) ; blackboard/ring en demande ≥2 »). (direct accepte 1.)
- `--route`/`--tags` sur un run **hierarchical** → `RunEvent::Warning{code:"routing_ignored_hierarchical"}` + log, et on continue avec la topologie config (sélection non appliquée).

## 7. Aperçu sans exécution — `--dry-run`

`armadai run --orchestrate blackboard --route security-audit --dry-run "<tâche>"` :
- Résout la sélection, affiche **agents retenus + raison + pattern effectif**, **sans exécuter** (0 token, aucun provider créé), exit 0.
- Réalise l'item backlog `dispatch --dry` : tester les règles de routage à coût nul.
- En mode `--json`, émet `AgentSelect` puis un `Result`-like minimal (ou juste `AgentSelect`) sans lancer les agents.

## 8. Composition (orthogonalité)

Trois étages indépendants, dans l'ordre : **quels agents** (C8, sélection) → **quel pattern** (classifier `Auto` sur le sous-ensemble) → **quel modèle** (`latest:auto`, OH4). C8 tourne en premier ; le classifier et le routing modèle opèrent ensuite sur les agents retenus.

## 9. Surface CLI

Sur la commande `run` : `--route <name>`, `--tags <t1,t2,…>` (CSV), `--dry-run`. Documenté dans l'aide clap.

## 10. Tests

- **Lot A** : `select_agents` — route seule (ok + UnknownRoute) ; tags seuls (intersection, casse, NoMatch) ; route+tags (raffinement, NoMatch) ; ni l'un ni l'autre (roster complet) ; déterminisme (même entrée → même sortie) ; garde-fou <2 agents non testé ici (c'est au niveau CLI/pattern). Sérialisation `AgentSelect` (clés courtes).
- **Lot B** : parsing des flags ; câblage dans `run_orchestrated` (sélection appliquée avant build providers) ; erreur <2 agents pour blackboard/ring ; warning hierarchical ; `--dry-run` n'exécute pas (aucun provider, sortie attendue) ; non-régression : sans flags, roster complet inchangé.

## 11. Contraintes CI (tous lots)

- Clippy 2 modes : `--no-default-features --features tui -- -D warnings` ET `--features tui,providers-api -- -D warnings`. `cargo fmt -- --check`. `cargo test`.

## 12. Découpage

- **Lot A — cœur** : `routes` config, `select_agents` + `SelectionError`, event `AgentSelect`, tests unitaires. Aucune dépendance CLI.
- **Lot B — CLI** : flags `--route`/`--tags`/`--dry-run` sur `run`, câblage dans `run_orchestrated` (sélection avant providers), garde-fou <2 + warning hierarchical, tests.

Deux PRs vers `release/1.0.0`, revue indépendante avant merge.

## 13. Hors périmètre

- Sélection LLM / hybride (le déterministe explicite est retenu ; une couche LLM resterait une évolution future).
- Application de C8 au pattern hierarchical (topologie = routage).
- Vivier = toute la bibliothèque (borné au roster).
- Vue squad TUI event-based → spec séparé (même rc.3).
