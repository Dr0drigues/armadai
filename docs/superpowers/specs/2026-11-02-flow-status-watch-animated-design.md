# flow status --watch animé (ASCII)

## 1. Intention

`armadai flow status --watch` redessine aujourd'hui le tableau de marques (`✓ ● ○`) quand le journal change (vérifié une fois par seconde) : on lit des lignes, on ne voit pas la forme du flow ni où l'exécution se trouve dedans. Dimitri a demandé (2026-11-02) : « avec les améliorations de graph, tu penses qu'on pourrait améliorer aussi les watch? avec un système d'animation des flows en ascii etc », puis « lance le lot watch animé ». Le lot a été mis en pause le 2026-11-02 et reprend le 2026-11-09, sur `master` `e263db5`.

Ce qui a changé depuis la pause (vérifié au dépôt, worktree `armadai-watch`, tête `e263db5`) : le lot graph (#538) est fusionné (`ascii_graph::flow_to_ascii`) ; le lot interface L1 a livré `flow status --json`, `--line`, `--color`, `--format md`, la table de marques partagée `Mark::sign()` (✓ ● ○ ? ✗ ~ -) et la sélection « dernière exécution ouverte » de `--line` (`flow_status/line.rs`, `line()`). Décision de Dimitri du 2026-11-05 (lot interface, Q3 « Ce lot d'abord (Recommandé) ») : le watch reprend la table de marques de L1 ; son critère « sans `--watch`, la sortie ne change pas » devient « inchangée hors des nouveaux drapeaux » (spec `2026-11-04-orchestrator-ui-design.md`, §4.1). La PR visée change : #538 étant fusionnée, elle ira vers `master` (la décision Q5 du cadre, « Vers feat/graph-improvements », est caduque).

Dimitri a aussi demandé un résumé d'activité par étape ; découpe décidée le 2026-11-02 : **ce lot-ci est l'animation seule** (« Animation d'abord (Recommandé) »). Le résumé est un lot suivant, dont la réflexion est conservée en fin de document.

État mesuré au dépôt :
- `crates/armadai/src/cli/flow.rs:180-201` : `--watch` est une boucle à 1 s (`// shortcut: polls each second`) qui écrit `\x1b[2J\x1b[H` puis `print_painted(&view.text())` quand la taille du journal ou la session change (`render::should_redraw`). Ni écran alternatif, ni signal : Ctrl-C tue le processus. Pas de crate de terminal : `anstream`/`anstyle` et séquences ANSI brutes.
- `flow_status/runs.rs` : `runs(&Project, &[FlowEvent]) -> Vec<Run>` pure ; `Run { flow, steps: Vec<StepView>, loops, transitions, finished, interrupted }`, `StepView { step, mark, state, note }` ; `Mark::sign()` et `Mark::name()` existent.
- `flow_status/line.rs` : `line(&[Run])` choisit `runs.iter().rfind(|r| !r.finished && !r.interrupted)`, sinon la dernière : la règle « quelle exécution montrer » existe déjà.
- `cli/flow.rs` : `View::snapshot()` (une lecture du journal, runs filtrés par flow), `painted()` (table Mark -> style), `--color` posé par `anstream::ColorChoice::write_global`.
- `ascii_graph.rs:46` : `flow_to_ascii(&FlowGraph) -> String`, pure, sans couleur : libellé `n.label()` (`step` ou `step (agent)`), préfixe `▶ ` au départ, suffixe ` ■` aux fins, arêtes avant dessinées, les autres sous `links not drawn:`. Elle ne porte aucun état d'exécution et construit elle-même ses libellés.
- Un nœud du graphe a pour `id` l'id de l'étape : c'est la clé de rattachement à `StepView.step`.
- `libc` est déclarée dans `armadai-core` sous `[target.'cfg(unix)'.dependencies]` (`libc = "0.2"`), pas dans `crates/armadai`.

## 2. Objectifs, non-objectifs, anti-objectifs

### Objectifs

1. **Vue animée.** Dans un terminal, `flow status --watch` dessine le flow de l'exécution en cours en boîtes ASCII (le dessin du lot graph), chaque étape préfixée de sa marque (`Mark::sign()`), lisible sans couleur, et un repère animé sur l'étape qui tourne.
2. **Réactivité.** La vue se redessine peu après un nouvel événement du journal, sans attendre la seconde de polling actuelle, et l'animation avance entre deux événements tant qu'une étape tourne.
3. **Repli propre.** Hors terminal, ni animation ni séquence d'échappement de positionnement ; la sortie est celle d'aujourd'hui.
4. **Terminal sain.** Le curseur masqué pendant la vue est restauré à toute sortie, Ctrl-C compris.

### Non-objectifs

- Aucun hook nouveau, aucune ligne de journal nouvelle, `link` et `unlink` inchangés. Le résumé d'activité par étape est le lot suivant (« Animation d'abord (Recommandé) », 2026-11-02).
- Claude Code seul : OpenCode, Codex, Copilot gardent `flow recording: not-compiled (Claude Code only for now)` (« Claude Code seul (Recommandé) »).
- `--watch` seul : `flow status` sans `--watch`, `--list`, `--json`, `--line`, `--format md` ne changent pas (« --watch seul (Recommandé) »).
- Pas de crate de terminal : séquences ANSI à la main (« ANSI à la main (Recommandé) »). Ni TUI plein écran, ni saisie clavier, ni écran alternatif.
- Rien de redéfini : ni la table de marques, ni `runs()`, ni le dessin de `flow_to_ascii`, ni le choix « dernière exécution ouverte » de `--line`. Le lot les réutilise.
- Pas de gestion de `SIGTERM`, `SIGWINCH` ni de redimensionnement en direct : la taille est relue à chaque image, sans signal.
- Windows : non mesurable ici ; hors `cfg(unix)`, pas de curseur masqué et pas de gestionnaire (voir 4.2).

### Anti-objectifs

- Afficher un état que le journal ne porte pas : le repère « en cours » suit `Mark::Running`, rien n'est deviné (`?` plutôt qu'une supposition).
- Écrire une séquence d'échappement quand stdout n'est pas un terminal.
- Laisser le terminal en mauvais état (curseur caché) après Ctrl-C ou une erreur.
- Consommer du CPU en veille : sans étape en cours, pas d'animation, donc pas plus de travail qu'avant.
- Dessiner un flow coupé ou faux quand il ne tient pas à l'écran.
- Changer le code retour de Ctrl-C.

