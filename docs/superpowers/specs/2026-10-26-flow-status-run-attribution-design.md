# flow status : attribuer une exécution au flow lancé

Issue publique : #527. Spec précédente du sujet : `docs/superpowers/specs/2026-10-22-step4-flow-status-design.md`.

## 1. Intention

`armadai flow status` déduit le flow d'une exécution de la séquence des sous-agents vus (`SubagentStart`/`SubagentStop`), jamais de ce que l'utilisateur a lancé. Sur une vraie session Claude Code (essai réel du 2026-10-26, deux flows partageant leur étape d'entrée, un agent tenant `ui` dans l'un et `fix-ui` dans l'autre), la commande affiche le mauvais flow, puis reste figée (`test`, `review`, `validate` en `pending` alors que leurs agents ont tourné) quand un sous-agent en arrière-plan finit après le démarrage du suivant. Un appel isolé hors flow ouvre une exécution fantôme, et une session mêle plusieurs exécutions sans rapport. Dimitri a choisi de traiter ce défaut, le principal de l'essai (2026-10-26, « Enchaine ») et veut le MVP d'abord.

## 2. Objectifs, non-objectifs, anti-objectifs

**Objectifs (MVP)**
1. Enregistrer la commande que l'utilisateur tape, comme frontière d'exécution : le nom de la commande, rien d'autre.
2. Quand la commande désigne un flow, l'exécution est attribuée à ce flow, sans inférence ni groupe `a | b`.
3. Dans une exécution ainsi attribuée, un chevauchement de sous-agents (un démarre avant que l'autre ait fini, arrière-plan) ne perd aucune étape : plus de `pending` pour un agent qui a tourné, plus d'exécution figée `in progress`.
4. Une nouvelle commande de flow ferme l'exécution précédente, affichée comme interrompue et non plus `in progress`.
5. Sans commande enregistrée (anciens journaux, flow lancé en langage naturel), la sortie reste celle d'aujourd'hui, au caractère près.

**Non-objectifs (pourront attendre, voir §4.3)** : séparer l'appel isolé d'un agent du flow *quand cet agent est une étape du flow attribué* ; `/clear` ; corriger le chevauchement sans commande enregistrée ; enregistrer le prompt ou les arguments ; Codex, OpenCode, Copilot (flow status reste Claude Code seul) ; un champ déclaré sur la commande (par exemple `flow:` dans `CommandFront`) : report explicite à un lot suivant, si le besoin apparaît (Dimitri, 2026-10-26 : « Nom d'abord, lien ensuite »).

**Anti-objectifs** : ne jamais écrire le texte du prompt ni les arguments de la commande dans le journal (la doc dit « no prompt, no reply text ») ; ne jamais deviner un flow quand le journal ne le dit pas (`?` plutôt qu'une supposition, règle déjà en vigueur) ; ne jamais faire échouer ni bavarder le hook (il ne sort rien, code 0 toujours).

## 3. Contraintes

- Événements documentés seulement (https://code.claude.com/docs/en/hooks.md, lu le 2026-10-26) :
  - `UserPromptExpansion` : « Runs when a user-typed command expands into a prompt before reaching Claude » ; il reçoit `expansion_type` (`slash_command` pour skill et commande), `command_name`, `command_args`, `command_source` et `prompt`. C'est l'événement retenu : il donne le nom de la commande déjà analysé.
  - `UserPromptSubmit` reçoit `prompt` (la ligne telle que tapée, `/deploy staging`), mais « also fires for scheduled tasks, background subagent reports, and messages from other sessions » : trop bruyant comme frontière, écarté.
  - `SessionStart` : `source` vaut `startup`, `resume`, `clear`, `compact` ou `fork`. Utile pour `/clear` plus tard ; non vérifié : si `session_id` change après `/clear` (cela décide si le journal est déjà séparé).
  - `SubagentStart` ne porte que `agent_id` et `agent_type` ; `PreToolUse` porte `tool_use_id` mais rien ne le relie au `agent_id` du sous-agent dans la doc lue : pas de corrélation par identifiant d'appel.
  - Champs communs `session_id`, `prompt_id` : la doc lue ne dit pas si `prompt_id` est partagé avec les hooks de sous-agent. Non vérifié, pas utilisé.
- Non vérifié : la version de Claude Code qui introduit `UserPromptExpansion`. Sur un Claude Code qui ne l'a pas, le hook ne se déclenche jamais et l'objectif 5 s'applique (dégradation à la sortie actuelle).
- Vérifié par `lead` : `CommandFront` (`crates/armadai-model/src/types.rs:249-256`) n'a que `description`, `argument_hint`, `native` : aucun lien commande -> flow dans le modèle. Décision de Dimitri (2026-10-26) : pour le MVP, le nom de la commande est l'identifiant du flow (`/bugfix` -> flow `bugfix`).
- Vérifié par `lead` dans la doc des hooks : pour `UserPromptExpansion`, la sortie standard du hook est ajoutée au contexte du modèle (l. 814). Le hook d'enregistrement ne doit donc rien écrire sur sa sortie standard (ni sur l'erreur standard), sinon il pollue le contexte de Claude.
- Lecture du dépôt faite : `record.rs`, `runs.rs` (jusqu'à la ligne 1483 sur 2263, tests non lus), `flow-status.md`, `link.md`.
- Les hooks sont posés par `link` dans `.claude/settings.json` (`compile/claude_settings.rs`, `flow_status_entries`), possédés par valeur exacte dans le manifeste ; un projet déjà lié doit relancer `armadai link --target claude` pour recevoir le nouveau hook.
- Barrière complète et mutations sur chaque test (`armadai-rules`) ; sorties des tests à égalité exacte.
- Le journal reste une ligne JSON par événement, ajoutée en une écriture ; les anciennes lignes restent lisibles.

## 4. Conception

### 4.1 Enregistrement (`flow_status/record.rs`, `compile/claude_settings.rs`)
- `link` installe un 4e hook asynchrone, `UserPromptExpansion` (sans matcher), commande `armadai __record-flow-event`, avec le même propriétaire, la même fusion et le même retrait par `unlink` que les trois autres. Le rapport de conformité de `link` garde une seule ligne `flow recording: compiled (...)`, dont le texte nomme les quatre hooks : `SubagentStart, SubagentStop, PreToolUse (SubagentHandback), UserPromptExpansion` (décision 4, §Cadre).
- `event_from_hook` accepte `UserPromptExpansion` avec `expansion_type == slash_command` et écrit `{"ts":N,"event":"command","command":"<command_name>"}`. Aucun autre champ : ni `command_args`, ni `prompt`. Un nouveau variant `EventKind::Command` ; `agent_id` et `agent_type` ne sont pas requis pour cette ligne (`FlowEvent` les met par défaut vides ou devient une énumération ; au choix de l'architecte, les anciennes lignes doivent se lire telles quelles).

### 4.2 Lecture (`flow_status/runs.rs`)
- Un événement `command` dont le nom désigne un flow (règle du MVP : le nom de la commande est identique à l'identifiant du flow) devient une frontière :
  1. toute exécution attribuée encore ouverte est fermée et marquée interrompue (nouvelle marque d'exécution, par exemple `interrupted`, affichée à la place de `in progress` ; les exécutions non attribuées encore ouvertes sont fermées de même) ;
  2. une exécution de ce flow seul s'ouvre, attribuée (pas de groupe `a | b`) ;
  3. tant qu'elle est ouverte, un sous-agent est placé dans cette exécution seule : si l'agent tient une étape unique du flow, il s'y place, **même si aucune étape source n'a fini** (étape `running`, la transition est notée à la fin de la source quand elle finit ; une étape démarrée avant la fin de sa source n'est jamais abandonnée) ; un agent qu'aucune étape du flow ne tient est ignoré (plus d'exécution fantôme pour lui). Les étapes ambiguës (un agent tenant deux étapes) gardent la règle actuelle (`?`).
  4. Une étape où une exécution peut s'ouvrir (une étape d'entrée, ou une étape juste après un relais du coordinateur) n'est pas mise en attente de sa source : sinon le flow `review-loop` (`review → write when rejected`) prendrait un faux lien et se déclarerait fini trop tôt. Une étape `?` ou `✗` ne mène à aucune autre étape, même par un lien tardif : une étape ne se suit pas elle-même par le coordinateur.
  5. Dans une exécution ouverte par une commande, une étape amont qui n'a jamais tourné prend la marque `- … (not run in this session)`, la même que pour l'inférence (décision 6) ; une étape aval qui n'a pas encore tourné reste `○ pending`. Pendant un chevauchement, une étape pas encore lancée peut s'afficher un instant ainsi, puis `running` : c'est vrai au moment du rendu (risque du même ordre que celui déjà accepté au §6). Les exécutions sans commande ne changent pas.
- Une commande qui ne désigne aucun flow n'est pas une frontière et ne change rien.
- Sans événement `command` dans le journal, le code actuel s'applique sans changement.

### 4.3 Reporté après le MVP
- `/clear` comme frontière (`SessionStart` avec `source == clear`), une fois mesuré si `session_id` change.
- L'appel isolé d'un agent qui est aussi une étape du flow attribué (ex. test manuel de `t` pendant `/bugfix`) : indiscernable sans frontière de fin ; piste : fermer l'exécution après un `Stop` de l'agent principal (événement `Stop`, documenté) ou un délai.
- Le chevauchement sans commande enregistrée : afficher `?` pour une étape démarrée sans source finie au lieu de la perdre ; apparier un `SubagentStop` tardif par `agent_id`.
- Un message d'aide quand le hook `UserPromptExpansion` manque (projet à relier).
- Limite connue, non corrigée (décision 5) : un `start`/`stop` forgé avec `agent_id` vide fait perdre l'attribution, comme deux identifiants identiques. Le défaut existait avant le lot et le hook n'écrit pas de telle ligne en usage normal ; elle est documentée dans `docs/wiki/flow-status.md` (tâche 4).

### 4.4 Sortie visible
Une exécution interrompue s'affiche `bugfix  interrupted` ; l'exécution attribuée ne porte jamais `a | b`. Dans une exécution ouverte par une commande, une étape amont jamais jouée s'affiche `- … (not run in this session)` et non `○ pending` (une étape aval pas encore jouée reste `○ pending`). Le reste du format (marques, transitions, UTC) ne change pas. `docs/wiki/flow-status.md` (tableau des hooks, lignes 101, 132 et 142 des limites) et `docs/wiki/link.md` (ligne 57, liste des hooks) sont mis à jour.

## 5. Critères d'acceptation

Environnement : celui de l'issue #527 (projet à deux flows `evolution` et `bugfix`, agents `an u t r v`, `ev` pour écrire les événements à la main). On ajoute `cmd(){ echo "{\"ts\":$((T+$1)),\"event\":\"command\",\"command\":\"$2\"}" >> .armadai/state/claude/s1.jsonl; }`. Chaque critère est rejoué sur un `.jsonl` neuf (`: > ...`).

1. Le hook : `printf '{"session_id":"s1","cwd":"<proj>","hook_event_name":"UserPromptExpansion","expansion_type":"slash_command","command_name":"bugfix","command_args":"secret arg","prompt":"/bugfix secret arg"}' | armadai __record-flow-event` rend le code 0, n'écrit rien sur la sortie standard ni sur l'erreur standard (octets de sortie : 0, car la sortie standard d'un hook `UserPromptExpansion` est ajoutée au contexte du modèle) et ajoute au journal une seule ligne contenant `"command":"bugfix"` et ni `secret` ni `/bugfix`. Un `expansion_type` autre que `slash_command` n'ajoute rien.
2. (a)+(b) `cmd 0 bugfix` puis les événements du premier scénario de l'issue (`t` démarre, `u` démarre avant l'arrêt de `t`) : `armadai flow status` affiche `bugfix  finished` avec `analyse`, `test`, `fix-ui`, `review`, `validate` en `done`, sans `|`.
3. (a) `cmd 0 bugfix` puis le second scénario (`u` démarre, puis `t`, `u` et `t` s'arrêtent ensuite) : `bugfix  finished`, cinq étapes `done` ; avec `cmd 0 evolution` à la place et les mêmes événements : `evolution`, jamais `bugfix`.
4. (d) Un seul flow `bugfix` (`analyse -> test -> fix-ui -> review`), `cmd 0 bugfix`, puis la séquence de l'issue (`t` et `u` se chevauchent) : `bugfix  finished`, quatre étapes `done`.
5. (c) `ev` d'un appel isolé de `t`, puis `cmd 3600 bugfix`, puis une exécution `an -> t -> u -> r -> v` : exactement deux blocs, le premier non attribué marqué `interrupted` (pas `in progress`), le second `bugfix  finished`. Un appel de `x`, agent qu'aucune étape de `bugfix` ne tient, après `cmd bugfix` : sortie identique à celle sans cet appel.
6. Deux commandes dans une session (`cmd bugfix`, exécution complète, `cmd evolution`, `an` démarre) : deux blocs, `bugfix  finished` puis `evolution  in progress` ; les événements après la 2e commande n'apparaissent pas dans le premier bloc.
7. Compatibilité : les scénarios de l'issue sans aucune ligne `command` donnent exactement la sortie de la version courante (`evolution  in progress` pour le second scénario, etc.).
8. `armadai link --target claude` écrit le hook `UserPromptExpansion` dans `.claude/settings.json` en plus des trois autres, le liste dans son rapport, en une seule ligne `flow recording: compiled (...)` dont le texte nomme `SubagentStart, SubagentStop, PreToolUse (SubagentHandback), UserPromptExpansion` ; `armadai unlink` le retire seul sans toucher un hook de l'utilisateur ; les golden de `tests/golden/init_pack/` et `link_new_cli.rs` sont mis à jour : ceux de `settings.json` (le 4e hook) et ceux qui contiennent le rapport (la ligne `flow recording` change de texte ; le nombre de lignes du rapport ne change pas).
9. Les pages `docs/wiki/flow-status.md` et `docs/wiki/link.md` décrivent le hook, la frontière et ce qui reste limité (§4.3).
11. Étape amont sautée (décision 6) : `cmd 0 bugfix`, puis `an`, `u`, `r`, `v` joués en entier sans que `t` (`test`) ait jamais tourné : `bugfix  finished`, `test` affiché `- … (not run in this session)` (ni `○ pending`, ni `done`), les quatre autres `done`. Avec `cmd 0 bugfix` et seul `an` joué : `bugfix  in progress`, `test`, `fix-ui`, `review`, `validate` en `○ pending`. Dans `review-loop` (`review → write when rejected`), `cmd 0 review-loop` puis `review` rejeté et `write` démarré : l'exécution n'affiche pas `finished` avant la fin de `write`. Une exécution sans commande garde la sortie actuelle (critère 7).
10. Essai réel avec Dimitri : sur une vraie session Claude Code, il tape la commande du second flow (celle de l'essai du 2026-10-26, deux flows à étape d'entrée partagée) avec des sous-agents en arrière-plan, et `flow status --watch` affiche ce flow, les étapes `done` au fur et à mesure et `finished` à la fin. La barrière complète est verte et chaque test ajouté a sa mutation vue rouge.

## 6. Risques

- La commande de l'utilisateur ne déclenche pas `UserPromptExpansion` (commande hors skill ou commande native, version de Claude Code) : aucun événement `command`, retour à l'inférence actuelle. Se voit à l'essai réel (critère 10) en lisant le journal.
- Le nom de commande ne correspond pas à l'identifiant du flow : l'exécution reste inférée. Connu et accepté pour le MVP (règle « nom = identifiant du flow ») ; le lien déclaré est reporté.
- Le hook d'enregistrement imprime sur sa sortie standard : son texte entrerait dans le contexte du modèle. Couvert par le critère 1 (zéro octet), y compris sur entrée invalide ou sans projet.
- Placer une étape dont la source n'a pas fini peut afficher un état plus généreux que la réalité (`done` pour un agent qui a tourné hors ordre) : le flow est une guidance, pas un moteur ; l'ordre réel reste lisible dans la liste des transitions.
- Un hook de plus par projet lié : à relier (`link`) pour en bénéficier ; sans cela, comportement actuel.
- Pendant un chevauchement, une étape pas encore lancée peut s'afficher un instant `- … (not run in this session)` puis `running` : vrai au moment du rendu, même ordre de risque que la ligne précédente.
- Les exécutions `interrupted` changent le vocabulaire de la sortie : les tests d'égalité exacte existants doivent rester verts pour les journaux sans commande.

## Questions ouvertes

Aucune pour le MVP. Tranché par Dimitri le 2026-10-26 : le nom de la commande est l'identifiant du flow (lien déclaré reporté) ; sans commande enregistrée, l'inférence actuelle est gardée ; l'appel isolé d'un agent du flow attribué est reporté.

## Cadre

Établi par `framer` le 2026-10-26, avant le plan. Sources : `armadai-rules`, `CLAUDE.md` (racine, « R »), `.claude/CLAUDE.md` (« E »), le code lu (`flow_status/record.rs`, `flow_status/runs.rs` en tête, `cli/flow.rs`, `compile/claude_settings.rs`), `docs/wiki/flow-status.md`, `docs/wiki/link.md`, `CHANGELOG.md`, la spec précédente `2026-10-22-step4-flow-status-design.md`. Aucune contradiction trouvée entre `armadai-rules`, `CLAUDE.md` et `.claude/CLAUDE.md`. `armadai-rules` n'est pas modifiée par cette étape (aucune règle nouvelle décidée).

### Règles du dépôt qui s'appliquent

- Barrière avant push, code retour de chaque commande lu, jamais de `tail` : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model` (R, Build & Test ; E, Barrière ; `armadai-rules`, Barrière). CI : 6 vérifications (fmt, clippy, test, build, Conventional Commits, audit).
- `frozen/armadai-audit` : vérification (`cd frozen/armadai-audit && cargo check --all-targets && cargo test`) seulement si un crate dont il dépend change. Le lot vise `crates/armadai` (binaire) ; si `armadai-core` ou `armadai-model` est touché, le faire (R, Architecture). `armadai-model` reste pur.
- Crate binaire seul : `cargo test --bin armadai <nom>` ou `--test <nom>`, jamais `--lib` ; toujours `--no-fail-fast` ; `--exact` exige le chemin complet (R, Non-obvious facts ; E, Pièges de mesure).
- `armadai_core::test_support::env_lock()` non réentrant : une seule garde par thread (R ; E). Toute valeur de référence se lit sous le verrou ; vérification par 5 à 8 passes (`mémoire:feedback_lire_sous_le_verrou.md`).
- Tests : égalité exacte (lignes entières, pas de `contains` qu'autre chose satisferait), fixtures à valeurs distinctes deux à deux, une mutation vue rouge par test ajouté ou changé, restauration avec `touch` (jamais `cp -p`), la mutation d'un échec rejouée par un tiers (E, Mesure ; mémoire `feedback_tests_qui_ne_peuvent_pas_echouer.md`, `feedback_harnais_de_mutation_qui_ment.md`). Pour ce lot, deux formes à surveiller : (1) la boucle de test sur les scénarios de l'issue qui oublie un scénario (les critères 2 à 6 sont des séquences, pas des cas uniques) ; (2) le critère 1 de la spec (pas de bavardage, `octets de sortie : 0`) se mesure sur stdout et stderr séparément, sur entrée valide, invalide et sans projet.
- Golden outputs de `link`/`unlink` : selon la spec (critère 8), `crates/armadai/tests/golden/init_pack/` et `crates/armadai/tests/link_new_cli.rs` ; l'architecte en dresse la liste exacte au dépôt avant le plan (je n'ai pas pu la rejouer ici : aucune commande de recherche). Le test existant `a_project_with_a_flow_records_subagents_after_the_gate` (`compile/claude_settings.rs`) compare la liste exacte des hooks et change avec le 4e hook.
- Aucune tâche en arrière-plan ; un `CARGO_TARGET_DIR` privé par worktree ; jamais `git stash` ; tout essai du binaire avec `HOME` et `ARMADAI_CONFIG_DIR` isolés (E, Règles communes ; `armadai-rules`, Environnement). Les essais de l'issue #527 écrivent dans un projet temporaire.
- Conventional Commits, un type par sujet, code et commits en anglais, corps de PR en français, trailer `Co-Authored-By` fourni par la session, aucun nom de modèle en dur (R, Git Conventions ; E, Commits ; hook `.githooks/commit-msg`).
- PR ouverte, sans fusion, sauf autorisation expresse du brief de Dimitri ; fusion alors en squash sur CI verte après revue indépendante ; Dependabot vérifié avant (R ; E).
- Ne présenter comme vérifié que ce qu'une sortie réelle a montré ; le critère 10 (essai réel dans Claude Code) revient à Dimitri avec les journaux lus (E ; `armadai-rules`, Rapport).

### Règles propres au sujet

- **Journal** `.armadai/state/claude/<session>.jsonl` : une ligne JSON par événement, une seule écriture en mode ajout, jamais de texte de prompt ni de réponse (spec §2 anti-objectifs, §3 ; `docs/wiki/flow-status.md`, « What is recorded » ; `record.rs`, `record`). La ligne `command` est `{"ts":N,"event":"command","command":"<nom>"}` (spec §4.1) : ni `command_args`, ni `prompt`.
- **Compatibilité ascendante** (spec §3 dernière puce, critère 7) : les anciennes lignes (`start`/`stop` avec `agent_id`, `agent_type`, `state`) se lisent telles quelles ; sans ligne `command`, la sortie est identique au caractère près. Constat au code : `read_log` (`cli/flow.rs`) ignore toute ligne qui ne se désérialise pas en `FlowEvent` ; un ancien binaire lisant un nouveau journal saute donc les lignes `command` sans erreur, et un nouveau lisant un ancien journal ne voit aucune frontière. Il faut garder ces deux propriétés : un test lit un journal ancien et un journal mêlant les deux formes, et `render::sessions` (`--list`) ne doit pas compter une session qui ne contient que des lignes `command` comme une exécution.
- **Hook silencieux** : `armadai __record-flow-event` sort 0 et n'écrit rien sur stdout ni stderr, y compris sur entrée invalide, sans projet, journal non inscriptible ou lien symbolique (`record_from_stdin` journalise en `tracing::debug!` seulement ; garde existante contre les liens symboliques de `state/`). La doc des hooks (https://code.claude.com/docs/en/hooks.md, lue par `lead` le 2026-10-26, l. 814) dit que la sortie standard de `UserPromptExpansion` entre dans le contexte du modèle : un octet écrit, c'est du contexte pollué. Hook `async: true` comme les trois autres.
- **Patch de `settings.json`** : le 4e hook passe par `flow_status_entries` (`compile/claude_settings.rs`) et `compile/patch.rs` (fusion, propriété par valeur exacte dans le manifeste, retrait par `unlink`, jamais d'écrasement des hooks de l'utilisateur ; un hook identique déjà présent n'est ni dupliqué ni revendiqué). Installé sous la même condition que les autres (le projet a au moins un flow). Aucun champ de marquage dans l'objet hook (R `claude_settings.rs`, en-tête). Un projet déjà lié se relie par `armadai link --target claude` (spec §3) ; l'entrée recalculée par `merge_settings` s'ajoute, l'ancien manifeste reste lisible. Vérifier `unlink` avec manifeste (retire le 4e hook seul) et sans manifeste (ne touche pas `settings.json`, comme le dit `docs/wiki/link.md`).
- **Événements documentés seulement** (E, adapter-specialist : vérifier chaque format natif dans la doc officielle avant de l'écrire) : `UserPromptExpansion`, `expansion_type`, `command_name`. La version de Claude Code qui l'introduit est non vérifiée (spec §3) ; l'architecte ou l'adapter-specialist la relève ou l'écrit « non vérifié ».
- **Documentation à tenir** (spec §4.4, critère 9) : `docs/wiki/flow-status.md` : le « three asynchronous hooks » (l. 13), le tableau des hooks (l. 16-20), le résumé du journal (l. 22-24), les puces `a | b` (l. 101) et les limites (l. 132, 142), la table des marques si `interrupted` y entre ; `docs/wiki/link.md` l. 57 (« three async hooks »). Écart de la spec : elle ne cite pas les l. 13 et 22-24 de `flow-status.md`, qui disent aussi « three » et listent les champs du journal. `CHANGELOG.md` : une entrée sous `## Unreleased` (la section existe) ; le hook ajoute une surface, la sortie corrige l'attribution : rubrique `### Fixed` (décision 2).
- **Hors lot** : `/clear`, l'appel isolé d'un agent du flow attribué, le chevauchement sans commande, le message d'aide (spec §4.3) ; `armadai graph` inchangé ; Codex, OpenCode, Copilot inchangés (`flow recording: not-compiled`).

### Observé, non décidé

- Le lot enregistre le nom de **toute** commande tapée (`expansion_type == slash_command`), flow ou non, y compris une commande sans rapport avec le projet : la spec le tranche (« le nom de la commande, rien d'autre »). Une alternative, filtrer à l'écriture par les flows du projet, n'est pas retenue par la spec ; je la signale pour mémoire, sans question (décision de Dimitri, 2026-10-26).
- Ordre des hooks dans `settings.json` et dans le rapport : le code actuel les ajoute dans l'ordre de `flow_status_entries` (Start, Stop, Handback) après la porte de politique ; la convention « le nouveau hook en dernier » est une habitude plausible, non imposée.

### Décisions de Dimitri (2026-10-26)

1. **Ligne de rapport de `link` pour le 4e hook** : « Garder la ligne (Recommandé) ». Une seule ligne `flow recording: compiled` couvre les quatre hooks ; seuls les goldens de `settings.json` changent. Source : Dimitri, 2026-10-26. **Dépassée sur le texte de la ligne par la décision 4** : le nombre de lignes reste 1, le texte change, donc les goldens qui contiennent le rapport changent aussi.
2. **Commit et CHANGELOG** : « fix / Fixed (Recommandé) ». Commit `fix(flow): …`, entrée sous `### Fixed` de `## Unreleased`. Source : Dimitri, 2026-10-26.
3. **Nom de commande** : « Égalité stricte (Recommandé) ». Le `command_name` est égal à l'identifiant du flow, sensible à la casse ; toute autre forme (espace de noms de plugin compris) n'est pas une frontière et retombe sur l'inférence. Source : Dimitri, 2026-10-26.
4. **Tâche 2, ligne de rapport de `link`** : la ligne `flow recording: compiled (...)` ne nommait que 3 hooks alors que `link` en installe 4. Réponse : « Nommer les 4 (Recommandé) ». La ligne nomme `SubagentStart, SubagentStop, PreToolUse (SubagentHandback), UserPromptExpansion` ; il n'y a toujours qu'une ligne, mais les golden qui contiennent le rapport changent d'une ligne de texte. Source : Dimitri, 2026-10-26.
5. **Tâche 3, `agent_id` vide** : un `start`/`stop` forgé avec `agent_id` vide fait perdre l'attribution, comme deux identifiants identiques ; le défaut existait avant le lot et le hook n'écrit pas de telle ligne en usage normal. Réponse : « Ne rien faire (Recommandé) ». C'est une limite connue, à documenter dans `docs/wiki/flow-status.md` à la tâche 4. Source : Dimitri, 2026-10-26.
6. **Tâche 3, étape amont sautée** : dans une exécution ouverte par une commande, une étape amont qui n'avait jamais tourné s'affichait `○ pending` alors que l'exécution pouvait être `finished`. Réponse : « not run in this session (Recommandé) ». Elle prend la marque `- … (not run in this session)`, la même que pour l'inférence ; une étape aval qui n'a pas encore tourné reste `○ pending` ; les exécutions sans commande ne changent pas. Source : Dimitri, 2026-10-26.

Choix de construction approuvés par `guardian` (pas des décisions de Dimitri) : une étape où une exécution peut s'ouvrir n'est pas mise en attente de sa source ; une étape pas encore lancée peut s'afficher un instant `- … (not run in this session)` pendant un chevauchement ; une étape `?` ou `✗` ne mène à aucune autre étape. Voir §4.2, points 4 et 5.

### Questions pour Dimitri

Aucune.
