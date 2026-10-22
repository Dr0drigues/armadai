# Étape 4 : « flow status », voir où en est un flow (Claude Code d'abord)

## 1. Intention

ArmadAI compile une équipe et ses flows, mais ne les exécute pas : une fois le travail lancé dans Claude Code, l'utilisateur ne voit pas à quelle étape en est le flow, quel verdict chaque vérificateur a rendu, ni combien de fois une boucle (`guard → build`, `validate → refine`) a tourné. Dimitri veut retrouver la lisibilité de l'ancienne TUI, sans revenir à un moteur. La simulation du pack « concept-to-code » a montré que ce suivi est le plus utile pendant les essais réels de B2.

Principe : l'outil raconte, ArmadAI affiche. `link` installe des hooks d'enregistrement qui écrivent une ligne par événement ; `armadai flow status` lit ces lignes et les rapporte au flow déclaré.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs (MVP) :
1. Sous la cible `claude`, quand le projet déclare au moins un flow, `link` installe dans `.claude/settings.json` des hooks **asynchrones** (`"async": true`, `type: "command"`) : `SubagentStart` et `SubagentStop` (tous types d'agent), qui appellent `armadai __record-flow-event`. `unlink` les retire (mêmes mécanismes de clés possédées que le gate de politique).
2. `armadai __record-flow-event` lit le JSON d'entrée et ajoute une ligne à `.armadai/state/claude/<session_id>.jsonl` : horodatage, événement (`start` / `stop`), `agent_id`, `agent_type`, et pour `stop` seulement l'**état** extrait de la réponse finale (voir 4). Aucun texte de prompt ni de réponse n'est enregistré. Le dossier `.armadai/state/` est ignoré par git (`.armadai/.gitignore`).
3. Verdict à format fixe : pour toute étape de flow qui a des `states`, `link` ajoute au prompt de l'agent (Claude et OpenCode) la consigne de terminer par une ligne `STATE: <état>` avec la liste des états de ses étapes. C'est ce qui rend l'état lisible (point relevé par la simulation).
4. `armadai flow status [<flow>] [--session <id>] [--watch]` : sans `--session`, la session la plus récente. Affiche pour chaque exécution du flow :
   - chaque étape : `✓ fait (état)`, `● en cours`, `○ à venir`, `✗` si l'étape a rendu un état qu'elle ne déclare pas (ruling du 2026-10-22 : un état déclaré sans lien applicable termine normalement) ;
   - le compteur de chaque lien emprunté plus d'une fois (`build ⇄ guard : 2`) ;
   - la liste horodatée des transitions.
   `--watch` réaffiche à chaque nouvelle ligne du journal (lecture périodique, une seconde), jusqu'à Ctrl-C. Texte simple, couleurs du style de la CLI, aucune dépendance TUI.
5. Idées reprises du visualiseur `omo-herdr-dag` (MIT, idées seulement) : les exécutions terminées restent visibles ; l'étape en cours est mise en avant ; `--session` pour passer d'une exécution à l'autre ; `flow status --list` liste les sessions enregistrées.
6. Doc (`docs/wiki/flow-status.md`, `link.md`), CHANGELOG, `armadai graph` inchangé.

Non-objectifs du MVP (reportés, pas refusés) :
- OpenCode (passerait par un plugin `.opencode/plugin/` à vérifier), Codex, Copilot.
- Un affichage graphique (panneau, HTML) ; seulement le terminal.
- Rejouer, reprendre ou piloter un flow : ArmadAI ne commande rien.
- Purge automatique des journaux (`rm -r .armadai/state/` suffit ; documenté).

Anti-objectifs :
- Un hook qui ralentit ou bloque la session : hooks asynchrones, `__record-flow-event` sort toujours avec le code 0 (même sur entrée invalide), jamais 2 (sur `SubagentStop`, 2 empêcherait le sous-agent de s'arrêter).
- Enregistrer des prompts, des réponses ou des secrets.
- Un affichage qui affirme une étape que le journal ne prouve pas : en cas de doute, l'étape est marquée `?` avec la raison.

## 3. Contraintes (doc officielle Claude Code, hooks, relevée le 2026-10-22)

- `SubagentStart` reçoit `agent_id`, `agent_type` ; `SubagentStop` reçoit en plus `last_assistant_message` (« the text content of the subagent's final response, so hooks can access it without parsing the transcript file ») et `agent_transcript_path`. Champs communs : `session_id`, `transcript_path`, `cwd`, `hook_event_name`.
- `SubagentStop` se déclenche aussi pour des agents internes (`agent_type` vide ou inattendu) : ignorés à l'affichage (pas à l'enregistrement).
- Depuis la v2.1.271, un sous-agent qui rend son rapport par l'outil `SubagentHandback` : `last_assistant_message` n'est alors que le texte de clôture ; le rapport est `tool_input.message` d'un `PreToolUse` sur `SubagentHandback`. Le MVP enregistre aussi ce `PreToolUse` (async) pour en extraire l'état.
- Hooks `async: true` : pas de délai imposé, ne peuvent rien bloquer, reçoivent le même JSON.
- Non documenté, à vérifier à l'essai réel : `SubagentStop` pour un sous-agent lancé en arrière-plan ; la sémantique d'un matcher vide (on écrit `"*"` si la doc le permet, sinon un matcher par événement sans filtre).

## 4. Rapprocher les événements du flow (règles du MVP)

- Étape d'un événement : l'agent (`agent_type`) désigne les étapes du flow qu'il tient ; s'il en tient plusieurs, on choisit celle qu'un lien autorise depuis la dernière étape terminée (même relation que le gate), sinon `?`.
- Une exécution commence quand une étape d'entrée démarre alors qu'aucune exécution du même flow n'est en cours dans la session ; elle se termine quand une étape sans lien sortant applicable est terminée.
- État d'une étape terminée : dernière ligne de la réponse qui correspond à `STATE: <état>` avec un état déclaré pour cette étape ; sinon, si l'étape n'a pas d'états, `done` ; sinon `?` (« no STATE line »).

## 5. Laissé au constructeur

Le format exact d'une ligne de journal (JSON, champs ci-dessus), la présentation du texte, l'ordre des sous-commandes, la façon d'installer les hooks dans le `settings.json` partagé (réutiliser le mécanisme du gate).

## 6. Critères d'acceptation

1. Barrière complète verte ; une mutation vue rouge par test ajouté.
2. Tests à égalité exacte : enregistrement (entrée JSON → ligne écrite, aucun texte de prompt/réponse), extraction de l'état, rapprochement sur une séquence d'événements qui fait deux fois la boucle `build → guard → build`, affichage complet.
3. `link`/`unlink` installent et retirent exactement les hooks ; étalons Claude mis à jour (consigne `STATE:` dans les agents à états).
4. Essai réel par Dimitri dans Claude Code : un flow à boucle lancé, `armadai flow status --watch` montre l'étape en cours, les états et le compteur de boucle.

## 7. Risques

- Un modèle qui oublie la ligne `STATE:` : l'étape apparaît `?` ; le linter d'agents (plus tard) pourra le signaler.
- Un agent qui tient plusieurs étapes ambiguës : `?` plutôt qu'une fausse certitude.
- Les journaux grossissent : une ligne par sous-agent, petit ; purge manuelle documentée.
