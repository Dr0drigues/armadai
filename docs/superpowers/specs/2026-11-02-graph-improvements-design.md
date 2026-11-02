# Améliorations de `armadai graph`

## 1. Intention

`armadai graph` n'imprime aujourd'hui que le Mermaid d'un flow (ou de chaque flow, un bloc par flow) : les nœuds sont dans l'ordre alphabétique des ids d'étape, rien ne distingue le départ ni les fins, la délégation entre agents n'est pas montrée, et il faut coller le texte dans un outil tiers pour voir quelque chose. Dimitri a demandé (2026-11-02) « des améliorations de la sub-command graph » et a retenu les quatre pistes proposées : ordre du flow, vue de l'équipe, rendu direct, autres formats. Priorité : v1, avant #455. Réponses de Dimitri (2026-11-02, mot pour mot) : rendu direct « --open mermaid.live (Recommandé) » ; ASCII « Boîtes via un crate » ; vue de l'équipe « --view flows|team (Recommandé) » ; ordre « Oui, assumé (Recommandé) » ; découpe « Un seul lot ». Reste ouvert : le choix du crate ASCII (voir « Questions ouvertes »).

État mesuré au dépôt (branche `feat/graph-improvements`, depuis `ce3d84f`) :
- `crates/armadai/src/cli/graph.rs` : `GraphFormat` n'a qu'une valeur (`Mermaid`) et `execute` ignore le format ; stdout = Mermaid seul, diagnostics sur stderr.
- `crates/armadai-model/src/mermaid.rs` : `flow_to_mermaid(id, &Flow)`, nœuds parcourus par `BTreeMap` (ordre alphabétique des étapes), arêtes dans l'ordre des liens puis des `on_failure` (tirets). Pas d'information sur `start` ni sur les `states`.
- `armadai_model::entry_steps` (`check.rs`) est la définition unique d'une étape d'entrée (`start` s'il est déclaré, sinon calcul). Elle est réutilisable telle quelle.
- La logique de délégation du gate (`allowed_targets`) vit dans le crate binaire (`claude_adapter/policy_gate.rs`), pas dans `armadai-model` ; elle rend des noms d'agents, avec la sémantique du gate (coordinateur jamais cible, `free_agents` depuis partout).
- Aucun crate de rendu (SVG, graphe, ASCII) n'est présent dans les dépendances.
- Sorties attendues : pas de fichier golden, mais des chaînes en ligne dans `crates/armadai-model/tests/mermaid.rs` (9 tests) et `crates/armadai/tests/graph_cli.rs` (constantes `REVIEW` et `SHIP`), plus deux assertions `contains` dans `crates/armadai/tests/new_cli.rs` (:272-273, :438-440).
- Doc : `docs/wiki/project-format.md` (section « Seeing a flow: `armadai graph` », :272-279), `docs/wiki/templates.md` (:84, :98), `docs/wiki/getting-started.md:68`, `README.md` (:40, :66, :87).

## 2. Objectifs, non-objectifs, anti-objectifs

### Objectifs

1. **Ordre du flow.** Les nœuds d'un flow suivent l'ordre du flow depuis `start` (parcours déterministe), non l'ordre alphabétique. Le départ et les fins sont mis en évidence.
2. **Vue de l'équipe.** Un graphe de délégation (qui peut appeler qui) et la lecture « agents par flow », en plus de la vue par flow.
3. **Rendu direct.** Obtenir une image ou une page affichable sans coller du texte à la main.
4. **Autres formats.** DOT, JSON (nœuds et arêtes), ASCII, en plus de Mermaid.

### Non-objectifs

- Aucune modification du modèle canonique, du format `.armadai/`, des codes de diagnostic ni des schémas JSON : `graph` lit, il n'écrit rien dans le projet.
- Pas de rendu interactif, d'édition ni de serveur.
- Pas de graphe pour Codex/Copilot, ni de graphe des automations, skills ou commandes.
- Pas de changement du gate ni de la sémantique de délégation : la vue équipe la montre, elle ne la redéfinit pas.

### Anti-objectifs

- `graph` ne doit jamais toucher le réseau lui-même (règle de `CLAUDE.md` : seuls `init --pack`, `skills sync`/`add` et `update` le font).
- `armadai-model` ne doit pas gagner clap, tokio, réseau ni dépendance hors de sa liste (serde, serde-saphyr, serde_json, thiserror, schemars).
- stdout ne doit porter que le résultat demandé ; erreurs et diagnostics sur stderr, code retour 1 (comportement actuel conservé).
- Aucune dépendance qui échoue à `cargo audit` ou casse `cargo build -p armadai-model` / `-p armadai-core`.

## 3. Contraintes

- Barrière unique de la CI (fmt, clippy `-D warnings`, test, builds `armadai-core` et `armadai-model`, audit) ; une seule configuration, pas de feature optionnelle.
- Sortie déterministe (déjà vraie : les tests comparent des chaînes exactes).
- `entry_steps` reste la seule définition d'une étape d'entrée.
- Règles de `armadai-rules` : mutation de chaque test ajouté ; comparer des lignes entières plutôt qu'un `contains` ; le texte de toute ligne nouvelle de sortie est proposé par le plan et validé par Dimitri.
- Compatibilité : le flag `--format` et la sortie Mermaid existants sont publiés ; l'ordre des nœuds change (assumé par Dimitri), l'ensemble des nœuds et des arêtes non.

## 4. Conception

Principe : la structure (nœuds, arêtes, rôles) se calcule une fois, de façon pure, dans `armadai-model` ; chaque format n'est qu'un rendu de cette structure. Mermaid, DOT, JSON et ASCII sont du texte pur : ils tiennent dans `armadai-model` sans dépendance. Le SVG/PNG/ouverture est hors de ce crate.

### 4.1 Ordre du flow (objectif 1)

- `armadai-model` : un graphe intermédiaire d'un flow, étapes ordonnées : les étapes d'entrée (`entry_steps`, `start` en tête), puis parcours en largeur par liens puis `on_failure` ; les étapes inaccessibles à la suite, par ordre alphabétique. Un cycle ne boucle pas.
- Rôles : `start` (étape d'entrée), `fin` (étape sans lien sortant, `on_failure` compris). Une étape peut être les deux (flow d'une seule étape).
- Mermaid : `start` et fins portent une classe (`classDef`) ; les fins comme le départ restent lisibles en texte brut (la mise en évidence ne doit pas dépendre de la couleur seule : forme ou libellé distincts, à fixer au plan).
- Ce qui n'est pas dans le lot : réordonner l'ordre des flows (reste l'ordre alphabétique des ids de flow).

### 4.2 Vue de l'équipe (objectif 2)

- Graphe de délégation : nœuds = agents du projet, plus le coordinateur (s'il y en a un) comme source « fil principal » et les `free_agents` ; arêtes = délégations permises sous les règles de `policy-gate.md` (du fil principal : tous les agents d'étape avec coordinateur, sinon les agents des étapes d'entrée ; d'un agent X : les agents des étapes atteintes depuis une étape de X par lien ou `on_failure` ; `free_agents` depuis partout). Le coordinateur n'est jamais une cible.
- Dériver ces arêtes de la même logique que le gate, pas d'une seconde copie : le plan décide s'il faut déplacer `allowed_targets` dans `armadai-model` (il ne dépend que de `Project` et `entry_steps`) ou une fonction commune. Les `free_agents` depuis partout seraient illisibles en arêtes : ils sont rendus comme un groupe distinct, pas comme N×M arêtes.
- « Agents par flow » : pour chaque flow, la liste des agents de ses étapes (groupes/sous-graphes selon le format).
- Fonctionne que `policy.delegation` soit `off` ou `strict`, avec une mention de la politique active.
- Interface (décidée) : `graph --view team` ; `--view flows` est le défaut.

### 4.3 Rendu direct (objectif 3)

Décidé : `armadai graph --open` ouvre le diagramme dans mermaid.live, dans le navigateur de l'utilisateur. `armadai` ne fait aucun appel réseau : il construit une URL et lance la commande d'ouverture du système (`open` sur macOS, `xdg-open` sous Linux, `start` sous Windows) ; seul le navigateur se connecte.

Format du lien (vérifié à la source, `src/lib/util/serde.ts` du dépôt mermaid-live-editor, branche `develop`, lu le 2026-11-02) : `<serde>:<charge>` placé dans le fragment de l'URL ; serde par défaut `pako` : JSON de l'état (`JSON.stringify`) encodé en UTF-8, compressé en `deflate` niveau 9, puis base64 URL-safe (`fromUint8Array(compressed, true)`). Un serde `base64` existe aussi (même JSON, sans compression, base64 URL-safe) ; un fragment sans `:` est lu comme du base64 hérité. Champs de l'état (`src/lib/types.d.ts`) : `code` (le Mermaid), `mermaid` (la config, chaîne JSON), `updateDiagram` et `rough` sont obligatoires ; les autres sont facultatifs.
Non vérifié (à confirmer par le plan avant d'écrire le code) : le chemin exact de la page (`https://mermaid.live/edit#pako:…` est la forme usuelle, mais ni le README ni la doc lus ne la citent), le contenu minimal de `mermaid` et la présence ou non du `=` de remplissage dans le base64 URL-safe de `js-base64`.

Conséquences :
- Une dépendance de compression (`deflate`) et de base64 est nécessaire dans le crate binaire (pas dans `armadai-model`). Elle passe `cargo audit` (critère 10) ; le choix est au cadre. À défaut, le serde `base64` sans compression n'exige que du base64, mais donne des URL très longues ; à trancher au plan.
- Mermaid lit un diagramme par document : `--open` exige un seul flow (ou la vue équipe) ; avec plusieurs flows, erreur qui les liste, code 1.
- Les noms d'étapes et d'agents quittent la machine via le navigateur : `--open` est un opt-in explicite, jamais implicite.
- Échec (commande d'ouverture absente, navigateur introuvable) : message sur stderr qui donne l'URL, code 1, stdout vide.

### 4.4 Autres formats (objectif 4)

- `dot` : un `digraph` Graphviz par flow (ids quotés, mêmes rôles `start`/`fin`, mêmes arêtes pointillées pour `on_failure`).
- `json` : un document avec `nodes` et `edges` (id d'étape, agent, états, rôle ; origine, cible, condition, nature `link`/`on_failure`) et, pour la vue équipe, le graphe de délégation. Sortie pour l'outillage : champ stable, trié, documenté dans `docs/wiki/project-format.md` ; pas de schéma JSON publié dans ce lot sauf décision contraire.
- `ascii` : dessin en boîtes dans le terminal, produit par un crate (décision de Dimitri). Le crate n'est pas choisi (voir « Questions ouvertes ») ; critères de choix : licence compatible, maintenu, passe `cargo audit`, comportement sur les cycles, crate d'accueil. Si le crate tire des dépendances lourdes, ce rendu va dans le crate binaire `armadai`, jamais dans `armadai-model` (qui reste pur) ; le graphe intermédiaire, lui, reste dans `armadai-model`. Pas de couleur obligatoire.
- Flags (décidés) : `--format mermaid|dot|json|ascii` (le flag existant, étendu ; `mermaid` reste le défaut), `--view flows|team` (défaut `flows`), `--open` (mermaid.live). Quels formats valent pour la vue `team`, et comment `--open` se combine avec `--format`, sont à poser au plan (`--open` n'a de sens qu'avec Mermaid).

### 4.5 Doc

Mise à jour de la section `graph` de `docs/wiki/project-format.md` (formats, vue équipe, rendu direct), des exemples de `docs/wiki/templates.md` si l'ordre des nœuds les change, et du `README` si une commande y est citée.

## 5. Critères d'acceptation

Flags : `--format mermaid|dot|json|ascii`, `--view flows|team`, `--open`.

1. Sur un flow dont l'ordre alphabétique des étapes diffère de l'ordre depuis `start`, `armadai graph <flow>` déclare les nœuds dans l'ordre du flow ; le jeu des nœuds et des arêtes est celui d'avant.
2. Sur ce même flow, le nœud de départ (celui d'`entry_steps`) et chaque étape sans lien sortant sont marqués ; une étape intermédiaire ne l'est pas.
3. Deux exécutions successives sur le même projet donnent une sortie identique octet pour octet, dans chaque format.
4. `armadai graph --format dot <flow>` imprime un `digraph` valide (accepté par `dot -Tsvg` quand Graphviz est installé, ou par un parseur DOT en test) avec les mêmes nœuds, arêtes et rôles que le Mermaid.
5. `armadai graph --format json <flow>` imprime un JSON valide dont `nodes` et `edges` correspondent exactement aux étapes et liens du flow (compte et contenu), `on_failure` distingué des liens.
6. `armadai graph --format ascii <flow>` imprime sur stdout un dessin en boîtes qui nomme chaque étape du flow, sans séquence d'échappement quand stdout n'est pas un terminal. Sur un flow à cycle (exemple : `plan-build-verify`, où un rejet revient en arrière), la sortie nomme encore chaque étape et chaque lien, et le code retour est 0 ; si le crate retenu ne dessine pas le cycle, les liens de retour sont écrits en liste sous le dessin, sans panique ni sortie vide.
7. `armadai graph --view team`, sur le projet de ce dépôt (`.armadai/`, coordinateur `lead`, `strict`), montre `lead` comme source vers tous les agents d'étape, `build-*` vers `guard`, `guard` vers les `build-*` (retour sur rejet), et ne montre jamais `lead` comme cible ; pour un projet sans coordinateur, seules les étapes d'entrée sont atteintes depuis le fil principal.
8. La vue `team` est cohérente avec le gate : pour chaque paire (appelant, cible) affichée, `armadai __claude-policy-gate` (ou `allowed_targets`) autorise cette délégation, et inversement pour chaque cible autorisée hors `free_agents` ; vérifié par un test qui compare les deux.
9. `armadai graph <flow> --open` lance la commande d'ouverture du système avec une URL mermaid.live dont le fragment, décodé (base64 URL-safe puis inflate puis JSON), donne un état dont `code` est exactement le Mermaid de `armadai graph <flow>`. Vérifié avec une fausse commande d'ouverture placée en tête du `PATH` qui enregistre son argument ; stdout vide ; avec plusieurs flows et sans nom, erreur qui les liste, code 1 ; commande d'ouverture absente : URL donnée sur stderr, code 1, stdout vide.
10. `armadai` n'ouvre aucune connexion réseau, `--open` compris : l'exécution de `graph` (tous formats, vues et `--open` avec la fausse commande d'ouverture) réussit sans réseau, et le seul processus lancé par `--open` est la commande d'ouverture du système ; seul le navigateur contacte mermaid.live. De plus, `cargo build -p armadai-model` et `-p armadai-core` passent, `armadai-model` n'a aucune dépendance nouvelle, et `cargo audit` passe avec les dépendances ajoutées.
11. Les erreurs actuelles sont inchangées : flow inconnu (liste des flows), projet sans flow, hors projet (`armadai init --project`), projet legacy (`armadai migrate`), projet invalide : code 1, stdout vide.
12. Essai réel avec Dimitri : il lance chaque forme (flow ordonné, `--view team`, `--format ascii`, `--open`) sur le dépôt et valide la lisibilité.

Douze critères, un seul lot (décision de Dimitri) : si le plan juge le lot trop gros, il le dit à `lead` plutôt que de couper de lui-même.

## 6. Risques

- **Tests de référence modifiés.** L'ordre des nœuds change l'octet près de `REVIEW`/`SHIP` (`graph_cli.rs`), des goldens de `mermaid.rs` et de deux `contains` de `new_cli.rs`. Les relire ligne à ligne contre cette spec, pas les régénérer en bloc.
- **Dépendances ajoutées** (crate ASCII, deflate, base64). Surface d'audit (`cargo audit`), poids de compilation, licence, code non portable. Se voit : `cargo audit` rouge, ou `cargo build -p armadai-model` qui tire une dépendance. Parade : tout dans le crate binaire, rien dans `armadai-model`.
- **Cycles et crate ASCII.** Nos flows ont des cycles (`publish → validate`, un rejet qui revient en arrière). Si le crate refuse un cycle (erreur, panique ou boucle), la sortie ne doit être ni vide ni cassée : repli défini au critère 6 (dessin de l'ensemble sans les liens de retour, ces liens écrits en liste dessous, code 0). Se voit : un test sur `plan-build-verify` et `review-loop`. Si aucun crate ne tient ce repli proprement, le plan le dit à `lead` avant de coder.
- **Données hors de la machine avec `--open`.** Les noms d'étapes et d'agents passent dans l'URL ouverte dans le navigateur, vers un site tiers. Parade : opt-in explicite, mention dans la doc. Le lien de mermaid.live peut changer de format : le critère 9 teste notre encodage, pas le site ; seul l'essai réel (critère 12) montre que la page s'ouvre.
- **Commande d'ouverture selon le système.** `open`, `xdg-open`, `start` : seul macOS est mesurable ici ; les autres se vérifient par la fausse commande du critère 9, non par un navigateur.
- **Mermaid : l'ordre de déclaration ne fixe pas la mise en page.** Mermaid place les nœuds selon les arêtes ; l'ordre des déclarations n'agit que sur l'ordre dans un même niveau. Le bénéfice visible de l'objectif 1 est donc plus fort en DOT/ASCII qu'en Mermaid. Se voit à l'essai réel (critère 12).
- **Seconde copie de la logique du gate.** Si la vue équipe recalcule les délégations, elle dérive du gate au premier changement de politique. Parade : critère 8 et source unique.
- **ASCII : lisibilité.** Un flow à beaucoup d'arêtes croisées se rend mal même avec un crate ; le repli par liste (critère 6) évite un dessin faux.
- **Étendue.** Quatre améliorations dans un seul lot (décision de Dimitri) avant #455 : risque de retard. Se voit au plan (nombre de tâches) ; la découpe se redemande alors à Dimitri.
- **Cas limites du parcours.** Flow vide (sortie actuelle : un `flowchart TD` nu, test `an_empty_flow_is_a_bare_flowchart`), lien vers étape inconnue (déjà dessiné avec un nœud fantôme), cycle, étape sans `start` déclaré : chacun à couvrir par un test.

## Questions ouvertes

Tranchées par Dimitri le 2026-11-02 : Q1 rendu direct = `--open` mermaid.live ; Q2 ASCII = boîtes via un crate ; Q3 vue équipe = `--view flows|team` ; Q4 ordre des nœuds changé, assumé ; Q5 un seul lot.

Ouvertes (pour le cadre, puis le plan ; non tranchées ici) :
- **Choix du crate ASCII.** Critères : licence compatible, maintenu, passe `cargo audit`, comportement sur les cycles (voir risque et critère 6), crate d'accueil (binaire si dépendances lourdes ; `armadai-model` reste pur). À décider par le cadre/le plan, ou par une mesure (essai de deux ou trois candidats sur `plan-build-verify` et `review-loop`).
- **Crates de compression et base64** pour `--open`, et choix serde `pako` ou `base64` sans compression (longueur d'URL).
- **Combinaisons de flags** : formats permis avec `--view team`, et `--open` avec un format autre que Mermaid (refus ou ignoré).
- **Format exact du lien mermaid.live** : chemin de la page, contenu minimal de `mermaid`, remplissage du base64 (voir 4.3, partie « non vérifié »).
- **Emplacement de la logique de délégation** (déplacer `allowed_targets` dans `armadai-model` ou fonction commune) : au plan.
