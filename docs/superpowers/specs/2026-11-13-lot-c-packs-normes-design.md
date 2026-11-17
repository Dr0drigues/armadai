# Lot C : les packs de normes

## 1. Intention

Les agents d'un pack d'équipe déclarent des prises (`slots: [code-standards]`) ; une skill les remplit par `metadata: { armadai-slot: … }`. Le mécanisme existe depuis B1 (étape 4), mais rien ne permet aujourd'hui de **brancher proprement un jeu de normes partagé** (celles d'une entreprise, d'une stack) à côté du pack d'équipe : un projet référence un seul pack, et la seule voie pour ajouter des skills venues d'ailleurs est de les copier à la main ou d'installer un second pack par copie, sans mise à jour possible. Dimitri a validé le 2026-10-27 « Packs de normes (lot C) » au périmètre v1, dernier lot avant la clôture (#260 : guide de migration, CHANGELOG, crates.io, tag v1.0.0). Codex et Copilot restent hors v1. Décisions de Dimitri du 2026-11-13 : branchement par une clé `standards:` (« Clé standards: (Recommandé) »), collision d'id refusée à la validation (« Erreur de validation (Recommandé) »), le lot livre le mécanisme et `rust-standards` (« Mécanisme + rust-standards (Recommandé) »). Le reste découle des specs B1, B2 et dogfood et des mesures du §4.1.

## 2. Objectifs, non-objectifs, anti-objectifs

Objectifs (sur la recommandation de chaque question) :
1. Définir ce qu'est un **pack de normes** : un pack natif (`format: 1`) qui ne contient que des skills, chacune branchée sur une ou plusieurs prises.
2. Un projet branche **un ou plusieurs** packs de normes à côté de son pack d'équipe (ou sans pack d'équipe), par référence versionnée et verrouillée, avec les mêmes garanties que `pack:` (lock, cache local, `link` sans réseau, pin par checkout).
3. Règle claire quand plusieurs sources remplissent la même prise ou portent le même id de skill.
4. `validate`, `link` (rapport) et `pack status` montrent ce qui est branché, de quel pack vient chaque skill de prise, et les prises que personne ne remplit ou que personne ne consomme.
5. Documentation (`docs/wiki/starter-packs.md`, `project-format.md`, `link.md`), CHANGELOG.

Non-objectifs :
- Un pack de normes qui apporte des agents, flows, commandes, automations, templates ou une `project.yaml` : refusé (décision B1 : « une prise ne contient que des skills »).
- Des normes privées d'une entreprise : hors dépôt public.
- Un second mode « copie » nouveau : `init --pack <normes> --force` continue de fonctionner comme aujourd'hui, sans promesse de mise à jour.
- Codex, Copilot.
- Changer la résolution des prises (union triée par id, B1) : elle est conservée.

Anti-objectifs :
- Une norme branchée qui en masque une autre en silence.
- Un `link` qui touche au réseau.
- Casser un projet à `pack:` seul : sans packs de normes, la sortie de `link`, `validate`, `pack status` ne change pas d'un octet.

## 3. Contraintes

- Manifeste `link-manifest.yaml` et `pack.lock` restent en `version`/`format: 1` : champs optionnels seulement, un test lit un lock écrit par rc.13 (`armadai-rules`).
- `armadai-model` reste pur ; un code de diagnostic nouveau entre dans la table de `docs/wiki/project-format.md`, en kebab-case, jamais renommé.
- Le texte des lignes nouvelles de sortie est proposé par le plan et validé par Dimitri à la relecture du plan.
- Rien de privé dans le dépôt : packs d'exemple génériques.
- `--json` du rapport de `link` : on ajoute des lignes, jamais de champ renommé.

## 4. Conception

### 4.1 État mesuré (rc.13, master `04653f3`)

