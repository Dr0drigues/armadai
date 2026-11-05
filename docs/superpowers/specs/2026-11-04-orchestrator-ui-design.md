# Interface dans les orchestrateurs

Document de recherche associé (décisions de Dimitri mot pour mot, faits V/C) : `docs/superpowers/specs/2026-11-04-orchestrator-ui-research.md`. Légende : **V** = lu dans la doc officielle, **C** = à confirmer ; tout comportement C devient un critère mesuré par essai.

## 1. Intention

Aujourd'hui l'avancement d'un flow ne se lit que dans un second terminal (`armadai flow status --watch`) : il n'y a rien dans Claude Code ni dans OpenCode, et rien à partager hors terminal. Dimitri a validé le 2026-11-04 : quatre pistes (statusLine Claude Code, panneau Claude Code, OpenCode, vue partagée) sous le principe que **l'interface est une sortie compilée vers les mécanismes natifs de l'outil, jamais une UI propre à ArmadAI** (cohérent avec le refocus : spec `2026-10-09-step0-retrait-surfaces-design.md`). Le lot se livre en tranches indépendantes autour d'un socle commun. La piste 3 (OpenCode) attend un essai réel et n'est pas spécifiée ici.

## 2. Objectifs, non-objectifs, anti-objectifs

### Objectifs
1. **Socle** : `flow status --json` stable et un résumé d'état en une ligne, partagés par toutes les pistes.
2. **Piste 1** : une `statusLine` Claude Code qui affiche ce résumé, posée par `link` seulement sur demande dans `project.yaml`.
3. **Piste 2** : une `subagentStatusLine` livrée par un plugin (déclaratif), puis, en opt-in, un mod (JS) qui dessine un panneau ; les documents produits y sont listés par parcours du disque.
4. **Piste 4** : `armadai graph` en Markdown + Mermaid, et `flow status` sous forme lisible hors terminal ; plus tard une page HTML avec Mermaid embarqué.