## 3. Contraintes

- Claude Code seul ; aucune écriture : `watch` lit le journal, rien d'autre.
- `armadai graph --format ascii` reste identique octet pour octet : le dessin avec marques est une variante de `flow_to_ascii`, pas une modification de sa sortie sans marques.
- Pur autant que possible : l'image est une fonction pure de (exécution, graphe du flow, numéro d'image, taille d'écran), testable sans terminal ni horloge ; le numéro d'image vient de l'appelant.
- Dépendances (décision de Dimitri, cadre 2026-11-02, « libc + handler SIGINT ») : `libc` déclarée dans `crates/armadai` sous `[target.'cfg(unix)'.dependencies]`, comme `armadai-core` ; **aucun paquet nouveau dans `Cargo.lock`** (seule la liste des dépendances d'`armadai` gagne `libc`) ; aucun crate de signal ni de terminal ; `armadai-model` et `armadai-core` inchangés, `cargo build -p armadai-model` et `-p armadai-core` passent.
- `unsafe` limité à un seul module restreint (gestionnaire `SIGINT`, `ioctl`), chaque bloc commenté de son invariant ; le gestionnaire ne fait que de l'async-signal-safe.
- Terminal ou non : `std::io::IsTerminal` sur stdout, seul (« IsTerminal seul (Recommandé) ») ; la couleur reste à `anstream` et à `--color` ; `NO_COLOR` retire la couleur, pas l'animation, puisque la marque se lit en texte brut.
- Taille d'écran (« Variables puis ioctl (Recommandé) ») : `COLUMNS`/`LINES`, puis `ioctl(TIOCGWINSZ)` via `libc`, puis un défaut fixe (valeur fixée au plan).
- Tests (« Pur + hors terminal (Recommandé) ») : fonctions pures en test unitaire ou golden, un test `assert_cmd` hors terminal ; la boucle en terminal et Ctrl-C se vérifient à l'essai réel, pas de pty.
- Règles `armadai-rules` : barrière complète ; mutation vue rouge pour chaque test ajouté ; égalité de lignes entières plutôt que `contains` ; texte de toute ligne nouvelle de sortie proposé par le plan et validé par Dimitri ; essai du binaire avec `HOME` et `ARMADAI_CONFIG_DIR` isolés ; reconstruire (`cargo build`) après une mutation avant tout essai au binaire ; bornes de temps choisies par rapport au seuil du défaut, stabilité par 5 passes ; target cargo privé par worktree.
- Pas de nouveau format de journal : les journaux existants sont lus tels quels.

## 4. Conception

### 4.1 L'image (fonctions pures)

- **Dessin avec marques.** `ascii_graph` gagne une variante de `flow_to_ascii` qui reçoit, par id d'étape, un préfixe de libellé ; `flow_to_ascii` en est le cas sans préfixe et garde sa sortie octet pour octet. Le préfixe est `Mark::sign()` suivi d'une espace : une marque a toujours la même largeur (un glyphe), donc le changement d'état d'une étape ne déplace pas le dessin. Une étape du graphe sans `StepView` dans l'exécution n'a pas de marque (aucun état n'est inventé).
- **Repère animé.** Le glyphe `●` de l'étape `Running` reste celui de la table. Le repère animé est un glyphe de rotation d'une cellule (proposition ASCII : `|` `/` `-` `\`) placé après le libellé de l'étape en cours, dans une cellule réservée à tous les nœuds (vide sauf sur l'étape `Running`) pour que la largeur des boîtes ne dépende pas de l'étape qui tourne. Fonction pure `spinner(tick) -> char`. (Décision du 2026-11-09.)
- **Image** `frame(run, graph, tick, size)` : le dessin avec marques, puis, sous lui, le bloc complet de `render::render` de cette exécution (lignes d'étapes avec leur état et leur note, liens pris plusieurs fois, transitions UTC) pour qu'aucune information de la vue actuelle ne disparaisse (décision du 2026-11-09, plan, Q1 : « Le bloc complet (Recommandé) »). Les noms et le texte exact sont fixés au plan.
- **Quelle exécution est dessinée** (décision de Dimitri, 2026-11-02, « Une dessinée + liste (Recommandé) ») : une seule, l'exécution en cours ; les autres (terminées, interrompues, et toute autre en cours) restent en liste au format actuel, sous le dessin. Le choix de l'exécution dessinée reprend la règle de `--line` (`line.rs`, dernière exécution ouverte) : extraite et partagée, pas recopiée (décision du 2026-11-09 : la dernière ouverte).
- **Pas de dessin** pour une exécution qui n'a pas un seul flow connu (`run.flow` de la forme `a | b`, `GROUP_SEP`), ni quand il n'y a aucune exécution en cours : liste actuelle.
- **Repli de taille.** Si le dessin (largeur du plus long ligne, hauteur dessin + lignes sous lui) dépasse la taille d'écran, l'exécution reste en liste, jamais un dessin coupé. Le test de taille est une fonction pure de (image, taille).
- **Flows à cycle** : les arêtes non dessinées restent sous `links not drawn:` (comportement de `flow_to_ascii`), chaque étape et chaque lien restent nommés.

### 4.2 La boucle `--watch`

- **Dans un terminal** (`IsTerminal`) : pas court (proposition : 200 à 250 ms, fixé au plan) ; le journal n'est relu que si sa taille ou la session change (déjà en place) ; l'animation ne relit rien. Le redessin place le curseur en tête (`\x1b[H`), écrit chaque ligne suivie de `\x1b[K`, puis `\x1b[J` : pas d'effacement complet à chaque image (pas de clignotement), un seul `\x1b[2J` à la première image. Sans étape `Running` : image fixe, la boucle garde son pas court mais ne fait qu'un `stat` du journal, sans relecture ni redessin (décision du 2026-11-09, plan, Q2 : « 250 ms, sans redessin (Recommandé) »).
- **Curseur masqué** pendant la vue, restauré par une garde (`Drop`) à toute sortie normale ou d'erreur, et par le gestionnaire `SIGINT` (`libc::signal`/`sigaction`, `unsafe` restreint) qui écrit la restauration (`\x1b[?25h`) avec `write(2)`, remet l'action par défaut du signal, puis se ré-envoie `SIGINT` : le processus meurt du signal comme aujourd'hui, donc **le code retour de Ctrl-C est celui d'avant le lot** (mesuré sur `master` avant tout changement, valeur reportée au plan). Hors `cfg(unix)` : pas de gestionnaire, curseur non masqué.
- **Hors terminal** : une image en texte brut par changement du journal, comme aujourd'hui, sans animation ; le code n'écrit aucune séquence de positionnement ni d'effacement (aujourd'hui `anstream` les retire en `auto` ; le lot ne les émet plus du tout, y compris avec `--color always`, qui ne règle que la couleur).
- Couleur : celle de `--color` et d'`anstream` ; la marque de chaque étape reprend la table Mark -> style de `painted()` (extraite si besoin, pas redéfinie).

### 4.3 Doc

`docs/wiki/flow-status.md` : ligne `--watch` de « Reading the output » (la vue animée, ce qu'elle montre, repli hors terminal, taille, limites) ; `--help` de `--watch` (aujourd'hui « Redraw on every new event (checked each second) until Ctrl-C ») ; `CHANGELOG.md` sous `## Unreleased`. Texte exact des lignes nouvelles : proposé par le plan, validé par Dimitri.

## 5. Critères d'acceptation

Environnement : un projet à flows et un journal écrit à la main sous `.armadai/state/claude/<session>.jsonl` (méthode des lots précédents), `HOME` et `ARMADAI_CONFIG_DIR` isolés.

1. Pour une exécution `in progress` dont une étape est `running`, la fonction pure d'image rend le dessin de `armadai graph --format ascii <flow>` (mêmes boîtes, mêmes arêtes) où chaque étape porte le préfixe `Mark::sign()` de sa marque ; deux images du même état avec deux numéros d'image différents ne diffèrent que par la cellule du repère, et le dessin garde la même largeur ; l'étape en cours garde le glyphe `●` à chaque image et le repère tournant (`| / - \`, décision du 2026-11-09) est la seule cellule qui change, vide sur les autres étapes.
2. Sur le texte brut d'une image (sans couleur), une étape finie, l'étape en cours et une étape à venir se lisent par `✓`, `●`, `○` ; `?`, `✗`, `~`, `-` y gardent le sens de la légende de `docs/wiki/flow-status.md`, et ces glyphes sont ceux de `Mark::sign()` (un changement de la table change l'image).
3. Quand stdout n'est pas un terminal (`armadai flow status --watch | cat` interrompu après un court délai, redirection vers un fichier), la sortie ne contient aucun octet `\x1b` sans `--color always` et, **même avec `--color always`**, aucune séquence autre que SGR (aucun `\x1b[H`, `\x1b[2J`, `\x1b[J`, `\x1b[K`, `\x1b[?25l`) ; elle ne contient aucune animation (le repère ne change pas) et pas plus d'une image par changement du journal.
4. Ctrl-C pendant `--watch` dans un terminal rend la main avec le curseur visible et le même code retour que `master` (valeur mesurée avant le lot) ; il en va de même après une sortie par erreur (journal supprimé, projet devenu invalide). Mesuré à l'essai réel.
5. Un journal sans étape `running` (tout fini, ou rien commencé) donne une image fixe : sur dix secondes d'attente, aucun redessin, et pas plus de CPU que `armadai flow status --watch` sur `master` (mesuré aux deux, même journal).
6. Un nouvel événement ajouté au journal pendant `--watch` apparaît à l'écran en moins d'une seconde (borne choisie par rapport au polling actuel de 1 s ; mesure répétée 5 fois).
7. Sur un journal à plusieurs exécutions (une `in progress`, une `finished`, une `interrupted`) l'image dessine l'exécution `in progress` et liste les deux autres au format de liste actuel ; avec deux exécutions `in progress`, la dernière ouverte (la même que choisit `armadai flow status --line`, décision du 2026-11-09) est dessinée et l'autre est listée ; une exécution `a | b` n'est pas dessinée et reste listée ; sur un flow à cycle (`plan-build-verify`, `review-loop`) chaque étape et chaque lien sont nommés, les liens non dessinés restent sous `links not drawn:`, sans panique ni sortie vide ; quand la taille donnée à la fonction pure est trop petite pour le dessin, l'image est la liste actuelle de cette exécution, jamais un dessin coupé.
8. Inchangé hors des nouveaux drapeaux : `armadai flow status`, `flow status <flow>`, `--session`, `--list`, `--json`, `--line`, `--format md` donnent exactement la sortie d'avant le lot (golden ou comparaison ligne à ligne avec `master`) ; `armadai graph --format ascii` aussi, octet pour octet.
9. `armadai link --target claude` sur un projet à flows produit exactement les mêmes hooks et le même rapport qu'avant le lot ; `Cargo.lock` ne gagne aucun paquet (seule la liste des dépendances d'`armadai` gagne `libc`) ; `libc` n'apparaît pas dans `armadai-model` ; `cargo build -p armadai-core` et `-p armadai-model` passent.
10. Essai réel avec Dimitri : sur une vraie session Claude Code avec un flow à sous-agents, il lance `flow status --watch`, voit le dessin suivre l'exécution, juge la lisibilité et l'absence de clignotement dans son terminal (et dans tmux s'il l'utilise), Ctrl-C lui rend un terminal propre (curseur visible), et la taille du terminal est bien prise en compte. La barrière complète est verte et chaque test ajouté a sa mutation vue rouge (par exemple : repère constant, préfixe de marque permuté, repli de taille supprimé, séquence de positionnement émise hors terminal).

## 6. Risques

- **Clignotement et terminaux.** Redessin par `\x1b[H` + `\x1b[K` + `\x1b[J` sans crate ; tmux et terminaux étroits ; seul macOS est mesurable ici. Se voit au critère 10. Si le clignotement gêne, la question d'un crate se rouvre auprès de Dimitri ; on n'en ajoute pas de soi-même.
- **`unsafe` et signal.** Un gestionnaire `SIGINT` mal écrit peut bloquer ou perdre le code retour. Parade : seulement `write(2)`, remise de l'action par défaut, ré-émission du signal. Se voit au critère 4.
- **`ioctl` hors terminal ou sur une plateforme sans `TIOCGWINSZ`.** Repli sur le défaut fixe, jamais d'échec.
- **Largeur du préfixe et du repère.** Marque d'un glyphe, cellule du repère réservée : seul le glyphe change entre images (critère 1). Un glyphe de largeur double d'un terminal ferait décaler le dessin : à l'essai réel.
- **Hauteur d'écran.** Une image en liste plus haute que l'écran défile et casse le repositionnement en tête ; la liste est écrite en entier comme aujourd'hui. À regarder à l'essai réel.
- **Plusieurs exécutions en cours** : tranché le 2026-11-09 (la dernière ouverte) ; les autres restent en liste.
- **Boucle serrée en veille** : critère 5.
- **Rebase** : `cli/flow.rs` et `flow_status/render.rs` ont bougé avec L1 ; la branche part de `master` `e263db5`, aucun conflit attendu avec les lots interface suivants (L2, L4 touchent `link` et des sous-commandes cachées).
- **Étendue.** Moyenne (un module `unsafe`, une variante de dessin, la boucle) ; si le plan juge le lot trop gros, il le dit à `lead`.

## Décisions de Dimitri (mot pour mot)

2026-11-02 :
1. Placement : « Lot suivant (Recommandé) ».
2. Découpe : « Animation d'abord (Recommandé) ».
3. OpenCode : « Claude Code seul (Recommandé) ».
4. Surface : « --watch seul (Recommandé) ».
5. Terminal : « ANSI à la main (Recommandé) ».
6. Exécutions multiples : « Une dessinée + liste (Recommandé) ».

2026-11-02, cadre :
- Ctrl-C : « libc + handler SIGINT ».
- Taille d'écran : « Variables puis ioctl (Recommandé) ».
- Hors terminal et couleur : « IsTerminal seul (Recommandé) ».
- Tests : « Pur + hors terminal (Recommandé) ».
- Base de la PR : « Vers feat/graph-improvements (Recommandé) », **caduque** : #538 est fusionnée (`e263db5`), la PR ira vers `master`.

2026-11-09 :
- Q1, exécution dessinée quand plusieurs sont `in progress` : « La dernière ouverte (Recommandé) » (règle de `--line`, partagée).
- Q2, repère de l'étape en cours : « ● + repère tournant (Recommandé) » (`●` reste, cellule `| / - \` réservée à toutes les boîtes).

2026-11-05, lot interface, Q3 : « Ce lot d'abord (Recommandé) » : le watch reprend le JSON et la table de marques de L1 ; critère « sans `--watch`, inchangé » devenu « inchangé hors des nouveaux drapeaux ».

## Ce qui est touché dans ce qui est déjà fait

Aucun plan ni aucune tâche n'existe pour ce lot (rien de commité avant la pause). Le plan est à écrire sur cette version : il reprend `flow_status::line` (sélection de l'exécution), `Mark::sign()`, `painted()` et `ascii_graph` ; il fixe les valeurs laissées ouvertes (pas d'animation, taille par défaut, texte des lignes nouvelles, code retour de Ctrl-C mesuré sur `master`).

## Questions ouvertes

Aucune : Q1 et Q2 sont tranchées le 2026-11-09 (voir « Décisions de Dimitri »).

## Questions pour Dimitri

Aucune pour ce lot.

---

## Lot suivant (hors périmètre) : résumé d'activité par étape

Ne fait pas partie de ce lot. Conservé avec l'état de la réflexion du 2026-11-02, pour être repris tel quel ; Q1 à Q3 n'ont pas été posées à Dimitri et se posent au lot suivant.

### Ce que Dimitri a demandé (2026-11-02, mot pour mot)

« petite question, fans watch, tu penses qu'on peut ajouter une step résumé aussi?

Exemple: on lance un flow, l'analyse commence, on à trois fichier, on ajoute:
- lecture du fichier {{filename}} *3 du coup
- erreur trouvée dans le {{filename}}
- ask: j'ai trouvé cette erreur:
- user_answer: corrige
- correction du {{filename}} »

### Avis de `lead` (proposition, non tranchée par Dimitri)

Ne capter que des événements d'outil réels (lecture, édition, écriture de fichier, question posée et réponse) via les hooks de l'outil ; ne produire aucune conclusion sémantique (« erreur trouvée ») : ArmadAI n'a plus de protocole de marqueurs (`ARMADAI_*` retirés) et n'exécute pas d'agents. Points à trancher : confidentialité (chemins, texte des questions et des réponses stockés dans `.armadai/state/`), volume (regrouper : « lecture ×3 »), OpenCode (déjà tranché pour ce lot : Claude Code seul ; à reconfirmer pour le résumé).

### Ce que la doc officielle dit (lue le 2026-11-02, https://code.claude.com/docs/en/hooks.md)

- Champs communs : `session_id`, `prompt_id`, `transcript_path`, `cwd`, `permission_mode`, `hook_event_name` ; les événements d'outil portent `tool_name`, `tool_input`, `tool_use_id`, et, dans un sous-agent, `agent_id` et `agent_type`. Le matcher d'un événement d'outil filtre sur le nom de l'outil (`Edit|Write`, `mcp__.*`).
- `PostToolUse` : `tool_input` et `tool_response` (+ `duration_ms`) ; exemple documenté pour `Write` : `tool_input {file_path, content}`, `tool_response {filePath, type}`.
- `tool_input` de `Read` : `file_path` (absolu), `offset`, `limit` ; de `Edit` : `file_path`, `old_string`, `new_string`, `replace_all` ; de `Write` : `file_path`, `content`.
- `AskUserQuestion` : `tool_input` porte `questions` (`question`, `header`, `options`, `multiSelect`) et un champ `answers` que Claude ne renseigne pas : un hook `PreToolUse` le fournit par `updatedInput`.
- **Non vérifié** (84 000 caractères sur 284 000 de la page non lus) : si `PostToolUse` se déclenche pour `AskUserQuestion` ; si la réponse de l'utilisateur figure dans son `tool_response` (sinon « user_answer » n'est pas enregistrable par hook : il faudrait lire le transcript) ; la forme de `tool_response` pour `Read` et `Edit` ; l'existence d'un outil `MultiEdit`. À vérifier dans la doc et par une capture de payload réel avant d'écrire un format.

### Proposition d'enregistrement (à reprendre au plan du lot suivant)

- Un hook `PostToolUse` `async`, commande `armadai __record-flow-event`, matcher restreint (proposition : `Read|Edit|Write|AskUserQuestion`, à confirmer), ajouté à `flow_status_entries`, rapport `link` : une seule ligne `flow recording: compiled (...)` dont le texte nomme le nouveau hook, validé par Dimitri. Un nouvel `EventKind` `Tool` : ligne `{"ts":N,"event":"tool","agent_id":"...","tool":"Read","path":"<...>"}` rattachée à l'étape par `agent_id`. Mêmes garde-fous que les autres lignes. Un appel sans `agent_id` vient du fil principal (non enregistré aujourd'hui).
- Un matcher large multiplierait les lancements du binaire (un processus par appel d'outil) : coût non mesuré.
- Compatibilité : lignes anciennes lisibles, sortie identique sans ligne `tool`, `--json` l'accueille par un champ ajouté (règle « ajout permis » du schéma `armadai.flow-status/1`), `flow status --list` ne compte pas une session qui ne contient que des lignes d'activité.

### Tableau de traduction de l'exemple de Dimitri

| Ce que Dimitri a écrit | Ce que la vue peut montrer | Source |
|---|---|---|
| lecture du fichier X ×3 | `read ×3` (ou les trois noms, selon Q1) | `Read` |
| erreur trouvée dans X | **rien** : une conclusion de l'agent, pas un appel d'outil | non capturable |
| ask : j'ai trouvé cette erreur | `asked 1 question` (ou son texte, selon Q2) | `AskUserQuestion` |
| user_answer : corrige | `answered` (ou la réponse, selon Q2) | `PostToolUse` d'`AskUserQuestion` si sa réponse y figure (non vérifié) |
| correction du X | `edited X` | `Edit` / `Write` |

Regroupement : des appels du même outil consécutifs dans une même étape se fondent en un compteur ; par étape, un nombre borné de lignes de résumé, le reste replié.

### Risques propres (rappel)

Confidentialité (la doc publie « no prompt, no reply text » ; chemins et textes de conversation sur disque), coût d'un lancement par appel d'outil, attribution à l'étape pour les sous-agents en arrière-plan ou un agent tenant deux étapes, réponse de l'utilisateur peut-être non capturable, hook non reçu sur un projet lié avant le lot.

### Questions pour Dimitri (lot suivant, non posées)

**Q1. Ce qu'on garde d'un fichier lu ou édité dans le journal.** Change la confidentialité et la doc.
- (a) Chemin relatif à la racine du projet, rien d'autre (Recommandé : « lecture de src/main.rs ×3 », sans chemin de la machine).
- (b) Seulement le nom de fichier (sans dossier).
- (c) Seulement un compteur par outil, aucun nom.
- Une autre réponse.

**Q2. Les questions posées à l'utilisateur et ses réponses.** Contredit « no prompt, no reply text » si le texte est gardé ; la réponse n'est peut-être pas disponible (non vérifié).
- (a) Événement seul : « question posée », « réponse reçue », aucun texte (Recommandé).
- (b) Le titre court (`header`) de la question et le libellé de l'option choisie.
- (c) Le texte complet de la question et de la réponse.
- Une autre réponse.

**Q3. Le fil principal (coordinateur) dans le résumé.** Ses appels d'outil n'ont pas d'`agent_id`.
- (a) Les ignorer (Recommandé).
- (b) Une ligne « fil principal » au niveau de l'exécution, sans rattachement à une étape.
- Une autre réponse.
