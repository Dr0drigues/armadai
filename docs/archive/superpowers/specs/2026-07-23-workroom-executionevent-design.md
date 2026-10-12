# Workroom piloté par les événements cœur (agnostique provider) — Design

## Contexte

La Workroom du shell (`src/shell/workroom.rs`) s'anime aujourd'hui uniquement quand le flux d'un CLI externe contient des marqueurs-commentaires `<!--ARMADAI_DELEGATE/META/END-->`. Diagnostic empirique (systematic-debugging, 2026-07-23, cf. mémoire `project_workroom_marker_emission`) : dans `armadai shell`, ArmadAI **relaie un CLI externe** (claude/gemini) — c'est ce CLI qui orchestre, pas ArmadAI. Or :
- **claude** délègue via l'outil natif `Agent`/Task (`tool_use` + `subagent_type`), n'émet pas les marqueurs de délégation ;
- **gemini** (et vraisemblablement codex/copilot) n'a **pas de concept de sous-agent** — il répond en agent unique.

Il n'existe donc **pas de norme de délégation uniforme** à lire dans la sortie hétérogène des providers. La bonne source agnostique existe déjà : **le moteur d'orchestration event-sourcé d'ArmadAI** (`src/core/orchestration/es/`) émet des `ExecutionEvent`, mappés par le bridge en **`RunEvent`** (`src/core/events.rs`) poussés en direct via `EventSink` (déjà utilisés pour la sortie `--json` de `armadai run`). Chaque agent n'est qu'un appel `Provider::complete/stream` (primitif mono-agent) ; c'est ArmadAI qui compose et émet les événements → **agnostique par construction**. Cette orientation s'aligne sur l'issue **#227** (« provider-neutral lifecycle policy engine ; les hooks Claude Code sont des adaptateurs *au-dessus* du cœur, pas la source de vérité »).

## Objectif

Faire de la Workroom un **consommateur du flux cœur `RunEvent`** afin qu'elle s'anime pendant `armadai run --orchestrate`, de façon **agnostique du provider**, en réutilisant les layouts et le drill-down livrés en T3 (hierarchical/blackboard/ring, ⌃W).

## Hors périmètre
- Le mode **relais du shell** (marqueurs / parsing `tool_use` claude) reste **inchangé/best-effort** — non touché par ce lot.
- L'**adaptateur hooks/plugin Claude Code** (mode hébergé fait proprement, alimentant le cœur en mode relais) — **brainstorm stratégique séparé**, aligné #227. Hors de ce lot.
- Le moteur lifecycle/policy complet de #227 — hors lot (on consomme le flux `RunEvent` existant).

## Architecture

Séparation **projection / rendu** (philosophie event-sourcing) :

### 1. Projection pure — `Workroom::on_run_event(&RunEvent)`
Fonction sans I/O, testable unitairement, qui applique un `RunEvent` à l'état de la Workroom. Réutilise l'état/API existants (`TrackedAgent`, `AgentState`, `pattern`, `on_complete`, transitions/last_action). Mapping :

