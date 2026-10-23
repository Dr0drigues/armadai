# Étape 4, lot B2 : le pack « concept-to-code »

## 1. Intention

Dimitri veut un pack qui mène une idée vague jusqu'à du code propre, en parlant avec l'utilisateur à chaque décision qui lui revient : affiner le besoin, anticiper les normes, outils, conventions et la sécurité, proposer des maquettes et une architecture qu'on valide visuellement, coder ou accompagner l'utilisateur qui code, vérifier, itérer. La simulation du 2026-10-22 (une app de workspace : projets, tableaux, calculatrice, todo) l'a conforté. B2 assemble ce que les lots précédents ont livré : modèles d'agents (B0), commandes et prises (B1), délégation `strict` compilée vers Claude et OpenCode (lot A), consigne `STATE:` et `flow status`.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs (MVP) :
1. Un 8e pack natif `concept-to-code` (`crates/armadai-core/starters/concept-to-code/`, `format: 1`), installable par `armadai init --pack concept-to-code [--link]`, cibles `claude` et `opencode`, coordinateur `lead`, `policy.delegation: strict`, `free_agents: [explore]`.
2. L'équipe et les flows du §4, les commandes du §5, les skills et prises du §6, le rangement des documents du §7.
3. Valide, lie sans erreur vers les deux cibles, étalons Claude et OpenCode, et passe les tests existants qui couvrent tous les packs natifs.
4. Doc `docs/wiki/starter-packs.md` (le pack, ses commandes, ses prises, comment brancher des normes), CHANGELOG.

Non-objectifs du MVP (reportés) :
- Packs de normes à brancher (lot C) : le pack déclare les prises, n'en livre aucune hormis la skill de projet écrite par `/init-standards`.
- Codex, Copilot (adaptateurs pas encore fidèles).
- Maquettes interactives au-delà d'un fichier HTML autonome par itération ; visualisation de la doc ou du backlog (lot D).
- Le linter d'agents.

