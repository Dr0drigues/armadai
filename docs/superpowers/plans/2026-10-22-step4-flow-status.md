# Étape 4 — « flow status » : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** enregistrer, par des hooks Claude Code asynchrones, le démarrage et la fin de chaque sous-agent, puis afficher avec `armadai flow status` où en est chaque flow déclaré.

**Architecture:** un module `crates/armadai/src/flow_status/` : `record.rs` (commande cachée `__record-flow-event`, écrit une ligne JSON par événement), `runs.rs` (fonction pure : événements + projet → exécutions de flow), `render.rs` (texte). `link` (adaptateur Claude) installe les hooks et ajoute la consigne `STATE:` aux agents à états (Claude et OpenCode).

**Tech Stack:** Rust 2024, serde_json, anstream (style CLI existant).

**Spec:** `docs/superpowers/specs/2026-10-22-step4-flow-status-design.md`

## Global Constraints

- MVP : Claude Code seulement pour l'enregistrement ; la consigne `STATE:` va aux agents des deux cibles fidèles (Claude, OpenCode).
- Hooks : `type: "command"`, `"async": true`, commande `armadai __record-flow-event` ; événements `SubagentStart`, `SubagentStop`, et `PreToolUse` avec le matcher `SubagentHandback`. Installés seulement si le projet a au moins un flow.
- `__record-flow-event` sort **toujours** avec le code 0 (entrée invalide, projet introuvable, écriture impossible : rien n'est écrit, rien n'est affiché sur stdout).
- Journal : `.armadai/state/claude/<session_id>.jsonl`, une ligne JSON par événement : `{"ts": <secondes unix>, "event": "start"|"stop", "agent_id": "...", "agent_type": "...", "state": "<mot>"|null}`. Jamais de texte de prompt ni de réponse. `session_id` invalide comme nom de fichier (`/`, `..`, vide) → rien écrit.
- `.armadai/.gitignore` gagne `state/`.
- État : la **dernière** ligne de la réponse de la forme exacte `STATE: <mot>` (mot = caractères d'id), sinon `null`.
- Consigne ajoutée au corps d'un agent qui tient au moins une étape à états : `\n\n## Verdict\n\nEnd your reply with exactly one line \`STATE: <state>\`, where <state> is one of: <états triés, séparés par ", ">.\n`.
- Affichage : `✓` fait, `●` en cours, `○` à venir, `?` incertain (avec la raison), `✗` état non prévu par un lien.
- Tests à égalité exacte ; une mutation vue rouge par test ajouté (restaurer puis `touch`) ; `crates/armadai` binaire seul (`--bin armadai` / `--test <nom>`), `--no-fail-fast`.
- Barrière : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)`.

## Review Focus

1. Le hook lancé depuis un worktree git ou un sous-dossier du projet : il écrit dans le `.armadai/` du projet (même résolution de racine que le gate de politique), test en tâche 1.
2. Deux sessions en parallèle : deux fichiers distincts, aucune ligne mélangée — test en tâche 1.
3. Un sous-agent interne (`agent_type` vide) : enregistré, ignoré à l'affichage — test en tâche 3.
4. Une boucle `build → guard → build → guard` : compteur `build ⇄ guard : 2` et une seule exécution — test en tâche 3.
5. `--watch` sur une session qui n'a encore aucun fichier : message clair puis attente, pas d'erreur — test de la fonction pure en tâche 4.

---

### Task 1 : enregistrement

**Files:**
- Create: `crates/armadai/src/flow_status/mod.rs`, `crates/armadai/src/flow_status/record.rs`
- Modify: `crates/armadai/src/cli/mod.rs` (commande cachée `__record-flow-event`, ajoutée à la liste des commandes cachées testée), `crates/armadai/src/claude_adapter/policy_gate.rs` (rendre la résolution de racine réutilisable, sans changer son comportement), `crates/armadai/src/main.rs` si la commande doit rester silencieuse comme le gate

**Interfaces:**
- Produces :
  - `pub struct FlowEvent { pub ts: u64, pub event: EventKind, pub agent_id: String, pub agent_type: String, pub state: Option<String> }` (`Serialize`, `Deserialize`), `pub enum EventKind { Start, Stop }` sérialisé `start`/`stop`.
  - `pub fn event_from_hook(raw: &str, now: u64) -> Option<(String /*session_id*/, PathBuf /*cwd*/, FlowEvent)>` : `SubagentStart` → `Start` ; `SubagentStop` → `Stop` avec `state` tiré de `last_assistant_message` ; `PreToolUse` sur `SubagentHandback` → `Stop` avec `state` tiré de `tool_input.message`, `agent_id`/`agent_type` de l'entrée (présents dans un sous-agent) ; tout le reste → `None`.
  - `pub fn state_of(text: &str) -> Option<String>`.
  - `pub fn record_from_stdin()` (toujours `Ok`, code 0).
  - `pub fn log_path(root: &Path, session_id: &str) -> Option<PathBuf>` (refuse un id de session non sûr).

- [ ] **Step 1 :** tests : `state_of` (dernière ligne `STATE: rejected` parmi plusieurs ; `STATE:rejected` sans espace → `None` ; `STATE: two words` → `None` ; absent → `None`) ; `event_from_hook` sur les trois exemples de la doc (JSON de la spec §3) avec la ligne exacte attendue, et `None` pour `Stop`/`SessionStart`/JSON invalide ; `log_path` refuse `../x`, `a/b`, `""` ; intégration : `__record-flow-event` lancé avec un `cwd` sous-dossier d'un projet écrit dans `.armadai/state/claude/<id>.jsonl` du projet (Review Focus 1), deux `session_id` → deux fichiers (Review Focus 2), entrée invalide → code 0, rien écrit, stdout vide ; le fichier ne contient ni le prompt ni la réponse (comparaison exacte de la ligne).
- [ ] **Step 2 :** rouge, implémenter (écriture en ajout, `create_dir_all`), vert ; barrière ; commit `feat(flow): record subagent events from Claude Code hooks`.

### Task 2 : `link` installe les hooks et la consigne `STATE:`

**Files:**
- Modify: `crates/armadai/src/compile/claude_settings.rs` (entrées de hooks, `native` portant `"async": true`), `crates/armadai/src/compile/claude.rs` et `crates/armadai/src/compile/opencode.rs` (consigne ajoutée au corps), `crates/armadai/src/cli/init.rs` ou l'écrivain de `.armadai/.gitignore` (ligne `state/`), étalons concernés
- Test: tests unitaires des adaptateurs, `crates/armadai/tests/`

**Interfaces:**
- Consumes : rien de la tâche 1 (seul le nom de commande `armadai __record-flow-event`).
- Produces : `pub fn flow_status_entries() -> Vec<HookEntry>` (3 entrées) ; `pub fn verdict_instruction(project: &Project, agent: &str) -> Option<String>` (dans `compile/mod.rs`, partagée par les deux adaptateurs).

- [ ] **Step 1 :** tests : projet avec un flow → `settings.json` contient exactement les 3 hooks (événement, matcher : aucun pour `SubagentStart`/`SubagentStop`, `SubagentHandback` pour `PreToolUse`, commande, `"async": true`) en plus du gate s'il est `strict` ; projet sans flow → aucun ; `unlink` les retire et laisse les hooks de l'utilisateur intacts ; un agent qui tient l'étape `guard` (`states: [approved, rejected]`) reçoit exactement la consigne des Global Constraints (Claude et OpenCode) ; un agent sans étape à états ne la reçoit pas ; agent dans deux flows → union des états, triés.
- [ ] **Step 2 :** si la doc Claude Code ne précise pas qu'un matcher absent capte tous les agents, utiliser le matcher absent et le noter pour l'essai réel (ne pas inventer de syntaxe).
- [ ] **Step 3 :** rouge, implémenter, régénérer et relire les étalons (`UPDATE_GOLDEN=1`), vert ; barrière ; commits `feat(link): install the flow recording hooks for Claude Code` et `feat(link): ask flow agents for a STATE line`.

### Task 3 : reconstruire les exécutions

**Files:**
- Create: `crates/armadai/src/flow_status/runs.rs`

**Interfaces:**
- Consumes : `FlowEvent` (tâche 1), `armadai_model::{Project, Flow, entry_steps}`.
- Produces :
  - `pub struct Run { pub flow: String, pub steps: Vec<StepView>, pub loops: Vec<(String, String, u32)>, pub transitions: Vec<(u64, String, String, Option<String>)>, pub finished: bool }`
  - `pub struct StepView { pub step: String, pub mark: Mark, pub state: Option<String>, pub note: Option<String> }`, `pub enum Mark { Done, Running, Pending, Unsure, Unexpected }`
  - `pub fn runs(project: &Project, events: &[FlowEvent]) -> Vec<Run>` selon la spec §4 (étape choisie par le lien autorisé depuis la dernière étape terminée ; exécution ouverte par une étape d'entrée ; terminée sur une étape sans lien sortant applicable ; état contrôlé contre les `states` de l'étape).

- [ ] **Step 1 :** tests (projet de test avec le flow `deliver` : design → build → guard(approved|rejected) → build si rejected, validate si approved) : séquence nominale → toutes `Done`, `finished` ; boucle deux fois → `loops == [("build","guard",2)]` hors aller simple, une seule exécution (Review Focus 4) ; étape en cours → `Running` ; `STATE:` absent sur `guard` → `Unsure` avec la note `no STATE line` ; état `scope_changed` non déclaré → `Unexpected` ; `agent_type` vide → ignoré (Review Focus 3) ; deux flows qui partagent un agent → étape choisie par le lien, sinon `Unsure` `ambiguous step`.
- [ ] **Step 2 :** rouge, implémenter, vert ; barrière ; commit `feat(flow): rebuild flow runs from recorded events`.

### Task 4 : `armadai flow status`

**Files:**
- Create: `crates/armadai/src/flow_status/render.rs`, `crates/armadai/src/cli/flow.rs`
- Modify: `crates/armadai/src/cli/mod.rs` (sous-commande `flow status [<flow>] [--session <id>] [--watch] [--list]`)

**Interfaces:**
- Consumes : `runs` (tâche 3), `log_path` (tâche 1).
- Produces : `pub fn render(runs: &[Run], session: &str) -> String` (texte sans couleur ; les couleurs sont ajoutées à l'affichage comme dans `list`) ; `pub fn latest_session(root: &Path) -> Option<String>` (fichier le plus récemment modifié).

- [ ] **Step 1 :** tests : `render` d'une exécution en cours avec une boucle, à égalité exacte du texte (en-tête `session <id>`, une ligne par étape avec sa marque et son état, la ligne `build ⇄ guard : 2`, les transitions horodatées en `HH:MM:SS` UTC) ; exécutions terminées gardées et listées avant l'exécution en cours ; `--list` liste les sessions (id, date, nombre d'exécutions) ; aucune session → message `no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)` et code 0 (Review Focus 5 : `--watch` attend au lieu d'échouer) ; `<flow>` inconnu → erreur avec suggestion.
- [ ] **Step 2 :** `--watch` : boucle qui relit le journal toutes les secondes et réaffiche seulement si la taille du fichier a changé ; la boucle elle-même n'a pas de test (fonction pure `should_redraw(old_len, new_len)` testée).
- [ ] **Step 3 :** rouge, implémenter, vert ; barrière ; commit `feat(flow): show where each flow stands`.

### Task 5 : documentation et essai réel

**Files:**
- Create: `docs/wiki/flow-status.md`
- Modify: `docs/wiki/link.md` (hooks installés, consigne `STATE:`), `CHANGELOG.md` (`## Unreleased`, `### Added`), `CLAUDE.md` (liste des commandes : `flow`)

- [ ] **Step 1 :** doc : ce qui est enregistré et ce qui ne l'est pas, où, comment purger (`rm -r .armadai/state/`), la consigne `STATE:`, les marques, les limites du MVP (Claude seulement, `?` en cas de doute, sous-agents en arrière-plan non garantis).
- [ ] **Step 2 :** barrière ; commit `docs: flow status`.
- [ ] **Step 3 (contrôleur) :** préparer un projet d'essai (flow à boucle) et donner les essais un par un à Dimitri : hooks présents, journal écrit pendant un flow réel, `flow status --watch` à jour, compteur de boucle, sous-agent en arrière-plan.