| `RunEvent` | Effet Workroom |
|---|---|
| `RunStart { agents, .. }` | (re)seed de la flotte : agents connus en `Idle` (rôles issus de la config d'orchestration déjà chargée ; à défaut, tous `Agent`). `visible = true`. |
| `AgentStart { agent }` | agent → `Working`, `started_at = now` (via l'horloge injectée, cf. tests). |
| `AgentEnd { agent }` | agent → `Done`, `finished_at`, `last_action` = extrait de `content`. |
| `Delegate { from, to }` | `from` → `Delegating` ; `to` = agent actif/détenteur du jeton (ring) ou enfant courant (hierarchical) ; enregistre la transition. |
| `Vote { agent, conf }` | `last_action(agent)` = `vote {conf}` (enrichissement ring). |
| `Board { agent, kind }` | `last_action(agent)` = `board {kind}` (enrichissement blackboard). |
| `NestedStart { team_lead, pattern }` | `team_lead` → `Delegating`, note le sous-pattern. |
| `NestedEnd { team_lead }` | `team_lead` → `Done`. |
| `AgentSelect { selected, .. }` | marque les `selected` actifs (blackboard/auto). |
| `Route { agent, tier, .. }` | `last_action(agent)` = `→ {tier}` (info). |
| `Result { .. }` | fin de run → `on_complete()`. |
| `Error { .. }` | état d'erreur (agent courant / global). |
| `Warning`, autres | ignorés (no-op). |

**Injection d'horloge** : `started_at`/`finished_at` utilisent `Instant`. `Instant::now()` n'est pas testable de façon déterministe ; la projection prend le temps via un paramètre/closure (ou une méthode `on_run_event_at(ev, now)`) pour des tests déterministes. Le renderer passe `Instant::now()`.

### 2. Pont — `WorkroomSink` (implémente `EventSink`)
`struct WorkroomSink { tx: Sender<RunEvent> }` : `emit(&RunEvent)` clone l'événement et le pousse dans un channel (`std::sync::mpsc` ou `tokio`). Le moteur `run --orchestrate` reçoit ce sink (en plus, ou à la place, du sink stdout selon le mode — voir Rendu) et tourne dans une tâche async ; la boucle TUI draine le channel et applique `on_run_event`.

### 3. Renderer — vue live sur `armadai run --orchestrate` (feature `tui`)
Boucle ratatui minimale (alternate screen), réutilisant le rendu Workroom de T3 :
- lance l'orchestration en tâche async (sink = `WorkroomSink`) ;
- draine le channel (non bloquant) → `workroom.on_run_event_at(ev, Instant::now())` → redraw ; `tick()` pour les spinners ;
- supporte le drill-down ⌃W (déjà en place) ;
- à la réception de `Result`/fin de tâche : restaure le terminal (leave alternate screen) et imprime le résultat/summary normal (comportement headless actuel préservé).

## Rendu — déclenchement
- **Live TUI par défaut quand `stdout` est un TTY** et que le run n'est ni `--json` ni `--quiet`.
- Flag **`--no-tui`** pour forcer la sortie plate.
- `--json` / `--quiet` / stdout non-TTY (pipe, CI, e2e) → **pas de TUI**, comportement headless actuel **strictement inchangé** (le sink stdout/json reste seul).
- Placement : `armadai run --orchestrate` uniquement (le chemin `armadai shell` relais n'est pas concerné par ce lot).

## Tests

### Unitaires (projection, feature `tui`, pas d'accès env)
- `on_run_event_at` : chaque variante → effet attendu (`AgentStart`→Working, `AgentEnd`→Done + last_action, `Delegate`→from Delegating/to actif, `NestedStart/End`→delegating/done, `AgentSelect`→actifs, `Result`→on_complete). Horloge injectée pour un `started_at`/elapsed déterministe.
- Séquence réaliste hierarchical/blackboard/ring → état final de la flotte attendu.

### Intégration (sink → channel → projection, sans terminal)
- Pousser une séquence de `RunEvent` via `WorkroomSink`, drainer, appliquer, vérifier l'état projeté final. Confirme le round-trip sink↔channel↔projection sans dépendre d'un terminal réel.

### e2e (harnais `tests/e2e/`, systématiques par pattern)
Le harnais lance le **vrai binaire** `armadai run` avec le provider **`fake-claude`** (réponses scriptées déterministes) et asserte la **séquence `RunEvent`** en `--json`. On s'y raccorde :
- **Assertion de projection Workroom réutilisable** : rejouer le flux `RunEvent` capturé d'un cas à travers `on_run_event_at` et asserter l'**état final de la flotte** (working/done par agent, détenteur de jeton en ring, etc.) — ajoutée aux cas `hierarchical`, `blackboard`, `ring`. Déterministe, agnostique, sans TUI.
- **Cas `--no-tui`** : confirmer que le flag laisse la sortie plate/headless intacte (mêmes events, exit 0).
- Le flux `RunEvent` sous-jacent est déjà asservi par les cas existants (`hierarchical/blackboard/ring/nested/direct`), donc la source que consomme la Workroom est couverte de bout en bout.

**Évolution du harnais (note, hors lot)** : ajouter un **`fake-gemini`** et un **faux modèle générique** (mappé par un proxy LLM ou stub) permettrait de *prouver* en e2e l'agnosticité multi-provider (même flux `RunEvent`, providers différents). Non requis pour ce lot (la Workroom ne dépend pas du provider ; `fake-claude` suffit comme primitif mono-agent), mais à cadrer ensuite.

### Visuel
La vue live TUI = validation manuelle Dimitri : `armadai run --orchestrate` sur les demos `examples/orchestration-patterns/{hierarchical,ring,blackboard}` (avec une clé API ou `fake-claude`), fond clair.

## Gate & CI
Clippy **3 modes** (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test` (dont e2e). Feature `tui` pour le renderer et le sink TUI ; la projection reste sous `tui` (la Workroom l'est déjà). PR + revue indépendante + validation visuelle Dimitri avant merge sur `release/1.0.0`.

## Risques
- **Boucle TUI + orchestration async** : bien séparer la tâche moteur (async) et la boucle de rendu (draine le channel, non bloquant) ; restaurer le terminal proprement même en cas d'erreur/panic (guard). 
- **Seed des rôles** : `RunStart` ne porte pas les rôles ; les dériver de la config d'orchestration chargée (comme `init_from_config`) pour que le layout hierarchical soit correct — sinon dégrader en flotte plate.
- **Restauration terminal** : réutiliser le pattern d'entrée/sortie alternate-screen du shell (`src/shell/app.rs`) pour éviter un terminal cassé.