Anti-objectifs :
- Un agent qui choisit seul une convention que le dépôt ne fixe pas (il demande, puis l'inscrit dans le cadre).
- Un sous-agent qui pose une question à l'utilisateur sous Claude (impossible : il rend ses questions au coordinateur, qui les pose).
- Des documents humains mélangés aux documents de travail des agents.

## 3. Contraintes

- Sous Claude, un sous-agent n'a jamais l'outil de question (doc officielle) : toute question passe par le fil principal (`lead` ou une commande). Sous OpenCode, même règle par cohérence.
- Les capacités ne savent pas limiter l'écriture à un dossier : un agent qui doit écrire `.armadai/docs/` a l'écriture autorisée ; la règle de rangement est dans son prompt.
- `/design` est une skill intégrée de Claude Code : aucune commande ni skill du pack ne s'appelle `design`, `init`, `review` ou un autre nom de `BUILTIN_COMMANDS`.
- Une commande ne peut pas porter le nom d'une skill (B1) : `/brief` charge `briefing`, `/mockup` et `/iterate` chargent `mockups`, `/init-standards` charge `standards-setup`.
- Les étapes à états reçoivent la consigne `STATE:` générée ; les prompts du pack n'en ajoutent pas d'autre.

## 4. Équipe et flows

| Agent | Origine | Rôle | Capacités, skills, prises |
|---|---|---|---|
| `lead` | écrit | coordinateur : parle avec l'utilisateur, choisit `deliver` ou `guide`, pose les questions que les autres lui rendent, synthétise ; ne code jamais | `write: deny` |
| `analyst` | écrit | brief : problème, utilisateurs, critères de réussite, non-objectifs ; rend ses questions à `lead` une par une | skill `briefing` |
| `scout` | écrit | cadre : stack, conventions en place, outils, normes, sécurité, infra ; sépare décidé / à décider ; rend les décisions ouvertes à `lead` | skills `cadre`, `standards-setup` ; prises `code-standards`, `security`, `infra-docs` ; `shell: deny` |
| `designer` | écrit | maquettes (3 variantes) et itérations ; rend les variantes à `lead` qui fait choisir | skill `mockups` ; prise `ux-guidelines` |
| `architect` | écrit | architecture, interfaces, compromis ; diagrammes systématiques quand une validation visuelle est utile (Mermaid dans la doc, rendu HTML autonome) | skill `diagrams` ; prise `infra-docs` ; `shell: deny` |
| `implementer` | écrit | code par petites étapes testées ; demande (via `lead`) avant toute convention non fixée, puis l'inscrit dans le cadre | skill `cadre` ; prises `code-standards`, `security` ; `shell: ask` |
| `coach` | écrit | flow `guide` : explique, propose le code à taper, relit ce que l'utilisateur a écrit ; n'écrit jamais | skill `cadre` ; prises `code-standards`, `security` ; `write: deny` |
| `guardian` | instance de `checker` | gardien du cadre : conventions, sécurité, périmètre | prises `code-standards`, `security` |
| `validator` | instance de `checker` | tests et critères du brief | `shell: ask` |

Modèle `templates/checker.md` : vérificateur qui lit, n'écrit pas (`write: deny`), juge `{{subject}}` selon `{{criteria}}`, explique chaque rejet en une ligne actionnable. Les états viennent du flow (consigne générée).

Flow `deliver` (start `refine`) : `refine(analyst) → frame(scout) → mockup(designer) → design(architect) → build(implementer) → guard(guardian: approved|rejected) → validate(validator: approved|rejected|scope_changed)` ; `guard → build` si `rejected` ; `validate → build` si `rejected` ; `validate → refine` si `scope_changed`.
Flow `guide` : identique, `build` tenu par `coach`.

Les deux flows partagent leur entrée : `flow status` les affiche `deliver | guide` jusqu'à `build` (comportement du lot « flow status »).

## 5. Commandes

| Commande | Corps (charge une skill, tourne dans le fil principal) |
|---|---|
| `/brief <idée>` | affiner l'idée avec la skill `briefing`, en posant les questions une par une |
| `/mockup <écran ou fonctionnalité>` | skill `mockups` : 3 variantes, choix par l'outil de question, décision notée |
| `/iterate <retour>` | skill `mockups` : repartir de la variante choisie, 3 nouvelles variantes `v<n+1>` ; la même skill se déclenche aussi sur un retour en langage naturel (« refais la B plus aérée ») |
| `/init-standards` | skill `standards-setup` : équivalent de `/init` pour les normes et la doc (§6) |

Toutes avec `argument_hint` sauf `/init-standards`.

## 6. Skills et prises

- `briefing` : forme du brief (`.armadai/docs/brief.md`) et des critères de réussite.
- `cadre` : lire et compléter `.armadai/docs/cadre.md` (décidé / à décider / décisions de l'utilisateur datées) ; règle « demander avant d'adopter une convention non fixée ».
- `mockups` : écrire `mockups/<sujet>/v<n>.html` (3 variantes A, B, C en onglets, aucune dépendance, s'ouvre dans un navigateur), faire choisir (choix multiples permis : « l'en-tête de A + la liste de C »), noter le choix et sa raison dans `.armadai/docs/design-decisions.md` ; itérer à partir de la variante retenue.
- `diagrams` : quand et comment produire un diagramme (Mermaid dans `docs/technical/…`, rendu HTML autonome pour validation).
- `standards-setup` : analyser le dépôt (langages, manifestes, linters et formateurs configurés, CI, docs existantes, infra), proposer les conventions détectées, faire trancher celles que le dépôt ne fixe pas (questions à choix), écrire `.armadai/skills/project-standards/SKILL.md` avec `metadata: { armadai-slot: "code-standards, security" }` (plus `infra-docs` si l'infra est décrite), créer l'arborescence `docs/functional/` et `docs/technical/`, puis lancer `armadai link` (demande d'accord pour le shell).
- Prises déclarées par les agents : `code-standards`, `security`, `infra-docs`, `ux-guidelines`. Vides à l'installation ; remplies par `/init-standards` ou par un pack de normes branché (lot C), ou par des skills de l'utilisateur portant `armadai-slot`.

## 7. Rangement des documents (décision de Dimitri)

- Documents de travail des agents, peu utiles à l'humain : `.armadai/docs/` (`brief.md`, `cadre.md`, `design-decisions.md`). Le chargeur d'ArmadAI ignore ce dossier.
- Documents destinés aux humains : `docs/functional/<domaine>/…` et `docs/technical/{architecture,infra,api,…}/<sujet>.md`, rangés en sous-dossiers cohérents.
- Maquettes : `mockups/<sujet>/v<n>.html`.

## 8. Laissé au constructeur

Le texte exact des prompts (en anglais, comme `armadai-dev` ; les agents s'adressent à l'utilisateur dans sa langue), la description de chaque agent et de chaque skill (déclencheurs), les critères du modèle `checker` pour chaque instance, le corps exact des commandes.

## 9. Critères d'acceptation

1. Barrière complète verte ; tests existants sur les packs natifs verts (installation, validation, link, liste blanche des clés OpenCode) ; étalons `init_pack` et `init_pack_opencode` du pack relus.
2. `armadai validate` sans erreur ni avertissement sur le pack installé ; aucun nom de commande intégré.
3. Essai réel par Dimitri dans Claude Code (puis OpenCode si le temps le permet) sur le scénario de la simulation : `/init-standards` sur un dépôt vide, `/brief`, une maquette avec `/mockup` puis `/iterate`, le flow `deliver` jusqu'à un rejet de `guardian` et sa reprise, suivi par `armadai flow status --watch`.

## 10. Risques

- Le flow complet est long et coûteux à l'essai : l'essai réel peut s'arrêter après la première boucle.
- Le respect des règles (questions rendues à `lead`, rangement des docs, demande avant convention) dépend du modèle : l'essai le montrera ; le linter d'agents pourra en vérifier une partie plus tard.
- `scope_changed → refine` est la boucle la moins sûre à suivre par un modèle (simulation).