- `project.yaml` : `pack:` est un `Option<PackRef>` (`crates/armadai-model/src/types.rs:85,100`) : **un seul** pack par projet, trois champs `name`, `source`, `version`.
- `.armadai/pack.lock` : un seul pack (`pack`, `source`, `resolved`), `crates/armadai/src/pack/lock.rs:12-31` ; le pin par checkout est la même forme.
- La vue effective (`pack/effective.rs`) : `pack_ref`, `LockedPack`, `open_in` supposent un pack ; la fusion `armadai-model/src/pack.rs::overlay` pose les fichiers du pack puis ceux du projet **au même chemin** (le projet gagne, sans message) ; `project.yaml` est fusionné clé par clé.
- Liste blanche de l'installation (`starter-packs.md`) : `project.yaml`, `agents/`, `templates/`, `commands/`, `skills/**`, `flows/`, `automations/`.
- En mode référence, `overlay` prend **tous** les fichiers du pack : la liste blanche ne vaut que pour la copie (`init --pack`, `ignored-pack-file`, `cli/native_pack.rs`). Le contrôle « que des skills » d'un pack de normes est donc nouveau, à poser dans le validateur. Le seul code de prise existant est `bad-slot` ; aucun code pour une prise vide ou inutilisée (rapport de `link` seulement, `compile/mod.rs::slot_lines`). Aucun test existant ne couvre deux sources sur une même prise ni une collision d'id entre pack et projet (mesure d'`Explore`).
- Prises : l'agent déclare `slots:`, la skill `armadai-slot` (virgules) ; l'agent reçoit ses `skills:` puis les skills de ses prises triées par id, sans doublon (`project-format.md`, §Slots). Plusieurs skills sur une même prise : **union**, pas de conflit. Prise vide : `agent <id> slot <s>: not-compiled (no skill fills it)`, jamais une erreur. Skill sur une prise qu'aucun agent ne déclare : aucun diagnostic.
- Un pack d'équipe peut déjà embarquer des skills de normes (`concept-to-code`, `armadai-dev`) : elles sont dans sa vue et remplissent ses prises.
- Deux packs installés par copie dans un projet : `pack update` refuse (`starter-packs.md`, Limits).

Ces points sont tirés des fichiers cités (`types.rs`, `lock.rs`, `effective.rs`, `pack.rs`, wiki), lus dans le worktree ; le détail des diagnostics de prises et de `pack status` est à vérifier par le plan.

### 4.2 Le pack de normes

Un pack natif dont `pack.yaml` porte `kind: standards` (clé optionnelle, valeur par défaut `team`, absente = pack d'équipe : les packs existants ne changent pas). Contenu admis : `pack.yaml`, `skills/<id>/…`, README ; tout autre élément de la liste blanche est une erreur de validation du pack. Chaque skill porte `metadata: { armadai-slot: … }` : une skill sans prise est un avertissement (elle ne sera jamais chargée par une prise). `armadai pack new <nom> --kind standards` écrit le squelette ; `armadai validate <dir>` applique ces règles.

### 4.3 Brancher (décidé : liste `standards:` référencée)

`project.yaml` gagne une clé optionnelle `standards:` : liste d'objets de la même forme que `pack:` (`name`, `source`, `version`). `pack:` reste un seul pack d'équipe. `pack.lock` gagne un champ optionnel `standards:` (liste de `{pack, source, resolved}`), le lock d'un projet sans `standards:` reste identique octet pour octet. `pack add <nom> --standards` ajoute une entrée (clé, lock, `link`), `pack update` avance tous les locks de la même façon que pour `pack:`, `pack pin` fige l'ensemble. La vue effective superpose : pack d'équipe, puis chaque pack de normes dans l'ordre déclaré (skills seulement), puis les fichiers du projet.

### 4.4 Deux sources, une prise ; un même id

- **Même prise, skills d'ids différents** (cas normal : des normes de code d'un pack et des règles de sécurité d'un autre) : union, ordre trié par id, comme aujourd'hui. Aucune erreur.
- **Même id de skill** (décidé) : entre deux sources de pack (normes contre normes, normes contre pack d'équipe), c'est une **erreur de validation** `duplicate-standards-skill`, dont le message nomme les deux packs ; `link` s'arrête avant d'écrire. Le projet garde la main : une skill de son `.armadai/skills/<id>/` du même id remplace celle du pack comme aujourd'hui (règle d'overlay inchangée), et lève l'erreur. Le rapport de `link` dit quelle source a fourni chaque skill de prise.

### 4.5 Ce que voient les commandes

Les textes exacts sont proposés au plan ; les informations sont :
- `validate` : erreurs sur un contenu non-skill d'un pack de normes, avertissement skill sans prise, erreur sur un id en collision (selon Q2), erreur si une entrée de `standards:` n'est pas un pack `kind: standards`. Codes proposés : `standards-foreign-file`, `standards-skill-no-slot`, `duplicate-standards-skill`, `standards-not-standards`, `standards-empty-source`, `standards-empty-version` (famille `standards-*`).
- `link` : une ligne `standards <nom>: compiled (<version> (<id12>), from the cache)` par pack de normes, une ligne par skill d'un pack de normes qui remplit une prise qu'aucun agent ne déclare (`not-compiled`, la raison nomme la prise), et la ligne de prise existante précise la source de chaque skill. La ligne `agent <id> slot <s>: not-compiled (no skill fills it)` ne change pas.
- `pack status` : une section `Standards <nom>` par pack de normes (source, plage, verrou, cache), sur le modèle de `Pack <nom> (reference)` ; sous un pin, la ligne `pinned (worktree)` couvre chaque entrée.
- `list` / `inspect` : la section `Slots:` indique le pack d'origine de chaque skill.

### 4.6 Ce que le lot livre

Décidé : le mécanisme et un pack générique embarqué, `rust-standards` (`kind: standards`, skills sur `code-standards` et `security`), dans `crates/armadai-core/starters/rust-standards/`, avec ses étalons `init_pack*`. Il s'installe par `init --pack rust-standards` (copie, sans mise à jour) : un pack embarqué n'est pas une source versionnée, donc `standards:` ne le référence pas. Pour la référence, les tests utilisent un faux registre local portant un pack de normes (règle `StarterFetcher`). Aucun contenu privé.

### 4.7 Crates touchés

- `armadai-model` : `PackRef` réutilisé pour `standards:`, schéma JSON régénéré, validateur et codes, `overlay` étendu à N couches de skills, `kind` de `pack.yaml`.
- `crates/armadai` : `pack/lock.rs`, `pack/effective.rs`, `cli/pack.rs` (`add --standards`, `status`, `update`, `pin`, `new --kind`), rapport de `link`, `list`, `inspect`, wiki, étalons.
- `armadai-core` : seulement les packs embarqués éventuels (Q3).

## 5. Critères d'acceptation

1. Barrière complète verte ; un lock `pack.lock` écrit par rc.13 est lu sans changement, et un projet sans `standards:` produit la même sortie de `link`, `validate` et `pack status` qu'avant (étalons inchangés).
2. `armadai pack new demo-standards --kind standards` écrit un pack que `armadai validate <dir>` accepte ; y ajouter un agent, un flow ou une commande fait échouer `validate <dir>` avec `standards-foreign-file`.
3. `armadai pack add demo-standards --standards` dans un projet à `pack:` ajoute l'entrée à `standards:`, écrit le lock, lie ; `armadai link` rapporte `standards demo-standards: compiled (…)`.
4. Avec deux packs de normes branchés dont les skills remplissent la même prise sous des ids différents, l'agent qui déclare la prise reçoit les deux skills (ordre par id) ; `link` nomme le pack d'origine de chacune.
5. Deux packs (normes contre normes, ou normes contre pack d'équipe) portant le même id de skill : `armadai validate` sort en erreur `duplicate-standards-skill` nommant les deux packs, `link` n'écrit rien ; en ajoutant au projet une skill du même id, l'erreur disparaît et `link` compile la skill du projet.
6. Une prise que personne ne remplit garde sa ligne `not-compiled (no skill fills it)` ; une skill d'un pack de normes sur une prise qu'aucun agent ne déclare donne sa ligne `not-compiled` nommant la prise.
7. `armadai pack status` montre chaque pack de normes (source, plage, verrou, présence au cache) ; `pack update` avance les locks sans réseau pour `link` ; `pack pin` puis `pack update` est refusé avant tout accès réseau.
8. `link` sans le cache d'un pack de normes s'arrête avec `… is not in the local cache: run armadai pack sync`, rien d'écrit.
9. `armadai unlink` après un `link` avec packs de normes revient à l'état initial (manifeste).
10. `armadai init --pack rust-standards` dans un projet sur `concept-to-code` installe ses skills sans erreur ni avertissement de `validate` ; essai réel avec Dimitri : après `link`, `implementer` charge la skill de `rust-standards` et la ligne `agent implementer slot code-standards` la liste, sous Claude Code puis sous OpenCode ; puis le même essai avec un pack de normes branché par `standards:` (pack d'une source locale d'essai de Dimitri).

## 6. Risques

- La vue effective suppose un pack partout (`pack_ref`, `LockedPack`, le gate, le hook `grant`, `pack test`) : passer à N packs touche chacun de ces lecteurs ; un oubli donne un pack de normes ignoré par le gate seul. Tout lecteur du lock passe par `pack::lock::read` : à vérifier site par site au plan.
- Le format du lock : le champ optionnel `standards:` ajouté à un `deny_unknown_fields` rend un lock nouveau illisible par un binaire plus ancien ; acceptable (le projet est refusé, pas corrompu), à dire dans le CHANGELOG.
- Les limites existantes de la copie (deux packs par copie, `pack update` refuse) restent vraies : le lot ne les corrige pas.
- Calendrier : c'est le dernier lot avant v1 ; un lot qui grossit retarde la clôture. Ordre de coupe si besoin : le modèle et la validation, puis la vue effective et `link`, puis `pack add --standards`/`status`/`pin`, puis `rust-standards`.

## Questions ouvertes

Aucune question produit en attente. Points laissés au plan : les textes exacts des lignes de sortie et des messages (validés par Dimitri à la relecture du plan), le nom définitif des codes `standards-*`, et la liste exacte des lecteurs de lock à étendre (gate, hook `grant`, `pack test`, `list`, `inspect`).
