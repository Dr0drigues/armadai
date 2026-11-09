# Lot interface dans les orchestrateurs : recherche et décisions (2026-11-04)

Recherche d'adapter-specialist (doc officielle lue, aucun essai réel), condensée par lead. V = lu dans la doc, C = à confirmer.

## Décisions de Dimitri
- Pistes retenues : « Les 4 me vont » (1 statusLine Claude Code, 2 panneau Claude Code, 3 OpenCode, 4 vue partagée). Principe : l'UI est une sortie compilée vers les mécanismes natifs, aucune UI propre à ArmadAI.
- Q1 code JS : « Déclaratif puis mod (Recommandé) » — d'abord statusLine et subagentStatusLine (plugin, déclaratif), le mod (JS, panneau) ensuite en opt-in dans project.yaml avec ligne de rapport.
- Q2 statusLine existante : « Opt-in + refus (Recommandé) » — écrite seulement si project.yaml la demande ; barre déjà présente → refus, ligne not-compiled, jamais de masquage silencieux.
- Q3 documents produits : « Parcours du disque (Recommandé) » — listés au moment de l'affichage, aucun nouvel événement enregistré, politique « rien d'autre n'est enregistré » inchangée.
- Q4 vue partagée : « graph + flow status (Recommandé) » — `armadai graph --format html|md` (structure), `armadai flow status --html` (état) ; noms finaux à cli-specialist.
- Q5 rendu : « Markdown puis HTML embarqué (Recommandé) » — Markdown + Mermaid d'abord ; HTML avec Mermaid embarqué (aucun réseau) plus tard.
- Q6 OpenCode : « Essai réel d'abord (Recommandé) » — plugin minimal qui journalise les événements, joué par Dimitri sur OpenCode 2.1.2, avant de spécifier la piste 3.

## Faits
### Socle
- `flow status` n'a pas de `--json` ; `runs()` (`flow_status/runs.rs`) donne Run/StepView/Mark. Un JSON stable est la brique commune des pistes 1, 2, 4.
- Les hooks n'enregistrent que timestamp, start/stop, id, agent, mot STATE (wiki flow-status.md « What is recorded »).

### Piste 1 statusLine (https://code.claude.com/docs/en/statusline)
- V : `{"statusLine":{"type":"command","command":"...","padding":2,"refreshInterval":N}}` ; stdin JSON avec `session_id`, `transcript_path`, `cwd`, `workspace.*` ; COLUMNS/LINES en env ; debounce 300 ms, script annulé si nouvelle mise à jour ; inactif → mettre refreshInterval ; une ligne par echo, ANSI, OSC 8 ; vide si exit≠0 ; vide tant que le dossier n'est pas approuvé.
- V : une seule statusLine, précédence managed > CLI > settings.local.json > settings.json > ~/.claude/settings.json ; celle du projet masque celle de l'utilisateur.
- V code : posable aujourd'hui par `native.claude.statusLine` (claude_settings::native_settings) ; `merge_settings` refuse en conflit si la clé existe avec une autre valeur.
- C : `session_id` de la statusLine = celui des hooks (probable, à mesurer).
- Branchement : `compile/claude.rs` (près de « project native »), sous-commande cachée qui lit stdin, trouve `.armadai/state/claude/<session_id>.jsonl` via `flow_status::record::log_path`, rend une ligne.

### Piste 2 panneau Claude Code
- V : un plugin ne peut PAS poser `statusLine` (son settings.json ne retient que `agent` et `subagentStatusLine`).
- V : `subagentStatusLine` (plugin) réécrit la ligne de chaque sous-agent ; reçoit `tasks[]` (name, agentType, description, tokenCount, cwd…) ; rend `{"id","content"}` avec ANSI/OSC 8. Seule surface déclarative livrable par plugin.
- V : les « mods » (https://code.claude.com/docs/en/plugins/mods/overview.md, interface.md, reference.md) dessinent un panneau (`$.ui.open`, Pane/AbovePrompt, Box/Text/Link/Markdown…), lisent des fichiers (`$.fs.read`), rafraîchissent (`$.clock.every`), peuvent lancer un process. JS non sandboxé, permissions de l'utilisateur. Activés par défaut à partir de Claude Code v2.1.287 ; coupés par `disableAllHooks`, `--safe-mode`, `allowManagedModsOnly`, extinction à distance possible. Pas en `claude -p`, cloud, WSL Desktop. `claude plugin test` pour tester. API qui bouge vite (types GitHub parfois en retard).
- V : Claude Code ne scanne pas `.claude/plugins/` ; installation projet par `extraKnownMarketplaces` + `enabledPlugins` dans `.claude/settings.json`, plugin en chemin relatif chargé en place ; appliqué seulement après le dialogue de confiance (sinon ignoré sans message). C : forme exacte d'une source `directory` relative (https://code.claude.com/docs/en/plugins/marketplace-reference.md).
- C : ouverture automatique d'un Pane ; `Link` `file://` en terminal.

### Piste 3 OpenCode 2.x (https://opencode.ai/v2/docs/plugins, /v2/docs/build/plugins, …/server, …/cli)
- V : plugins locaux `.opencode/plugins/` ; clé `plugins` d'opencode.json ; plugin = serveur + TUI ; `Plugin.define`, `ctx.tool.hook("execute.before"/"execute.after")`, `ctx.event.subscribe` ; TUI : toast, `ui.slot` (sidebar.content, session.panel, home.footer.status…), panneaux, `data.on`.
- C : liste des événements (sous-agents ?), accès fichier depuis le plugin, chargement auto de la partie TUI d'un plugin local (un plugin TUI se déclare dans `cli.json`, autre fichier partagé). Écart d'API v1/v2.
- Aujourd'hui `compile/opencode.rs` : `flow recording: not-compiled` (RECORDING_CLAUDE_ONLY), automation « needs an OpenCode plugin ».

### Piste 4 vue partagée
- V : `armadai graph` sort du Mermaid seul (`flowchart TD`, `%% flow: <id>`), sans état d'exécution ; un seul format.
- Markdown + Mermaid : rendu GitHub/GitLab/VS Code ; page statique = instantané ; `link` ne touche pas le réseau.

## Risques
1. Mod = JS exécuté avec les permissions de l'utilisateur : opt-in, le dire.
2. API des mods et d'OpenCode v2 instables.
3. Dialogue de confiance : sans lui, silence total (barre vide).
4. statusLine de projet masque celle de l'utilisateur (décidé : opt-in + refus).
5. session_id statusLine/hooks non mesuré.
6. OpenCode : enregistrement non démontré.
7. Golden à mettre à jour (`tests/golden/`).

## Ordre recommandé (adapter-specialist)
1. Socle : `flow status --json` + résumé en une ligne. 2. statusLine. 3. Vue Markdown (en parallèle). 4. subagentStatusLine par plugin, puis mod opt-in. 5. OpenCode après l'essai réel.