### Non-objectifs
- Piste 3 (OpenCode) : aucun critère avant l'essai réel (section 7).
- Aucun nouvel événement enregistré : les hooks et le journal `.armadai/state/claude/*.jsonl` restent ce qu'ils sont (timestamp, start/stop, id, agent, mot `STATE`, nom de commande). Politique « rien d'autre n'est enregistré » inchangée.
- Aucun serveur, aucune UI propre, aucun accès réseau par `link`.
- Le résumé d'activité par outil (lectures, éditions, questions) est le lot suivant du watch animé, pas celui-ci.
- Codex et Copilot : rien (`not-compiled` comme aujourd'hui).
- Le HTML avec Mermaid embarqué n'est pas dans la première tranche (décision Q5).

### Anti-objectifs
- Masquer silencieusement la barre d'état de l'utilisateur (la `statusLine` de projet masque celle de l'utilisateur : V).
- Exécuter du JS non demandé : un mod n'est jamais posé sans opt-in explicite et ligne de rapport.
- Afficher un état que le journal ne porte pas (règle en vigueur : `?` plutôt qu'une supposition).
- Une barre qui ralentit Claude Code : la commande est relancée à chaque mise à jour (debounce 300 ms, V).

## 3. Contraintes

- `armadai-model` reste pur ; le JSON et le résumé sont calculés à partir de `runs()` (`flow_status/runs.rs`, binaire) ; `cargo build -p armadai-core` / `-p armadai-model` passent.
- Écritures de fichiers partagés par `compile/patch.rs` ; `link` garde sa politique de refus en conflit (`merge_settings`) ; `unlink` retire ce que `link` a écrit (manifeste, `version: 1`, champs optionnels seulement).
- Le texte de toute ligne nouvelle de rapport est proposé par le plan et validé par Dimitri ; codes de diagnostic kebab-case, table de `docs/wiki/project-format.md` à jour ; golden de `tests/golden/` relus ligne à ligne.
- Aucun contenu privé, aucun chemin de machine dans fixtures, docs, wiki. Aucun test sur le réseau. Mutation vue rouge pour chaque test ajouté. Essais avec `HOME` et `ARMADAI_CONFIG_DIR` isolés.
- Claude Code seul pour les pistes 1 et 2. `armadai link` ne touche pas le réseau.
- Un plugin Claude Code ne peut pas poser `statusLine` (V) : seulement `subagentStatusLine`.

## 4. Conception

### 4.1 Socle
- `flow status --json` : un document par session (id de session, runs : flow, état `in progress|finished|interrupted`, étapes avec marque et état, boucles, transitions horodatées UTC). Champ `schema` versionné ; ajout de champs permis, retrait ou renommage interdits sans nouvelle version. Sortie vide mais valide quand il n'y a pas de journal.
- Résumé en une ligne : fonction pure de `Vec<Run>` (nom de flow, étape en cours, avancement, `loops`), sans ANSI par défaut. Exposé par `flow status --line` (nom provisoire, fixé au plan). Décision de Dimitri (2026-11-05, Q1 « Les deux (Recommandé) ») : `--line` est le socle commun, que l'utilisateur ajoute dans sa propre barre (widget Custom Command de ccstatusline) ; il lit un JSON éventuel sur stdin : avec un `session_id`, il lit le journal de cette session ; sans `session_id` (stdin vide, absent, ou JSON sans la clé, cas du widget dont seul `terminal_width` est documenté), il se rabat sur le dernier journal modifié du projet (projet trouvé par `cwd`). Il ne bloque jamais sur un stdin interactif.
- Articulation avec le lot « watch animé » (en pause, `armadai-watch`, non commité) : les deux lisent `runs()` et ne modifient pas sa sémantique ; le watch ajoute une vue plein écran de `--watch`, ce lot un `--json`/`--line` et des formats de `flow status`. Recoupements : (a) le résumé en une ligne et le glyphe/préfixe d'état de la vue animée doivent partager la même table de marques (✓ ● ○ ? ✗ ~ -) pour ne pas diverger ; (b) les deux touchent `cli/flow.rs` et `flow_status/render.rs` ; le critère « `flow status` sans `--watch` inchangé » du watch devient, après ce lot, « inchangé hors des nouveaux drapeaux » ; (c) le watch ajoute `GraphFormat::Ascii` (branche `feat/graph-improvements`), ce lot `GraphFormat::Md` : même enum, conflit de rebase prévisible, pas de conflit de sens ; (d) le résumé d'activité suivant du watch ajoute un événement `tool` au journal : `--json` doit alors l'accueillir par un champ ajouté, d'où la règle « ajout permis » ci-dessus. Décision de Dimitri (2026-11-05, Q3 « Ce lot d'abord (Recommandé) ») : au moins L1 passe avant le lot watch animé, qui reprend ensuite le JSON et la table de marques ; son critère « sans `--watch` inchangé » s'ajuste au moment de sa reprise (spec du watch à affiner alors, pas avant).

### 4.2 Piste 1 : statusLine
- Deux usages, un socle (`--line`). (i) Pour qui a déjà une barre (cas de Dimitri : ccstatusline) : il ajoute `armadai flow status --line` comme widget Custom Command de sa barre ; `link` n'écrit rien. La doc (`docs/wiki/flow-status.md`) décrit ce branchement et ses limites (timeout et cache du widget). (ii) Pour qui n'en a pas : la `statusLine` de projet ci-dessous, opt-in.
- Opt-in par une **clé dédiée, validée, dans `project.yaml`** (décision de Dimitri, 2026-11-05, Q2 « Clé dédiée (Recommandé) » ; nom et forme fixés au plan, avec code de diagnostic et schéma) que `link` compile vers `native.claude.statusLine`. `link --target claude` écrit `{"statusLine":{"type":"command","command":"armadai __claude-statusline","refreshInterval":N}}` dans `.claude/settings.json`.
- La sous-commande cachée lit le JSON de stdin (`session_id`, `cwd`, V), trouve `.armadai/state/claude/<session_id>.jsonl` (`flow_status::record::log_path`), imprime une ligne ; sortie vide si pas de journal ; jamais d'échec bruyant. **C** : `session_id` de la statusLine = celui des hooks.
- Si une `statusLine` existe déjà (dans le fichier du projet, ou portée par une autre valeur de la clé), `link` n'écrit rien, rapporte `not-compiled` avec la raison. Les barres de niveau utilisateur ne sont pas lisibles par `link` : voir risque 2 et question 1.
- `unlink` retire la clé si c'est `link` qui l'a écrite.

### 4.3 Piste 2 : panneau Claude Code
- Étape A (déclaratif) : un plugin avec `settings.json` portant `subagentStatusLine`, commande cachée qui reçoit `tasks[]` et rend `{"id","content"}` (V). Installation projet par `extraKnownMarketplaces` + `enabledPlugins` dans `.claude/settings.json`, plugin chargé en chemin relatif ; actif seulement après le dialogue de confiance (V). **C** : forme exacte d'une source `directory` relative.
- Étape B (mod, opt-in) : un mod JS qui ouvre un panneau (`$.ui.open`), relit le journal ou `flow status --json`, rafraîchit (`$.clock.every`), liste les documents produits en parcourant le disque (dossiers `docs/` du projet, forme fixée au plan) ; ligne de rapport qui dit explicitement « JS exécuté avec vos permissions ». **C** : ouverture automatique d'un panneau ; liens `file://` cliquables.
- Les mods n'existent qu'à partir de Claude Code v2.1.287, sont coupés par `disableAllHooks`/`--safe-mode`, absents en `claude -p` : la ligne de rapport est `approximated` ou `not-compiled` selon le cas, jamais `compiled` sans que le mod soit réellement chargé.

### 4.4 Piste 4 : vue partagée
- `armadai graph --format md` : par flow, un titre et un bloc Mermaid (même contenu que `--format mermaid`), sur stdout.
- `flow status` en Markdown (drapeau fixé au plan) : les marques en texte, les transitions en tableau, lisible dans GitHub/GitLab/VS Code ; c'est un instantané daté, pas un suivi vivant.
- Plus tard : HTML avec Mermaid embarqué, aucun réseau (tranche ultérieure, non spécifiée).

## 5. Critères d'acceptation

Par livraison (chacune sa PR). Environnement commun : projet à flows, journal écrit à la main sous `.armadai/state/claude/<session>.jsonl`, `HOME` et `ARMADAI_CONFIG_DIR` isolés.

### Livraison L1 : socle
1. `armadai flow status --json` sur un journal à un run `in progress` imprime un JSON valide (`jq .` rend 0) qui porte le run, ses étapes avec marque et état, et un champ `schema`.
2. Sur un journal à trois runs (`in progress`, `finished`, `interrupted`) et un flow à cycle, le JSON liste les trois runs avec leur état exact et les boucles comptées.
3. Sans journal pour la session : `--json` imprime un JSON valide à zéro run, code retour 0.
4. `armadai flow status --line` imprime une seule ligne (un `\n`), sans séquence d'échappement, qui nomme le flow, l'étape en cours et l'avancement ; sur un journal sans run : une ligne vide ou fixe documentée, code retour 0.
4b. `echo '{"terminal_width":120}' | armadai flow status --line` (JSON sans `session_id`) et `armadai flow status --line </dev/null` impriment la ligne du dernier journal modifié du projet ; avec `{"session_id":"S"}` la ligne est celle de la session `S` même si un autre journal est plus récent ; dans les deux cas, code 0, une seule ligne, et la commande rend la main sans stdin interactif (mesuré avec une limite de temps).
5. Sans les nouveaux drapeaux, `flow status` donne la sortie d'avant (golden ligne à ligne) ; `link` produit les mêmes fichiers et le même rapport.

### Livraison L2 : statusLine
5b. La clé dédiée de `project.yaml` est validée : une valeur invalide donne un diagnostic positionné à code stable et `armadai validate` sort avec le code d'erreur habituel ; le schéma JSON la contient (régénéré) ; la table de `docs/wiki/project-format.md` liste le code.
6. Sans demande dans `project.yaml`, `armadai link --target claude` n'écrit aucune clé `statusLine` (diff de `.claude/settings.json` vide sur ce point).
7. Avec la demande, `link` écrit la `statusLine` et le rapport a une ligne `compiled` pour elle ; `unlink` la retire et laisse les clés de l'utilisateur.
8. Avec une `statusLine` déjà dans `.claude/settings.json`, `link` ne la modifie pas, sort avec le code habituel des refus de fusion, et le rapport a une ligne `not-compiled` nommant la raison.
9. `echo '{"session_id":"S","cwd":"..."}' | armadai __claude-statusline` imprime la ligne du critère 4 pour le journal de la session `S`, et rien (code 0) pour une session inconnue ou un JSON invalide.
10. Essai réel avec Dimitri : dans Claude Code, après le dialogue de confiance, la barre affiche l'avancement pendant un flow, **mesure C** : l'`id` de session reçu par la barre est celui du nom de fichier du journal écrit par les hooks ; mesure du coût d'une mise à jour (temps de la commande) ; constat de ce qui arrive à la barre `ccstatusline` de Dimitri (masquée par la barre du projet, comme la doc l'annonce).

10b. Essai réel avec Dimitri, widget ccstatusline (voie (i), sans clé dans `project.yaml`) : il ajoute `armadai flow status --line` comme widget Custom Command de sa barre. Mesures : (1) la commande telle qu'elle apparaît dans la config ccstatusline et la ligne affichée dans la barre pendant un flow ; (2) la sortie quand le widget n'envoie pas de `session_id` : la ligne du dernier journal du projet, ou rien (sans erreur ni texte parasite dans la barre) hors projet ou sans journal ; (3) le cache : avec `customCommandCacheTtlSeconds` réglé, la barre ne montre l'avancement qu'avec ce retard (valeur mesurée, à reporter dans la doc), et un TTL de 0 ou absent suit chaque mise à jour ; (4) le timeout du widget n'est pas atteint (temps de la commande mesuré) ; (5) **C à lever** : ANSI et sortie multi-ligne dans ce widget (la page GitHub ne le dit pas), pour décider si `--line` reste sans couleur.

### Livraison L3 : vue Markdown (parallèle à L2)
11. `armadai graph --format md` sort, pour chaque flow, un titre et un bloc ```` ```mermaid ```` dont le contenu est identique à `--format mermaid` du même flow ; un flow inconnu garde l'erreur actuelle ; `--format mermaid` reste identique octet pour octet.
12. `flow status` en Markdown sort un document dont chaque étape porte sa marque et son état, et un tableau des transitions horodatées UTC ; il contient zéro séquence d'échappement.
13. Essai réel : le Markdown de `graph` s'affiche comme diagramme sur la page GitHub/VS Code de Dimitri.

### Livraison L4 : subagentStatusLine par plugin
14. Avec la demande, `link` écrit le plugin et les clés `extraKnownMarketplaces` / `enabledPlugins` ; sans la demande, rien ; `unlink` retire ce qu'il a écrit.
15. `echo '{"tasks":[...]}' | armadai __claude-subagent-statusline` rend un `{"id","content"}` par tâche, sans panique sur une entrée vide ou invalide.
16. Essai réel : **mesures C** : la source `directory` relative est acceptée par Claude Code ; la ligne du sous-agent est réécrite pendant un flow ; sans dialogue de confiance, rien n'apparaît et rien ne casse.

### Livraison L5 : mod (opt-in)
17. Sans demande explicite de mod dans `project.yaml`, aucun fichier JS n'est écrit ; avec elle, `link` écrit le mod et la ligne de rapport nomme l'exécution de JS avec les permissions de l'utilisateur.
18. Le mod passe `claude plugin test` (V : la commande existe).
19. Essai réel : **mesures C** : le panneau s'ouvre (seul ou sur commande), se rafraîchit pendant un flow, les documents produits sont listés ; sous `--safe-mode` ou `disableAllHooks`, rien ne s'affiche et la ligne de rapport n'affirme pas le contraire.

Chaque test ajouté a sa mutation vue rouge ; la barrière complète est verte à chaque livraison.

## 6. Risques

1. **API des mods et d'OpenCode v2 instables** : se voit aux essais réels et à `claude plugin test` ; d'où le mod en dernier et en opt-in.
2. **Barre de l'utilisateur masquée** : `link` ne lit que `.claude/settings.json` du projet ; la barre `ccstatusline` de Dimitri est au niveau utilisateur, `link` ne peut pas la refuser. Mitigation (décision Q1) : la voie principale pour Dimitri est `--line` dans sa propre barre, qui ne masque rien ; la `statusLine` de projet reste opt-in pour qui n'a pas de barre ; doc et critères 10 et 10b.
3. **Dialogue de confiance** : sans lui, barre vide et plugin ignoré sans message ; documenté, critères 10 et 16.
4. **`session_id` statusLine/hooks non mesuré (C)** : si différent, la barre ne trouve pas son journal ; repli à définir au plan (dernier journal modifié du projet), décidé après la mesure.
5. **Coût d'un processus par mise à jour** : mesuré au critère 10.
6. **Dérive du JSON** : un consommateur externe s'y attache ; `schema` + règle d'ajout seul.
7. **Golden** de `link` à mettre à jour et à relire ligne à ligne.
8. **Conflit avec le lot watch** (4.1) : `cli/flow.rs`, `render.rs`, enum `GraphFormat`.
9. **ccstatusline** : la page GitHub (lue le 2026-11-05) mentionne un widget « Custom Command » qui reçoit un JSON en stdin (`terminal_width` seul nommé), avec timeout et cache optionnel jusqu'à 60 s ; elle ne documente ni `customCommandCacheTtlSeconds`, ni sortie multi-ligne, ni ANSI de ce widget : **C**, à mesurer au critère 10b (la voie « commande dans votre barre » est retenue). Risque associé : le cache du widget retarde l'affichage, et un widget sans `session_id` oblige au repli « dernier journal » (deux sessions en parallèle sur un projet : la ligne peut montrer l'autre ; dit dans la doc).

## 7. Piste 3 : à compléter après l'essai

Dimitri : « Essai réel d'abord ». Aucun critère ici. L'essai (plugin minimal qui journalise, OpenCode 2.1.2) doit lever trois inconnues :
1. **Événements** : lesquels reçoit `ctx.event.subscribe` / `execute.before|after`, et les sous-agents y apparaissent-ils (sinon pas d'enregistrement de flow sous OpenCode).
2. **Accès fichier** : le plugin peut-il lire/écrire `.armadai/state/` ou le projet.
3. **Chargement TUI** : la partie TUI d'un plugin local de `.opencode/plugins/` se charge-t-elle seule, ou faut-il la déclarer dans `cli.json` (fichier partagé avec l'utilisateur).
Aujourd'hui : `flow recording: not-compiled` pour OpenCode, inchangé tant que la piste n'est pas spécifiée.

## 8. Ordre des livraisons
L1 socle, puis L2 (statusLine) et L3 (Markdown) en parallèle, puis L4, puis L5 ; piste 3 après l'essai. Le HTML embarqué est une tranche ultérieure. Chaque livraison est une PR séparée ; le lot peut être coupé après L1/L2/L3 sans perdre de cohérence.

## Décisions de Dimitri (2026-11-05, mot pour mot)
- Q1 : « Les deux (Recommandé) ».
- Q2 : « Clé dédiée (Recommandé) ».
- Q3 : « Ce lot d'abord (Recommandé) ».

## Questions ouvertes
Aucune pour l'instant. À fixer au plan (texte validé par Dimitri) : noms des drapeaux et de la clé de `project.yaml`, dossiers parcourus pour les documents produits (L5), repli exact de `--line` entre plusieurs sessions.
