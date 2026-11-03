# Cadre du lot « gestion des packs »

Date : 2026-11-03 · Étape `frame` du flow `lot` · Spec : `docs/superpowers/specs/2026-11-03-pack-management-design.md` (décisions A-L de Dimitri, non rediscutées). Branche `feat/pack-management`, sur master `2ee093e`.

Lu pour ce cadre : la spec, `armadai-rules` (`.armadai/skills/armadai-rules/SKILL.md`), `CLAUDE.md`, `.claude/CLAUDE.md`, `.github/workflows/ci.yml`, `crates/armadai-model/Cargo.toml` et `agent_templates.rs`, `crates/armadai/src/linker/manifest.rs`, `crates/armadai/src/starters_registry/mod.rs`, `crates/armadai-core/src/{template,project_registry,registries}.rs`, `crates/armadai/src/cli/{mod,models}.rs`, `.armadai/.gitignore`, `docs/wiki/{project-format,starter-packs}.md`. Je n'ai pas Bash : aucune commande lancée, aucune recherche `grep` ; ce qui suppose une recherche dans tout le dépôt est marqué « à vérifier ».

## 1. Règles qui s'appliquent (avec source)

Barrière et CI
- Barrière unique : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model` ; la CI y ajoute `cargo build --release` et Conventional Commits (`.github/workflows/ci.yml`, `RUSTFLAGS=-D warnings`). Code retour lu, jamais de tube. Source : `armadai-rules`, Barrière ; CI.
- `armadai-core` et `armadai-model` compilent seuls ; `armadai-model` reste pur : `serde`, `serde-saphyr`, `serde_json`, `thiserror`, `schemars`, rien d'autre (vérifié : `crates/armadai-model/Cargo.toml`, ni clap, ni tokio, ni réseau). Source : `CLAUDE.md`, Where things live.
- `frozen/armadai-audit/` : à recompiler seulement si un crate dont il dépend change (il vendore le lecteur du cache de modèles, pas `starters_registry`). Source : `CLAUDE.md`, Architecture.

Point 1 de lead, réseau
- `link` ne touche jamais le réseau ; seuls `init --pack` (sur un échec local), `skills sync`/`add` et `update` le font. Compatible avec « link compile depuis le cache ». Conséquence : `pack sync` et `pack update` (qui rafraîchit) rejoignent la liste des commandes réseau, et `CLAUDE.md`, Architecture, doit être mis à jour par `lead` (je n'écris que `armadai-rules`). Source : `CLAUDE.md`, Architecture.
- Les sources git sont clonées par `git clone --depth 1` et mises à jour par `git pull --ff-only` (vérifié : `starters_registry/mod.rs`, `GitFetcher`). Voir question 2.

Point 2, pureté de `armadai-model`
- Aucune règle n'interdit nommément `std::env` dans le crate ; c'est la liste fermée de dépendances qui fixe la pureté. Le loader lit déjà des fichiers sous un `root`. Ce qui est sûr : le crate ne doit pas connaître `~/.config/armadai/` ni le cache des sources (c'est le binaire/`armadai-core` qui les résolvent). Voir question 4.
- Deux moteurs de `{{var}}` existent déjà : `armadai-core/src/template.rs` (utilisé par `new`) et `armadai-model/src/agent_templates.rs` (templates d'agents rendus au chargement, `vars:`, `\{{` littéral, valeur jamais rerendue, `render_value` récursif sur les seules `String` JSON). La spec (section 4) dit « probablement `template.rs` du core » : le modèle ne peut pas dépendre du core, donc la substitution qui touche `native:` est celle de `agent_templates.rs` ou une nouvelle fonction pure du modèle. À fixer au plan. Voir question 3.

Point 3, manifeste
- `link-manifest.yaml` reste `version: 1` ; tout ajout est un champ optionnel `#[serde(default, skip_serializing_if)]` ; un test lit un manifeste de la version précédente. Source : `armadai-rules`, Diagnostics, sorties et manifeste (Dimitri, 2026-10-27).
- Fait mesuré à la lecture : `ManifestEntry` décrit une sortie compilée (`path`, `produced_by`, `outcome`, `digest` de ce que `link` a écrit) ; `ProducedByKind` ajouter une valeur rend le manifeste illisible pour un binaire plus ancien (commentaire de `manifest.rs`, `Lookup::Fallback`). Voir question 5.
- `.armadai/link-manifest.yaml` est dans `.armadai/.gitignore` : il ne voyage pas avec le dépôt. Voir question 5.

Point 4, schémas et codes
- Si `project.yaml` gagne une clé (référence de pack), les schémas `schemas/v1/` sont regénérés par `cargo run -p armadai-model --example gen_schemas`. Source : `CLAUDE.md`, Where things live.
- Codes de diagnostic : kebab-case, préfixe de famille, sans verbe d'action, jamais renommés, chaque code dans la table de `docs/wiki/project-format.md`. Source : `armadai-rules` (Dimitri, 2026-10-27).

Point 5, textes et nommage
- Le texte de toute ligne de sortie nouvelle (rapport, diagnostic, erreur) est proposé par le plan et validé par Dimitri à sa relecture du plan ; golden ensuite relus ligne à ligne contre la spec. Source : `armadai-rules` (Dimitri, 2026-10-27). Cela couvre les messages de `pack status`, `pack sync`, `pack update`, les avertissements de variable manquante et de fichier gardé, et les reprises dans `list`/`inspect`/`link`.
- Nommage : `update` est pris (`cli/update.rs`, auto-mise à jour) et `models update` existe avec un autre sens ; la spec renvoie le nommage des commandes, des fichiers de variables et du fichier de registre au plan, validé par Dimitri à la relecture du plan (spec, « Questions ouvertes »). Pas de question de plus ici.
- `pack new` : même contrat que `new agent`/`new flow` (écriture gardée, annulation sur nouvelle erreur, `plan_*` pures derrière l'assistant, mode non interactif équivalent, règle T9 de l'étape 3 citée par la spec). Source : `CLAUDE.md`, Where things live (`cli/new*.rs`) ; spec critère 11.

Point 6, écritures hors projet et tests
- Tout essai du binaire isole `HOME` et `ARMADAI_CONFIG_DIR`. Source : `armadai-rules`, Environnement.
- `armadai_core::test_support::env_lock()` : non réentrant, deux gardes sur un thread bloquent ; la couche « variable d'environnement » des variables et le cache/registre sous `config_dir()` sont des tests qui modifient l'environnement, donc sous ce verrou. Valeur de référence lue sous le verrou, 5 à 8 passes. Source : `armadai-rules`, Tests et mutations.

Tests
- Une mutation par test, nommée ; fixtures à valeurs distinctes ; `contains` suspect ; `--bin armadai` ou `--test <nom>`, `--no-fail-fast`. Source : `armadai-rules`, Tests et mutations. Les risques R3 à R7 de la spec (valeurs JSON non textuelles dans `native:`, matrice des trois couches, cache figé entre deux `link`, fusion de listes, entrée stale) sont chacun une mutation à nommer.

Git, PR, rapport
- Conventional Commits en anglais, un type par sujet ; PR en français ; ouvrir la PR et s'arrêter sauf brief contraire ; revue indépendante avant fusion ; 6 vérifications CI. Source : `armadai-rules`, Git et PR.
- Mesure, pas raisonnement ; rapporter honnêtement ce qui est laissé (R1 : 14 critères en un lot). Source : `armadai-rules`, Rapport ; `.claude/CLAUDE.md`.

## 2. Habitudes observées, non enregistrées

- Test réseau : l'interface `StarterFetcher` (`starters_registry/mod.rs:45`) permet d'injecter un faux fetcher ; la spec demande un faux registre local, sans accès réseau réel. Non fixé par une règle de `armadai-rules` (à vérifier : comment les tests actuels de `starters_registry` s'isolent du réseau). Voir question 7.
- « Rien de privé dans le dépôt public » (contenu d'un pack privé, chemins absolus propres à une machine, jetons) : la mémoire du coordinateur le consigne, mais ce n'était pas une règle de `armadai-rules` (elle l'est devenue, voir 3 bis). Recherche dans le reste du dépôt (docs, fixtures, wiki) : non faite, pas d'outil de recherche ici ; à faire par le validator. Voir question 7.
- Documentation du wiki mise à jour avec le code (`docs/wiki/starter-packs.md`, `project-format.md`, probablement `link.md`) : pratique constatée, pas de règle écrite ; la spec la demande elle-même pour le dépôt à plusieurs packs (non-objectif « à documenter dans le wiki »). Je la traite comme livrable du lot, pas comme règle.

## 3. Contradictions et conflits à signaler

- Spec, critère 12, contre la conception à trois couches. Le critère dit que la variable est « résolue à l'installation ou à la mise à jour » et que « le `project.yaml` et le `.claude/settings.json` écrits portent la valeur résolue ». Or la couche 1 est un fichier versionné partagé par l'équipe et la couche 2 un fichier local propre à la machine : si la valeur résolue est écrite dans `.armadai/project.yaml` (versionné), un chemin de machine part dans git ; et en mode référence le `project.yaml` du projet n'est pas celui du pack. Voir question 1.
- Spec (section 3, 4) contre le code : « `crates/armadai-core/src/template.rs` » comme base de substitution, alors que `native:` et les agents sont rendus par `armadai-model` (pur, sans dépendance au core). Voir question 3.
- `armadai_core::project_registry` existe déjà (`~/.config/armadai/projects.json`, un projet enregistré à chaque `armadai link`, `prune_stale` qui retire silencieusement les projets sans config) ; la spec (décision L) crée un registre des référents « jamais retiré en silence ». Voir question 6.
- Rien dans `armadai-rules` n'est contredit par `CLAUDE.md` ou `.claude/CLAUDE.md`.
- Mises à jour de `CLAUDE.md` à faire par `lead` (hors de mon périmètre) : la liste des commandes réseau (Architecture) ; la ligne « `.armadai/` is versioned except `link-manifest.yaml`, `.migrated/` and `state/` » si un fichier local de variables est ajouté à `.armadai/.gitignore`.

## 3 bis. Décisions de Dimitri sur ce cadre (2026-11-03, mot pour mot)

Elles s'ajoutent aux décisions A-L de la spec ; les questions de la section 4 ci-dessous sont closes.

- Q1, moment de la substitution : « Au link seulement (Recommandé) ». `.armadai/` garde `{{var.nom}}` ; `link` résout et écrit les sorties ; le critère 12 de la spec se reformule sur les sorties compilées (l'analyst met la spec à jour).
- Q2, épingle : « je voudrais qu'on en discute ». Après discussion, à « deux projets d'une même machine sur des versions différentes du même pack ? » : « 1. avec possibilité de placer un latest ou equivalent (idem, si on met @1 ou @^1.3, ce devrait être exploitable) ». Donc : le cache garde un contenu par version utilisée (machine neuve : `pack sync` avant `link`), avec `latest` et des plages semver. Suites : verrou « Fichier verrou versionné (Recommandé) » (plage dans `project.yaml`, version et commit résolus dans un fichier verrou versionné type `.armadai/pack.lock` ; `link` lit le verrou, seul `pack update` l'avance) ; semver « Crate semver (Recommandé) » (dans `crates/armadai` seulement, `armadai-model` reste pur) ; liste des versions : « Les deux. si trop complexes, voir si en sub module c'est plus simple » (l'index de la registry s'il liste des versions, sinon des tags git préfixés par le nom du pack, ex. `<pack>-v1.3.2` ; une archive exige l'index ; repli à évaluer par architect : un dépôt par pack en sous-module, tags sans préfixe). Proposition de `lead`, non encore répondue : enregistrer tag et commit, le commit faisant foi.
- Q3 : « {{var.name}} + ARMADAI_VAR_ ». Syntaxe `{{var.nom}}`, environnement sous le préfixe `ARMADAI_VAR_` ; le scanner du modèle est étendu (le `.` sort de l'id, jamais confondu avec une variable de template).
- Q4 : « Pur dans le modèle (Recommandé) ». Clé de référence, fusion clé par clé et substitution : fonctions pures de `armadai-model` ; `schema_version` reste 1 ; le binaire lit environnement, fichiers, cache et registre.
- Q5 : « Section packs: du manifeste (Recommandé) ». Section optionnelle de premier niveau, v1 conservée.
- Q6 : « Nouveau fichier (Recommandé) » pour le registre des référents, indépendant de `projects.json`.
- Q7 : « Les deux (Recommandé) » : deux règles enregistrées dans `armadai-rules` (aucun réseau réel dans les tests ; rien de privé dans le dépôt public).
- Q8 : « Hors lot (Recommandé) » : `armadai link update` n'est pas dans ce lot.

Points laissés à l'architect : noms du fichier verrou, des fichiers de variables et du registre ; repli sous-module/tags sans préfixe ; obtention de plusieurs versions d'une source git clonée en `--depth 1` (un contenu par version dans le cache).

## 4. Questions pour Dimitri (closes, voir 3 bis)

### Q1. Quand les variables sont-elles substituées : à l'installation (fichiers écrits) ou à la compilation par `link` ?
Bloque : tout le plan des variables, le format du `project.yaml` installé, l'empreinte « inchangé depuis l'installation » du mode copie.
Constat : le critère 12 de la spec parle d'un `project.yaml` écrit avec la valeur résolue ; la couche versionnée et la couche locale décidées ne sont compatibles qu'avec une substitution au moment de compiler.
- Option A (recommandée) : au `link` seulement. `.armadai/` garde le texte `{{var}}` (le pack reste générique, rien de propre à la machine dans git), `link` résout puis écrit `.claude/settings.json`, `opencode.json` et les fichiers d'agents. Le critère 12 est alors reformulé sur les sorties compilées. Le fingerprint du mode copie porte sur le fichier non substitué. Un `validate` doit accepter un `{{var}}` non résolu.
- Option B : à l'installation et à `pack update`. `.armadai/` contient les valeurs résolues (et donc des chemins de machine, dans git si `.armadai/` est versionné). Plus proche de la lettre du critère 12 ; casse l'idée d'un fichier local « propre à la machine ».
- Option C : les deux selon le mode (copie : à l'installation ; référence : au `link`).

### Q2. Épinglage : où vit le contenu d'une épingle (tag ou commit) quand le cache est un clone `--depth 1` ?
Bloque : le mode référence, l'index versionné (J), le critère 2 et le risque R5.
Constat : une source git est clonée `--depth 1` puis `pull --ff-only` : le cache ne contient que la tête ; un tag ou un commit plus ancien n'y est pas, et une archive n'a pas de tags. Décider « épinglé à un tag/commit » ne dit pas où lire le contenu de ce repère.
- Option A (recommandée) : le cache garde un contenu par repère lu : `pack update` récupère le repère demandé (`git fetch --depth 1 origin <tag|commit>`) et en extrait le pack dans un sous-dossier du cache de la source (par repère) ; `link` lit ce sous-dossier ; l'épingle d'une archive est la `version` de l'index.
- Option B : l'épingle est la `version` du nouvel index (pas un repère git), vérifiée contre la tête du cache : `link` refuse si le cache n'est plus à cette version. Simple, mais le projet ne peut pas lire une version plus ancienne que celle du cache.
- Option C : cache git complet (sans `--depth 1`) et `git checkout`/`git archive` par repère. Plus de disque et de temps de clone, et il change le comportement de `GitFetcher`.

### Q3. Syntaxe des variables de pack, face aux variables de template existantes, et nom des variables d'environnement
Bloque : la substitution (objectif 3), le validateur, les noms de fichiers de variables.
Constat : `{{name}}` est déjà la syntaxe des templates d'agents (noms = ids, `missing-template-var` est une erreur, `\{{` littéral). Une variable de pack en `{{name}}` entre en collision dans `templates/<id>.md` : une variable absente du `vars:` de l'instance serait lue comme de template (erreur) alors que le pack veut « avertissement + valeur vide » (Question B). `${NAME}` n'est pas libre : `${CLAUDE_…}` est une erreur dans une commande. Ensuite, la couche environnement : quel nom de variable d'environnement correspond à `{{x}}` ?
- Option A (recommandée) : même `{{name}}`, rendue après les templates, en dehors de leurs variables ; une variable de template porte sur ses `vars:`, le reste va aux trois couches ; environnement lu sous un préfixe, `ARMADAI_VAR_<NOM_EN_MAJUSCULES>` (`-` devient `_`). Un préfixe évite qu'un `{{home}}` lise `$HOME`.
- Option B : espace de noms dans la balise, `{{var.name}}` (le `.` sort de l'id, donc jamais confondu avec un template) ; même préfixe d'environnement. Demande d'étendre le scanner du modèle.
- Option C : l'environnement est lu sans préfixe (`{{HOME}}` lit `$HOME`). Plus court, mais une variable d'environnement quelconque peut entrer dans des fichiers générés : je ne le recommande pas.

### Q4. Où vivent, dans l'architecture, la référence de pack, la fusion et la lecture de l'environnement ?
Bloque : le découpage du plan en tâches `schema` / `adapter` / `cli`, et la clause de pureté de `armadai-model`.
- Option A (recommandée) : `armadai-model` gagne une clé optionnelle de `project.yaml` (la référence : nom, source, épingle) sans changer `schema_version: 1`, la fusion clé par clé (listes complétées des deux côtés, R6) et la substitution comme fonctions pures sur des valeurs (`serde_json::Value` et `BTreeMap<String,String>`) ; le binaire lit l'environnement, les fichiers de variables, le cache et le registre, puis passe des données. Aucun nouveau crate, pureté préservée, schémas régénérés.
- Option B : même chose mais la clé de référence vit dans un fichier séparé du `project.yaml` (`.armadai/pack.yaml`). Pas de changement du schéma de `project.yaml`, mais décision I (« project.yaml versionné ») non respectée à la lettre.
- Option C : le modèle lit lui-même l'environnement et le cache. Je le déconseille : le cache est sous `~/.config/armadai/`, que le modèle ne connaît pas.
Question liée : ajouter une clé à `project.yaml` sans passer `schema_version` à 2 est-il acceptable ? Rien dans `armadai-rules` ne le dit (la règle « champ optionnel, v1 » vise le manifeste de `link`). Je recommande oui, par symétrie avec cette règle.

### Q5. Forme du champ de traçabilité dans le link-manifest, et ce qu'il advient après un clone
Bloque : la tâche link-manifest, le critère 3 (mode copie), le critère 4.
Constat : `ManifestEntry` décrit une sortie compilée ; l'empreinte du mode copie décrit un fichier d'entrée de `.armadai/` (un agent donne plusieurs sorties, `project.yaml` n'est la sortie de rien). Les installations (`init --pack`) ont lieu avant le premier `link`. Et `link-manifest.yaml` est ignoré par git : après un clone, l'origine d'un pack copié est perdue.
- Option A (recommandée) : une section optionnelle au premier niveau du manifeste (`packs:`, `#[serde(default, skip_serializing_if)]`, v1) listant par fichier copié son empreinte et son pack d'origine ; écrite par `init --pack` et `pack update`, et le mode référence y garde l'origine résolue. Après un clone, `pack update` en mode copie refuse en disant qu'aucune origine n'est connue, et indique de réinstaller (aucune devinette, cf. l'anti-objectif du spec).
- Option B : un champ optionnel sur chaque `ManifestEntry`. Colle à la lettre « un champ optionnel nouveau » mais l'entrée ne représente pas le fichier copié.
- Option C : en mode copie, garder l'origine dans `project.yaml` versionné aussi (comme en mode référence). Résout le clone, mais contredit la décision « via le link-manifest ».

### Q6. Registre des projets référents : étendre `projects.json` ou créer un nouveau fichier ?
Bloque : tâche registre, critères 2 et 7.
Constat : `~/.config/armadai/projects.json` (`armadai_core::project_registry`) enregistre déjà chaque projet à son `link`, mais sans notion de pack, et `models check --all --prune` y retire des projets obsolètes. Décision L : fichier dans `~/.config/armadai`, entrée (pack, chemin), jamais retiré en silence.
- Option A (recommandée) : un nouveau fichier (nom au plan, par exemple `pack-referents.yaml`), indépendant de `projects.json` ; pas de risque d'élaguer par erreur.
- Option B : étendre `projects.json` avec un champ optionnel de packs par projet ; réutilise le code, mais `prune_stale` doit être contraint à ne plus rien retirer pour ces entrées.
- Option C : un seul registre, `projects.json` migré vers une forme (pack, chemin) ; le plus propre mais touche `models` hors périmètre.

### Q7. Enregistrer deux règles dans `armadai-rules` ?
Bloque : rien d'immédiat ; sert le validator et le reviewer.
a. Les tests de ce lot n'ouvrent aucun accès réseau : faux registre local (dépôt git local ou archive locale) injecté par l'interface `StarterFetcher`.
b. Rien de privé dans le dépôt public : aucun contenu d'un pack privé, aucun chemin absolu propre à une machine, aucun jeton dans les fixtures, la spec, les docs et le wiki.
- Option A (recommandée) : enregistrer les deux, source « Dimitri, 2026-11-03 ».
- Option B : enregistrer seulement la (b) (la (a) reste une habitude de ce lot).
- Option C : n'enregistrer ni l'une ni l'autre.

### Q8. `armadai link update` est-il dans ce lot ?
Bloque : rien tant que la réponse est non ; sinon un critère manque.
Constat : Dimitri l'a demandé (2026-11-02/03), la spec n'en fait qu'un anti-objectif conditionnel et aucun critère ne le couvre ; « un seul lot, tout » liste cinq capacités qui ne le nomment pas.
- Option A (recommandée) : hors lot ; `pack update` couvre le re-link du mode référence.
- Option B : dans le lot comme alias fin de `link` après `pack update` ; ajoute un critère.
- Option C : hors lot et retiré du backlog.

## 5. À retenir pour le plan (architect)

- Cadre applicable : section 1 ; deux règles enregistrées dans `armadai-rules` après la réponse Q7 (aucun réseau réel dans les tests ; rien de privé dans le dépôt public).
- Dépend des réponses : Q1, Q2, Q3, Q4, Q5, Q6 changent l'ordre et le découpage des tâches ; Q7 et Q8 non.
- Les textes de sortie, les noms de commandes, de fichiers de variables et du registre sont proposés par le plan et validés par Dimitri (section 1, points 5).
- Après toute édition de `armadai-rules`, `lead` relance `armadai link --target claude`.
