# Plugin Claude Code → Workroom — P1 : reconstruction depuis le transcript — Design

**Date** : 2026-07-29
**Statut** : validé (design), à implémenter
**Cible** : post-OH7 (le cœur `armadai-core` expose `RunEvent`/`EventSink` et le Workroom les consomme déjà)

## Contexte & objectif

Le Workroom d'ArmadAI consomme déjà un flux agnostique de `RunEvent` (issu du cœur pour les runs `armadai`). Pour visualiser proprement une **session Claude Code** (là où aujourd'hui on *scrape* fragile­ment des marqueurs `<!--ARMADAI_DELEGATE-->` dans la sortie texte, cf. mémoire *reference_armadai_markers_leak* et *project_workroom_marker_emission* #244), on veut un **adaptateur** qui transforme l'activité d'une session Claude Code en `RunEvent`.

Décision de cadrage (validée) : la cible est **hybride** (hooks temps réel + transcript canonique) et sert **deux cas d'usage** (plugin autonome *et* relais piloté par armadai). C'est trop pour un seul spec → **découpage en 3 phases**, chacune son cycle spec→plan→impl :

- **P1 (ce spec)** — Reconstruction depuis le **transcript JSONL** (source canonique) + **plugin minimal** (hook `SessionStart` d'enregistrement) + attache Workroom. Livre une vue live+replay fiable d'une session Claude Code, avec tokens. Plugin installable en standalone.
- **P2** — Couche **hooks temps réel** (`async`) qui pousse des signaux basse-latence, fusionnés avec le transcript (canonique pour dédup/enrichissement). Atteint l'hybride.
- **P3** — Intégration **relais armadai** : `armadai shell/run` (provider claude) auto-active le plugin + ouvre le Workroom, et retire le scraping de marqueurs.

Ce document ne conçoit que **P1**.

## Faits établis (à revérifier à l'implémentation)

**Transcript JSONL** (inspecté sur un vrai fichier, `~/.claude/projects/<slug>/<session_id>.jsonl`, ~62 Mo / 20k lignes pour une grosse session) :
- Une ligne JSON par entrée, clé discriminante `type`. Types utiles : `assistant`, `user` ; types app-spécifiques à **ignorer** : `ai-title`, `last-prompt`, `mode`, `permission-mode`, `pr-link`, `queue-operation`, `file-history-delta/snapshot`, `frame-link`, `system`, `attachment`.
- Entrées `assistant`/`user` : `message` (le contenu), + `cwd`, `gitBranch`, `isSidechain`, `parentUuid`, `uuid`, `requestId`, `sessionId`, `timestamp`, `version`, parfois `toolUseResult`, `sourceToolAssistantUUID`, `attributionPlugin`, `attributionSkill`.
- `assistant.message` : `role`, `model`, `stop_reason`, `content` (blocs `type` ∈ {`thinking`, `text`, `tool_use`}), `usage` (`input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, …).
- **Sous-agents** : le tool de sous-agent s'appelle `Agent` (bloc `tool_use{name:"Agent"}`, input avec `subagent_type`) ; son résultat revient en `tool_result`/`toolUseResult`. Dans les sessions observées, `isSidechain` est **toujours false** et le détail interne des sous-agents vit dans des fichiers **séparés** `agent-<id>.jsonl` du même répertoire — **non consommés en P1** (on s'arrête au niveau parent : spawn + result).
- Autres tools observés : `Bash` (majoritaire), `Edit`, `Read`, `Write`, `AskUserQuestion`, `Skill`, etc.

**Hooks de plugin Claude Code** (source : agent doc `claude-code-guide`, **à confirmer à l'implémentation** — la sortie a été partiellement neutralisée par le harness ; traitée comme info) :
- Un plugin déclare ses hooks dans `hooks/hooks.json` (ou inline `plugin.json`). Manifeste `.claude-plugin/plugin.json`.
- `SessionStart` : déclenché à l'ouverture/reprise d'une session ; payload commun `session_id`, `transcript_path`, `cwd`. Variables d'env dispo : `CLAUDE_PLUGIN_ROOT`, `CLAUDE_PLUGIN_DATA`, `CLAUDE_PROJECT_DIR`.
- `async: true` = hook non bloquant. **Le stdout d'un hook est interprété par Claude Code** (ne jamais y écrire de données ; écrire dans un fichier). Exit 2 bloque l'action (à éviter → toujours exit 0).
- Installation : `claude plugin install <path|name@marketplace>` ; scopes user/project/local ; `enable/disable`.

## Architecture (P1)

```
[Claude Code session]
  │  hook SessionStart (async, exit 0)
  ▼
armadai-claude-plugin  ──append──►  index de sessions
  (.claude-plugin/plugin.json         ~/.config/armadai/claude-sessions.jsonl
   + hooks/hooks.json                  {session_id, transcript_path, cwd, started_at}
   + bin/on-session-start)
                                              │  lu par
                                              ▼
                          armadai watch  ──►  claude_adapter (bin)
                                              tail transcript_path (streaming)
                                              map entrées → RunEvent
                                              │
                                              ▼
                                     Workroom (consommateur RunEvent existant)
```

**Placement (cohérent OH7)** : `RunEvent`/`EventSink` restent **génériques dans `armadai-core`** (aucune connaissance de Claude Code). L'adaptateur transcript→RunEvent est **côté bin** : nouveau module `crates/armadai/src/claude_adapter/` (comme `es_log`/`db` sont des adaptateurs bin). Il n'introduit **aucune** dépendance Claude-Code dans le cœur.

## Composants

### 1. Le plugin minimal — `crates/armadai/assets/claude-plugin/`
Répertoire de plugin Claude Code, versionné dans le repo sous `crates/armadai/assets/claude-plugin/`, contenant :
- `.claude-plugin/plugin.json` : manifeste (`name: "armadai-workroom"`, version, description, `hooks: "./hooks/hooks.json"`).
- `hooks/hooks.json` : déclare **un** hook `SessionStart` (`async: true`, `type: "command"`), dont la commande **invoque le binaire `armadai` lui-même** : `command: "armadai __claude-register-session"` (sous-commande interne, voir §4). Pas de script shell ni de dépendance `jq` : la logique (lire le JSON stdin, extraire `session_id`/`transcript_path`/`cwd`, append l'index) vit **en Rust**, testable. Prérequis : `armadai` sur le `PATH` (acceptable — regarder une session Claude Code *via armadai* suppose armadai installé).

Contraintes du hook : idempotent (append), robuste (le binaire échoue silencieusement / exit 0 même en cas d'erreur d'écriture, pour ne jamais perturber Claude Code), aucune sortie stdout (interprétée par Claude Code).

### 2. L'index de sessions
Fichier JSONL append-only à un chemin connu (défaut `~/.config/armadai/claude-sessions.jsonl`, surchargable via `ARMADAI_SESSION_INDEX`). Une ligne par `SessionStart`. `armadai watch` le lit pour lister/résoudre les sessions (dédup par `session_id`, dernière entrée gagne).

### 3. L'adaptateur `claude_adapter` (bin)
Module `crates/armadai/src/claude_adapter/` :
- `session_index.rs` : lire/parser l'index (`Vec<SessionRef>`), résoudre `--last`/`--session <id>`.
- `transcript.rs` : lecteur **streaming** d'un transcript JSONL. API type : un itérateur/flux qui, ligne à ligne (sans charger le fichier), désérialise l'entrée en un `enum TranscriptEntry` défensif (variantes connues + `Other` ignoré), et un `mode replay` (lecture jusqu'à EOF) et `mode live` (tail : re-lecture des lignes ajoutées, ex. poll de la taille/offset).
- `mapper.rs` : machine à états qui consomme les `TranscriptEntry` et **émet des `armadai_core::RunEvent`** :
  - première entrée de la session → `RunStart { run_id: session_id, … }` (agent racine logique « claude »).
  - `assistant` avec `usage` → accumulation tokens/coût (le coût dérivé du modèle si dispo, sinon tokens bruts).
  - bloc `tool_use{name:"Agent"}` → `AgentStart { agent: <subagent_type> }` (+ `Delegate` du parent vers l'agent) ; le `tool_result` correspondant (corrélé par `tool_use_id`) → `AgentEnd { agent, tokens?, content: résumé }`.
  - autres `tool_use` → **non émis individuellement en P1** ; un compteur d'outils par agent peut être exposé via un champ agrégé (pas de nouvel event ; pas d'extension de `RunEvent`).
  - dernier texte assistant / `stop_reason` terminal → `Result { content }`.
  - erreurs de parsing d'une ligne → ligne ignorée + `tracing::debug` (jamais faire échouer le flux) ; `version` du transcript loggé une fois.
- **Aucune extension de `RunEvent` en P1** (les variantes existantes suffisent au niveau agent).

### 4. Commande CLI `armadai watch`
Nouvelle commande (dans `crates/armadai/src/cli/`, enregistrée dans `cli/mod.rs`) :
- `armadai watch` (ou `armadai watch claude`) : lit l'index ; sans argument → sélecteur interactif (dialoguer) des sessions récentes ; `--last` → la plus récente ; `--session <id>` → ciblée.
- Attache le **Workroom** (réutilise la vue existante consommant `RunEvent`) à la session choisie via `claude_adapter` en mode live (tail) ; si la session est terminée → replay jusqu'à EOF puis fin.
- `--json` (optionnel, cohérent avec `run`) : émet les `RunEvent` en JSONL au lieu du Workroom (réutilise `make_sink`).

**Sous-commande interne `armadai __claude-register-session`** (cachée, `hide` dans clap) : lit le payload JSON du hook sur **stdin**, en extrait `session_id`/`transcript_path`/`cwd`, append `{session_id, transcript_path, cwd, started_at}` dans l'index (`ARMADAI_SESSION_INDEX` ou défaut). Toujours exit 0 (erreur d'écriture → warn silencieux, jamais de panic/stderr bloquant). C'est la cible du hook `SessionStart` du plugin — logique Rust unique et testable (pas de shell/jq).

## Flux de données & états

`index → SessionRef → transcript (streaming) → TranscriptEntry* → mapper → RunEvent* → EventSink (Workroom | JSONL)`.

État du mapper : `run_id`, agent courant / pile de délégation (map `tool_use_id → agent` pour corréler spawn/result des sous-agents `Agent`), accumulateur tokens. Le `Result` final est émis à la fin du flux (EOF en replay, ou dernier `stop` terminal en live).

## Gestion d'erreurs

- Transcript introuvable/illisible (chemin de l'index périmé) → message clair, code retour non nul, pas de panic.
- Ligne JSON mal formée / type inconnu → ignorée (debug log), le flux continue.
- Dérive de format (clés manquantes, nouvelle `version`) → dégradation gracieuse (on émet ce qu'on peut) + warn une fois ; le design assume que le format peut changer (risque documenté).
- Hook (plugin) : toujours exit 0 ; en cas d'échec d'écriture de l'index, échoue silencieusement côté hook (ne perturbe jamais Claude Code).

## Tests

- **Fixtures** : petits transcripts JSONL synthétiques (dérivés d'un vrai, réduits) couvrant : session simple (assistant text → Result), session avec 1+ sous-agents `Agent` (spawn/result → AgentStart/End/Delegate), tokens (`usage`), entrées app-spécifiques à ignorer, ligne malformée (doit être sautée). Asserts sur la **séquence exacte de `RunEvent`** produite par le mapper.
- **Streaming** : test que le lecteur ne charge pas tout en mémoire (lecture ligne à ligne) et gère le mode tail (append détecté).
- **Index** : parse + résolution `--last`/`--session` + dédup.
- **Plugin** : le script `on-session-start` produit la bonne ligne d'index à partir d'un payload stdin d'exemple (test shell simple ou test d'intégration).
- Gate habituelle (workspace-wide) : fmt + clippy 3 combos + tests + `cargo build -p armadai-core` inchangé (l'adaptateur est côté bin, le cœur ne bouge pas).

## Hors périmètre (P1)

- Couche **hooks temps réel** (P2) : en P1, un seul hook `SessionStart` (enregistrement) ; le reste vient du transcript.
- **Intégration relais armadai** / auto-activation du plugin + retrait des marqueurs (P3).
- **Drill-down des sous-agents** (`agent-<id>.jsonl`) et affichage des **tool calls individuels** : différés (nécessiteraient possiblement une extension de `RunEvent` — hors P1).
- Coût monétaire précis par modèle si non trivial : P1 peut se limiter aux **tokens** (coût = enrichissement ultérieur via le catalogue de modèles).
- Publication marketplace du plugin.

## Risques

- **Format du transcript non documenté / instable** : principal risque. Atténué par un parser défensif (types inconnus ignorés, dégradation gracieuse, `version` tracée) et des fixtures qui pinnent le schéma observé ; une dérive cassante se verrait en tests et via le warn de version.
- **Faits hooks/plugin à confirmer** : la surface exacte de `plugin.json`/`hooks.json` et le payload `SessionStart` viennent d'une source doc secondaire → à valider sur un vrai `claude` à l'implémentation (P1 n'utilise QUE `SessionStart` + les env vars, surface minimale).
- **Sous-agents en fichiers séparés** : le niveau parent (spawn/result) suffit en P1 ; si l'affichage manque de profondeur, le drill-down est une extension propre (P2+).
