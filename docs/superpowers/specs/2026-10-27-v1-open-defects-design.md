# Défauts ouverts avant la v1.0.0

Issues publiques (`Dr0drigues/armadai`) : #534, #521, #522, #523, #524, #525, #526, #517, #505, #506.

## 1. Intention

Dix défauts sont ouverts, tous trouvés à l'usage (essais réels, revues) sur `link`, `unlink`, `validate` et `init --pack`. Chacun est petit, mais ensemble ils laissent ou perdent des octets chez l'utilisateur (hooks laissés par `unlink`, bit exécutable perdu, clés réordonnées), refusent un cas légitime (`permissions`), ou donnent un diagnostic faux ou trompeur. Le 2026-10-27, à la question « Qu'est-ce qui entre dans la v1.0.0, avant la clôture #260 ? », Dimitri a répondu : « Défauts ouverts, Table de modèles #455, Packs de normes (lot C) ». Ce lot ne couvre que les défauts ouverts ; #455 et le lot C auront chacun leur lot, Codex et Copilot ne sont pas de la v1. Pour #522, décision de Dimitri du 2026-10-26 : « ArmadAI fusionne ».

**État des issues (affinage du 2026-10-27).** Le texte des dix issues a été lu (extrait par `lead` avec `gh issue view`) : les dix sont `OPEN`, et `lead` a mesuré qu'aucun commit de `master` ne les corrige. Code relu : `compile/claude_settings.rs`, `compile/patch.rs`, `armadai-model/src/check.rs` (l. 290-340, 400-790), `diag.rs`, `cli/native_pack.rs`, `cli/validate.rs`, `cli/link.rs` (`copy_executable_bits`), `docs/wiki/link.md`. Non lu : la lecture de `config.yaml` et de `model_tiers` (#506), `cli/mod.rs` (`--help`), `docs/wiki/starter-packs.md`, `tests/init_pack.rs` ; les lignes que les issues citent pour eux sont reprises telles quelles.

## 2. Objectifs, non-objectifs, anti-objectifs

**Objectifs**
1. Que des hooks identiques trouvés par `link` sans manifeste puissent être adoptés explicitement, pour qu'`unlink` les retire, et que le rapport le dise (#534, décision 1) ; que `link` puis `unlink` ne réordonnent jamais les clés de l'utilisateur (#505).
2. Que `link` fusionne `native.claude.permissions` liste par liste avec les `permissions` existants (#522).
3. Que `validate` dise juste : la position du lien fautif pour `unknown-state` (#523), un script cité et absent (#524), les agents intégrés des cibles déclarées non signalés à tort (#517).
4. Qu'une seule forme et un seul compte existent pour « ignored » (#525), et que l'aide et la doc de la liste blanche d'`init --pack` soient complètes (#526).
5. Que `init --pack` garde le bit exécutable des scripts d'une skill (#521).
6. Qu'une clé inconnue sous `model_tiers` ne désactive plus des fonctions sans rapport (#506).

**Non-objectifs** : la table de modèles (#455), les packs de normes (lot C), Codex et Copilot, la fusion de clés de `settings.json` autres que `permissions`, un script nommé et présent mais non exécutable (cité par #524 comme voisin hors périmètre), tout nouveau format ou nouvelle commande.

**Anti-objectifs** : ne jamais supprimer ou modifier un hook ou une permission de l'utilisateur ; ne jamais changer l'ordre de ses clés ; ne jamais faire échouer `link` ou `validate` sur un projet qui passait, (#524 est un avertissement, décision 5) ; ne jamais deviner un lien ou un agent (message précis plutôt que probable).

## 3. Contraintes

- `armadai-model` reste pur (serde, serde-saphyr, serde_json, thiserror, schemars). Un code de diagnostic ajouté passe par `docs/wiki/project-format.md` ; les schémas se régénèrent si un type change (`cargo run -p armadai-model --example gen_schemas`).
- `settings.json` : propriété par valeur exacte dans le manifeste, aucun champ de marquage dans le hook, fichier réécrit seulement si sa valeur change ; sans manifeste, `unlink` ne touche pas `settings.json` (`link.md`). Les manifestes existants restent lisibles.
- Barrière complète, une mutation vue rouge par test, `HOME` et `ARMADAI_CONFIG_DIR` isolés (`armadai-rules`). Les tests actuels de `claude_settings.rs` comparent des `Value`, dont l'égalité ignore l'ordre des clés : un test d'ordre compare le texte.
- `frozen/armadai-audit` à vérifier si `armadai-core` ou `armadai-model` change.
- Les golden (`tests/golden/`, `link_new_cli.rs`, `tests/init_pack.rs:737`) changent si un message ou un rapport change.

## 4. Conception

Chaque défaut : comportement actuel (selon l'issue), attendu, domaine, où.

### 4.1 #534 — `link` ne reprend pas un hook identique (adaptateur)
- Actuel : la sortie `.claude/**` et `settings.json` est commitée, le manifeste est ignoré par git. Sur un clone, un worktree ou après `git clean`, `merge_settings` ne revendique jamais un hook identique déjà présent si le manifeste précédent ne le possédait pas (`if present(..) { if previous.entries.contains(e) {..} continue }`). `--force` n'y change rien, et le manifeste étant réécrit depuis `owned`, l'état est permanent. `link` dit déjà `hook … approximated (existing identical hook kept, not owned)` ; `unlink` retire agents, skills et bloc `CLAUDE.md` mais laisse la gate et les hooks d'enregistrement. Les fichiers byte-identiques, eux, sont repris (`UpToDate` → `Created`). rc.7 enregistre bien les hooks sur un `link` neuf.
- Attendu (décision 1, « Adoption explicite ») : `link` ne revendique jamais de lui-même un hook identique trouvé sans manifeste (le comportement actuel de `merge_settings` reste). Il offre une façon explicite de l'adopter, et son rapport la dit, à l'endroit où il dit aujourd'hui « not owned » (le hook survivra à `unlink` tant qu'il n'est pas adopté). Après adoption, `unlink` retire ces hooks et le manifeste les possède. La forme de l'adoption (option de `link`, commande, ou message avec la réparation en une étape) n'est pas décidée : le plan la propose, Dimitri la valide. Contrainte : jamais de perte d'un hook de l'utilisateur sans geste explicite de sa part.
- Impact : le test `an_identical_entry_already_present_is_neither_duplicated_nor_claimed` reste vrai sans adoption ; un test couvre l'adoption. La phrase de `link.md` (« a hook identical to one link would add is left to you and reported ») et le rapport changent pour nommer l'adoption. Site : `merge_settings` et `link`.

### 4.2 #522 — fusion de `native.claude.permissions` (adaptateur)
- Actuel : `native.claude` écrit des clés de premier niveau, chacune prise en bloc ; `bail!("settings.json key `{k}` is already set to another value")` (`claude_settings.rs` l. 275 de l'issue) dès que `permissions` existe avec une autre valeur, même si elles ne diffèrent que par une entrée de liste. Rien n'est écrit. L'erreur nomme la clé mais pas la valeur en conflit, et sa première ligne est le seul chemin du fichier.
- Attendu : `permissions.{allow,ask,deny}` fusionnés liste par liste, comme les hooks : règles du projet ajoutées à la suite, celles de l'utilisateur gardées, sans doublon, `unlink` retire seulement ce que `link` a ajouté. Les autres clés gardent la règle « valeur différente = erreur ». Piste de l'issue : traiter les listes comme un conteneur de hook, entrées enregistrées dans le manifeste (`Owned`), à la manière de `JsonKeys.array_entries` d'`opencode.json`. Décisions 2 et 3 : toute clé de `permissions` dont les deux valeurs sont des listes est fusionnée (`additionalDirectories` comprise, sans liste de noms figée) ; une règle `deny` d'ArmadAI qui contredit un `allow` de l'utilisateur est écrite et signalée dans le rapport (Claude Code fait gagner `deny`). Au passage, l'erreur restante (valeur non liste différente, par exemple `defaultMode`) nomme la valeur en conflit.
- Domaine : adaptateur (`claude_settings.rs`, `Owned`, manifeste).

### 4.3 #505 — `unlink` réordonne les clés (adaptateur)
- Actuel (issue, « constaté, non mesuré ») : `claude_settings.rs` retire des clés par `serde_json::Map::remove`, qui est un `swap_remove` avec `preserve_order` : la dernière clé prend la place de la clé retirée. Lu dans le code : `unmerge` (`doc.remove(*k)` pour les clés natives) et `prune` (`doc.remove("hooks")`). Le même défaut a été mesuré et corrigé pour `opencode.json` (#504 : `shift_remove` et remplacement en place). Je n'ai pas vérifié l'activation de `preserve_order` ni mesuré le défaut : à faire d'abord, par un test qui compare le texte, comme l'issue le demande.
- Voisin que l'issue nomme : `Owned.entries[].native` (par exemple `timeout: 5`) serait stocké dans `link-manifest.yaml` sous la forme privée `"$serde_json::private::Number"` (`arbitrary_precision`) ; corrigé pour OpenCode via `NativeValue`. À mesurer de même. Décision 6 : dans le lot s'il est mesuré, même correctif qu'OpenCode (`NativeValue`) ; s'il n'est pas confirmé, rien à faire.
- Attendu : après `link` puis `unlink`, mêmes clés, même ordre, à tous les niveaux.

### 4.4 #523 — position d'`unknown-state` (modèle)
- Actuel : un lien `when: rejected` dont l'étape source ne déclare pas `rejected` est signalé à la position du premier `rejected` du fichier (ici le `states:` d'une autre étape, `3:37`), pas du lien (ligne 8). Cause : `check.rs` l. 719-724 passe seulement le jeton à `unknown_ref` → `emit` → `locate()` (`diag.rs`), première occurrence du mot (code commenté `shortcut`). Même chemin pour `unknown-step` (l. 663, 706, 754), non reproduit.
- Attendu : la position du `when:` du lien fautif. Piste : le chargeur parse déjà le YAML, une position par lien, ou une recherche qui part de la clé `links:` ; aucun code de diagnostic ne change. `emit_pos` existe déjà (`body_pos` en est un exemple). Décision 4 (« 3 diagnostics ») : `unknown-state`, `unknown-step` (l. 663, 706, 754) et la position d'`unknown-free-agent` ; les autres diagnostics qui passent par `locate()` gardent le `shortcut`.

### 4.5 #524 — script cité et absent (modèle)
- Actuel : `SKILL.md` qui dit `Run scripts/run.sh then scripts/missing.sh` (en prose, pas en code), `missing.sh` absent : `validate` rend `0 error(s), 0 warning(s)`, code 0 ; `link` rend `skill s: compiled`. Le corps d'une skill n'est pas lu pour des chemins (`check.rs` ~565-636, contrôles de skill ; `compile/claude.rs` copie « byte for byte »). `project-format.md` dit que le reste du dossier n'est pas contrôlé.
- Attendu (l'issue : « raised as a gap, not as a fixed requirement ») : un diagnostic, un avertissement suffit, sur la `SKILL.md`, ou au moins une ligne du rapport, quand un chemin relatif cité n'existe pas dans le dossier. L'issue suggère une règle étroite : un chemin sous `scripts/` ou `references/` écrit dans un code en ligne. Cette règle ne verrait pas la repro de l'issue, écrite en prose. Décision 5 (« scripts/ ou references/, avert. ») : un chemin commençant par `scripts/` ou `references/`, en code en ligne ou en prose, absent du dossier de la skill, donne un avertissement (pas une erreur : `link` n'est pas bloqué).
- Domaine : modèle (`check.rs`, nouveau code) ; `link` l'affiche si les avertissements du validateur y figurent (à vérifier).

### 4.6 #525 — « ignored » hors du compte (CLI)
- Actuel : `validate <pack>` imprime sur stderr `warning: ignored: tests/x (not part of a pack)` (`validate.rs` l. 69-71, `eprintln!` avant `report`), puis le pied `0 error(s), 0 warning(s)` qui ne compte que `ds` ; avec `--json`, la ligne reste sur stderr et le tableau est `[]`. `init --pack` écrit la même chose en `  warn: ignored: … ` (`native_pack.rs` l. 155-159, figé par `tests/init_pack.rs:737`).
- Attendu : une forme et un compte : soit un vrai diagnostic compté et présent dans `--json`, soit la ligne ne s'appelle plus avertissement. Piste de l'issue : un `Diagnostic` de gravité warning avec un code stable dans le résultat de `native_pack::stage`, rendu par le même rendu pour `validate`, `--json` et `init`. Décision 7 (« Vrai avertissement ») : c'est cette piste ; `tests/init_pack.rs:737` change.

### 4.7 #526 — doc de la liste blanche (CLI et doc)
- Actuel : `init --pack` copie `commands/*.md` (`native_pack.rs` l. 4 et 23 le disent bien). Manquent : `docs/wiki/starter-packs.md` l. 30-31 (`commands/*.md`) et `--help` (`cli/mod.rs` l. 260-263 : ni `templates/` ni `commands/`).
- Attendu : doc et aide énumèrent la liste blanche du code : `project.yaml`, `agents/*.md`, `templates/*.md`, `commands/*.md`, `skills/**`, `flows/*.yaml`, `automations/*.yaml`. Sans question.

### 4.8 #517 — `unknown-free-agent` (modèle)
- Actuel : `policy.free_agents: [Explore, Plan, general-purpose, claude-code-guide, claude]` lève `unknown-free-agent` (déjà un avertissement, pas une erreur) pour chaque agent fourni par Claude Code (`check.rs` l. 487-503 ; le code a un commentaire « a free agent may be one the target provides itself » et un indice « fine if the target provides it (Claude Code: Explore, Plan, general-purpose) »). Deux défauts voisins dans l'issue : la position pour `claude` pointe `targets: [claude]` (première occurrence, même mécanisme que #523), et l'indice ne cite que trois agents.
- Attendu (l'issue le dit) : le validateur connaît les agents intégrés des cibles **déclarées** : Claude Code `Explore`, `Plan`, `general-purpose`, `claude-code-guide`, `claude` ; OpenCode `explore`, `general`, `build`, `plan`… La position est celle de la ligne de `free_agents`, l'indice est complet. L'issue tranche donc « par cible » : la liste exacte est à relever sur la documentation officielle de chaque cible par `adapter-specialist` avant d'être écrite (règle de l'équipe), pas recopiée de l'issue.

### 4.9 #521 — `init --pack` perd le bit exécutable (CLI)
- Actuel : `files()` lit chaque fichier du pack en octets (`native_pack.rs` l. 56), le mode n'est pas gardé dans `PackFiles.files` ; `Created::write` crée le fichier par `OpenOptions … create_new(true)` au mode par défaut (l. 108-136). Une skill exécutable dans le pack (`-rwxr-xr-x`) arrive en `-rw-r--r--` sous `.armadai/skills/…`, puis `link` la copie ainsi (`copy_executable_bits`, `link.rs` l. 339-367, garde le bit qu'il trouve). Témoin : un script rendu exécutable à la main sans `init --pack` reste `rwxr-xr-x` après `link`.
- Attendu : le bit exécutable d'un fichier du pack est gardé à l'installation, comme `link` le garde (bit exécutable seul, comme `copy_executable_bits`). Piste de l'issue : garder le mode avec les octets dans `PackFiles` et l'appliquer dans `Created::write`. `files` et `stage` partagent le type `(PathBuf, Vec<u8>)` et `validate.rs` appelle les deux : leur signature change avec la piste. Unix seulement.

### 4.10 #506 — `model_tiers` (configuration)
- Actuel (mesuré pendant la revue de #504) : une clé inconnue sous `model_tiers.opencode` (par exemple `medium:`) fait échouer `link --target opencode`, mais aussi `config starters-dir add/remove` (lecture stricte), et la recherche de packs (`starter.rs:137` via `load_user_config`) avertit puis ignore les `starters_dirs` de l'utilisateur. Une clé propre à une cible désactive des fonctions sans rapport.
- Attendu : l'issue tranche le défaut, pas le remède : lecture tolérante de `model_tiers` (avertissement) ou erreur limitée aux commandes qui la lisent. Décision 8 (« Tolérant sauf usage ») : lecture tolérante pour tout ce qui n'a pas besoin de `model_tiers` (`config starters-dir add/remove`, recherche de packs : un avertissement nommant la clé, lecture continuée, `starters_dirs` gardés) ; erreur nommant la clé là où `model_tiers` sert : `link --target opencode` seulement (`compile/opencode.rs`, `model_for_tier` ; ni `cli/models.rs` ni `compile/claude.rs` ne lisent `config.yaml`, vérifié par la tâche 4). Crate de la lecture de `config.yaml` : non lu, à trouver au plan.

## 5. Critères d'acceptation

Chaque essai : projet temporaire, `HOME` et `ARMADAI_CONFIG_DIR` isolés. Les repros des issues sont les environnements.

1. #534 : un projet avec un flow et la sortie Claude déjà commitée (le `settings.json` contient les hooks que `link` génère, aucun `.armadai/link-manifest.yaml`). (a) `armadai link --target claude` rend 0, ne revendique rien : le manifeste ne possède aucun de ces hooks, et le rapport dit comment les adopter ; `armadai unlink --target claude` les laisse. (b) Après le geste d'adoption proposé par le plan, le manifeste les possède et `unlink` les retire tous, en gardant tout hook de l'utilisateur dont le texte diffère. Commandes exactes de l'adoption : à écrire au plan.
2. #522 : `settings.json` = `{"permissions":{"allow":["Bash(ls)"]}}`, `native.claude.permissions.ask: ["Bash(git push:*)"]` (repro de l'issue). `armadai link --target claude` rend 0 ; le fichier a `"allow": ["Bash(ls)"]` et `"ask": ["Bash(git push:*)"]` ; un second `link` ne change pas le fichier ; `armadai unlink` rend `{"permissions":{"allow":["Bash(ls)"]}}`. Avec aussi `allow: ["Bash(git *)"]` dans le projet : `"allow": ["Bash(ls)", "Bash(git *)"]`, ordre gardé ; avec `additionalDirectories` des deux côtés : fusionné de même. Une règle `deny` du projet contredisant un `allow` de l'utilisateur est écrite et le rapport de `link` la signale. Une clé non-liste (`defaultMode`) en valeur différente garde l'erreur, qui nomme la valeur.
3. #505 : un `settings.json` utilisateur `{"zeta":1,"permissions":{…},"alpha":{…}}` plus un `native.claude` d'une clé (`model`) : après `link` puis `unlink`, `jq 'keys_unsorted'` redonne `["zeta","permissions","alpha"]` ; et le manifeste d'un hook à `timeout: 5` y garde `timeout: 5` (si la mesure confirme la forme privée, décision 6 ; sinon ce point est sans objet).
4. #523 : la repro de l'issue (flow `y`) : `armadai validate` affiche `…/.armadai/flows/y.yaml:8:23: error[unknown-state]: unknown state `rejected`` (ligne 8, colonne du `when:` du lien fautif ; colonne à confirmer au plan) et non `3:37`.
5. #524 : la repro de l'issue (chemins en prose) : `armadai validate` rend le code 0 et `0 error(s), 1 warning(s)`, l'avertissement nomme `scripts/missing.sh`, rien pour `scripts/run.sh` ; avec `missing.sh` créé, `0 error(s), 0 warning(s)`. Un chemin `references/x.md` absent, en code en ligne, donne le même avertissement.
6. #525 : la repro de l'issue (`tests/x` ignoré) : `armadai validate pack` imprime la ligne comme un diagnostic et le pied annonce `0 error(s), 1 warning(s)` ; `armadai validate pack --json` contient ce diagnostic (code stable, plus `[]`) ; `armadai init --pack` imprime la même forme.
7. #526 : `armadai init --help` contient `templates/` et `commands/` ; `docs/wiki/starter-packs.md` liste `commands/*.md`.
8. #517 : `free_agents: [Explore, Plan, general-purpose, claude-code-guide, claude]` avec `targets: [claude]` : `armadai validate` n'affiche aucun `unknown-free-agent` ; avec `free_agents: [inconnu-xyz]` il l'affiche, positionné sur la ligne de `free_agents` ; `claude` seul dans `free_agents` ne pointe plus `targets: [claude]`.
9. #521 : la repro de l'issue : après `armadai init --pack ../pack --no-link`, `test -x .armadai/skills/s/scripts/run.sh` réussit ; après `armadai link --target claude`, `test -x .claude/skills/s/scripts/run.sh` aussi.
10. #506 : un `config.yaml` avec `model_tiers: { opencode: { medium: x } }` : `armadai config starters-dir add <dir>` rend 0 avec un avertissement nommant `medium`, et la recherche de packs garde les `starters_dirs` ; `armadai link --target opencode` échoue en nommant la clé `medium`, sans rien écrire.
11. La barrière complète est verte, chaque test ajouté a sa mutation vue rouge, `frozen/armadai-audit` compile si `armadai-core` ou `armadai-model` a changé.
12. Essai réel avec Dimitri : sur son projet à `settings.json` non vide, `link` puis `unlink` ne laissent aucun hook ArmadAI, changent ni l'ordre ni le contenu de ses clés, et `permissions` est fusionné.

Dix défauts + deux transverses : au-dessus d'une dizaine ; Dimitri a retenu un lot en quatre tâches (décision 9).

## 6. Risques

- **#534** : sans adoption, le défaut subsiste pour qui ne lit pas le rapport (les hooks survivent à `unlink`) ; se voit au critère 1 (a). L'adoption explicite évite de retirer un hook que l'utilisateur a écrit lui-même ; sa forme reste à choisir au plan.
- **#522** : `Owned` change de forme ; un manifeste ancien doit rester lisible. Une entrée de liste que l'utilisateur ajoute ensuite, de même valeur, ne doit pas être retirée à tort. Une règle `deny` d'ArmadAI face à un `allow` de l'utilisateur change le comportement de Claude Code (le `deny` gagne) : le dire dans le rapport.
- **#505** : défaut non mesuré ; risque de corriger le mauvais site (`unmerge`, `prune`, `render`) ; les tests actuels comparent des `Value` et ne peuvent pas l'attraper.
- **#523/#517** : un correctif de position par lien touche `emit`, utilisé par tous les diagnostics ; les golden de diagnostics peuvent bouger.
- **#524** : une règle sur la prose donne des faux positifs (un chemin donné en exemple) et durcit `validate` ; en avertissement seulement, le risque reste visible, pas bloquant.
- **#517** : une liste figée d'agents intégrés vieillit avec les cibles ; la doc dit d'où elle vient.
- **#525** : si la ligne devient un `Diagnostic`, `tests/init_pack.rs:737` et les consommateurs de `--json` changent.
- **#521** : `files`/`stage` changent de type (`validate.rs` aussi) ; Unix seulement.
- **#506** : tolérer une clé inconnue contredit la lecture stricte ailleurs.
- La taille : dix défauts, trois domaines, une revue de branche entière lourde.

## 7. Regroupement proposé en tâches

1. **Adaptateur — `settings.json`** (`claude_settings.rs`, `patch.rs`, manifeste) : #534, #522, #505. Un fichier, une structure `Owned` ; #505 est mesuré d'abord, par un test sur le texte. Les tests de mutation couvrent `merge_settings` et `remove_entries`.
2. **Modèle — validateur** (`armadai-model` : `check.rs`, `diag.rs`) : #523, #517 (même mécanisme de position, à faire ensemble), #524 (nouveau code, doc `project-format.md`).
3. **CLI — packs et sorties** : #521, #525, #526 (`native_pack.rs`, `validate.rs`, `cli/mod.rs`, `starter-packs.md`). #521 et #525 touchent tous deux `files`/`stage` : une seule tâche.
4. **Configuration — `model_tiers`** (#506) : crate à déterminer à la lecture de `config.yaml` ; tâche à part ou jointe à la 3.

Les tâches 1 à 4 sont indépendantes ; la 1 est la plus lourde. Le CHANGELOG reçoit une entrée par défaut corrigé (`### Fixed`).

## Questions ouvertes

- La forme de l'adoption d'un hook (#534) : option de `link`, commande, ou message avec la réparation en une étape. À proposer par le plan, à valider par Dimitri ; ce n'est pas encore une décision.
- Le crate qui lit `config.yaml` (#506) et l'activation de `preserve_order` (#505) : à lire et mesurer au plan.
- L'essai réel (critère 12) reste à jouer avec Dimitri.

## Décisions de Dimitri (2026-10-27)

Réponses aux huit questions de la première version, citées mot pour mot.

1. **#534** : « Adoption explicite ». Ce n'est pas la recommandation de l'analyste (revendiquer sans manifeste, comme pour les fichiers) : `link` ne revendique pas de lui-même ; il propose une façon explicite d'adopter, et le rapport la dit. Forme : question ouverte ci-dessus.
2. **#522, clés** : « Toute clé liste (Recommandé) ».
3. **#522, `deny` d'ArmadAI contre `allow` de l'utilisateur** : « Écrire et signaler (Recommandé) ».
4. **#523** : « 3 diagnostics (Recommandé) » : `unknown-state`, `unknown-step`, position d'`unknown-free-agent`.
5. **#524** : « scripts/ ou references/, avert. (Recommandé) ».
6. **#505, voisin `native`** : « Oui, s'il est mesuré (Recommandé) ».
7. **#525** : « Vrai avertissement (Recommandé) ».
8. **#506** : « Tolérant sauf usage (Recommandé) ».
9. **Taille du lot** : « Un lot, 4 tâches (Recommandé) ».

Tranché par les issues : le périmètre « par cible » de #517 et sa gravité (déjà un avertissement).

## Questions pour Dimitri

Aucune pour l'instant : les huit questions sont tranchées (voir « Décisions de Dimitri »). Reste la forme de l'adoption de #534, que le plan propose.

## Cadre

Établi par `framer` le 2026-10-27, avant le plan. Sources : `armadai-rules`, `CLAUDE.md` (racine, « R »), `.claude/CLAUDE.md` (« E »), la CI (`.github/workflows/ci.yml`, lue), `CHANGELOG.md` (tête), `docs/wiki/project-format.md` (table des codes), le code lu : `linker/manifest.rs`, `compile/patch.rs`, `compile/claude_settings.rs` (tête), `compile/json_keys.rs` (tête), `tests/init_pack.rs` (l. 715-745). Non relu : les golden (`tests/golden/`, `tests/link_new_cli.rs`) et leur mécanisme de régénération éventuel (aucune commande de recherche disponible) ; l'architecte en dresse la liste au dépôt. Aucune contradiction trouvée entre `armadai-rules`, `CLAUDE.md` et `.claude/CLAUDE.md`. `armadai-rules` n'est pas modifiée par cette étape (aucune règle nouvelle décidée).

Deux faits du dépôt à connaître avant le plan : (1) la tête de `CHANGELOG.md` est `## v1.0.0-rc.8 (2026-10-27)` : il n'y a plus de section `## Unreleased` (le cadre du lot précédent la disait présente, la rc.8 l'a consommée) ; (2) la spec avait une seconde section « Questions pour Dimitri », reliquat périmé de la première version (ses huit questions sont tranchées) : supprimée par `framer` le 2026-10-27 sur demande de `lead`, `analyst` étant inactif (la section lui appartient).

### Règles du dépôt qui s'appliquent

- Barrière avant push, code retour de chaque commande lu, jamais de `tail` : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model` (R, Build & Test ; E, Barrière ; `armadai-rules`, Barrière). CI : fmt, clippy, test, build (`cargo build --release` plus les deux gardes), Conventional Commits, audit ; `RUSTFLAGS="-D warnings"` (`ci.yml`).
- `frozen/armadai-audit` : `cd frozen/armadai-audit && cargo check --all-targets && cargo test` seulement si `armadai-core` ou `armadai-model` change (R, Architecture). Les tâches 2 (`armadai-model`) et peut-être 4 (selon le crate de `config.yaml`) le déclenchent. `armadai-model` reste pur (serde, serde-saphyr, serde_json, thiserror, schemars) : la liste des agents intégrés de #517 est de la donnée dans `check.rs`, sans dépendance nouvelle.
- Crate binaire seul : `cargo test --bin armadai <nom>` ou `--test <nom>`, jamais `--lib`, toujours `--no-fail-fast`, `--exact` avec le chemin complet ; `env_lock()` non réentrant, valeurs de référence lues sous le verrou, 5 à 8 passes (R ; E ; `mémoire:feedback_lire_sous_le_verrou.md`).
- Tests : une mutation vue rouge par test ajouté ou changé, rejouée par un tiers, restauration par `touch` ; égalité exacte (lignes entières), fixtures à valeurs distinctes (`armadai-rules`, Tests et mutations). Formes à surveiller dans ce lot : (1) #505 : un test qui compare des `Value` ne voit pas l'ordre des clés ; comparer le texte (la spec le dit, §3) ; (2) #523/#517 : un `contains` sur `rejected` ou `claude` est satisfait par la mauvaise position (c'est le défaut) : comparer ligne et colonne exactes ; (3) #522 : une boucle sur `allow`/`ask`/`deny` oublie `additionalDirectories` et la clé non-liste ; (4) après avoir corrigé un site (`unmerge`), vérifier l'autre (`prune`, `render`) : « fermer la classe de défaut, vérifier la couche au-dessus ».
- Mesurer avant de corriger : #505 et son voisin `native` sont « constatés, non mesurés » (§4.3) ; un fait tiré d'une issue se revérifie au dépôt avant d'entrer dans un brief (`armadai-rules`, Rapport).
- Environnement : aucune tâche en arrière-plan, `/usr/bin/grep` et `/bin/ls`, `/bin/ps` après une commande mise en arrière-plan par le harnais, un `CARGO_TARGET_DIR` privé par worktree, jamais `git stash`, `HOME` et `ARMADAI_CONFIG_DIR` isolés pour tout essai du binaire (`armadai-rules`, Environnement). Pour #506, `ARMADAI_CONFIG_DIR` isolé est indispensable (le défaut touche `config.yaml`).
- Git : Conventional Commits, un type par sujet, commits en anglais, PR en français, trailer fourni par la session sans nom de modèle ; PR ouverte, sans fusion, sauf autorisation expresse du brief (R ; E ; hook `.githooks/commit-msg`).
- Domaine natif : l'adapter-specialist vérifie chaque format natif dans la documentation officielle avant de l'écrire (E, tableau de l'équipe) : la forme de `permissions` de `settings.json` (#522 : `allow`, `ask`, `deny`, `additionalDirectories`, et « `deny` gagne ») et la liste des agents intégrés de Claude Code et d'OpenCode (#517) se relèvent à la source, pas à l'issue.
- `settings.json` : propriété par valeur exacte dans le manifeste, aucun champ de marquage dans le hook, fichier réécrit seulement si sa valeur change, hook identique déjà présent ni dupliqué ni revendiqué, sans manifeste `unlink` ne touche pas `settings.json` (`claude_settings.rs` en-tête ; `patch.rs` ; `docs/wiki/link.md`, §3 de la spec). Précédent d'une liste fusionnée par entrées : `JsonKeys.array_entries` / `OwnedJson` (`json_keys.rs`), dont les nombres sont écrits par `NativeValue` (correctif de #504).
- Documentation tenue avec le code : `docs/wiki/link.md` (phrase sur le hook identique, #534 ; fusion de `permissions`, #522), `docs/wiki/project-format.md` (table des codes, section Validation « première occurrence du mot », dossier de skill « non contrôlé », #524/#523/#517), `docs/wiki/starter-packs.md` et `--help` d'`init` (#526), `docs/wiki/policy-gate.md` si la liste des agents libres bouge. Schémas : `cargo run -p armadai-model --example gen_schemas` seulement si un type change.

### Conventions déjà décidées

- **Le périmètre et les neuf décisions** de la section « Décisions de Dimitri (2026-10-27) » : adoption explicite sans revendication automatique (1), toute clé liste de `permissions` et `deny` écrit et signalé (2, 3), trois diagnostics à position exacte (4), #524 en avertissement sur `scripts/` ou `references/` (5), `NativeValue` si mesuré (6), #525 vrai avertissement (7), #506 tolérant sauf usage (8), un lot en quatre tâches (9). Source : Dimitri, 2026-10-27.
- **Codes de diagnostic stables.** Source : `CLAUDE.md` (« semantic validator (stable codes) ») ; `docs/wiki/project-format.md`, table « Diagnostic codes » : un code publié ne change pas de sens ni de nom. Les codes existants de la table sont une kebab-case sans verbe d'action. La règle de nommage elle-même n'est écrite nulle part : voir la question 3.
- **Un `link` neuf n'écrase jamais l'utilisateur** : jamais de perte d'un hook ou d'une permission de l'utilisateur sans son geste explicite (spec, anti-objectifs ; `link.md`).
- **`unlink` sans manifeste ne touche pas `settings.json`** (`link.md`).
- **Forme de l'adoption (#534)** : « Option de link (Recommandé) » : l'option `--adopt-hooks` de `link`, citée par le rapport à l'endroit du « not owned » ; pas de nouvelle commande, `--force` n'est pas réutilisé. Source : Dimitri, 2026-10-27.
- **Manifeste (#522)** : « Champ optionnel, v1 (Recommandé) » : un champ optionnel ajouté à `Owned`, `version` reste 1 ; un manifeste rc.8 se lit sans migration (un test en lit un vrai, avec et sans la forme privée des nombres) ; la limite d'un binaire rc.8 (il ignore le champ et `unlink` laisserait les permissions) est écrite dans `link.md`. Source : Dimitri, 2026-10-27.
- **Nouveaux codes** : « skill-missing-path, ignored-pack-file (Recommandé) », avec la règle : kebab-case, préfixe de la famille quand elle existe, jamais de renommage après publication. #524 : `skill-missing-path` (avertissement) ; #525 : `ignored-pack-file` (avertissement). Source : Dimitri, 2026-10-27.
- **Texte des lignes nouvelles** : « Toi, au plan (Recommandé) » : le plan propose le texte de chaque ligne nouvelle (adoption, `deny` contre `allow`, #524, #525, erreurs de #522 et #506), Dimitri les valide à sa relecture du plan, puis les golden sont mis à jour mécaniquement, relus ligne à ligne. Source : Dimitri, 2026-10-27.
- **CHANGELOG** : « Fixed + Added (Recommandé) » : `### Fixed` pour les défauts, `### Added` pour l'option `--adopt-hooks` et les deux codes ; commits `fix(...)` et `feat(...)` pour ces derniers ; `## Unreleased` créée au-dessus de la rc.8 par la première entrée. Source : Dimitri, 2026-10-27.

### Réponses aux points que le brief demande de cadrer

- **Golden.** Aucune règle de `armadai-rules` ne dit comment mettre à jour un golden. Ce qui s'applique : l'égalité exacte (un golden change à la ligne près, jamais par régénération en bloc non relue) et « ne présenter comme vérifié que ce qui est vu ». Le diff d'un golden se relit ligne à ligne contre la spec : chaque ligne qui bouge est une ligne que la spec fait bouger, sinon c'est un défaut. Une mise à jour dont la nouvelle ligne est écrite par la spec ou par une décision (ex. #525 : « Vrai avertissement ») est mécanique pour le spécialiste. Le **texte** d'une ligne nouvelle (le rapport de `link` pour l'adoption, le signalement `deny` contre `allow`, le message de #524, la forme imprimée de #525, l'erreur qui nomme la valeur ou la clé `medium`) est une décision de Dimitri, par précédent : le 2026-10-26 il a tranché le texte de la ligne `flow recording` (« Nommer les 4 »). Question 4. Golden à relire dans ce lot : `tests/init_pack.rs:737` (vérifié : l'assertion `"  warn: ignored: templates/notes.txt (not part of a pack)\n"` sur stderr), `tests/golden/`, `tests/link_new_cli.rs` ; l'architecte les liste au dépôt.
- **Manifeste (#522).** Fait lu : `linker/manifest.rs` écrit un champ `version: u32` (`MANIFEST_VERSION = 1`) ; « a manifest whose `version` differs is treated exactly like a missing one » (`Lookup::Fallback`). Le commentaire de tête du module dit « Not versioned » : il se contredit avec le code (le format porte une version), constaté, non tranché. `Owned` (`claude_settings.rs`) a trois champs, tous `#[serde(default, skip_serializing_if)]`, donc un champ ajouté de même forme se lit sur un manifeste de la rc.8 (précédent : `patched`, « absent from manifests written before step 2, hence the default »). Deux conséquences : (a) un manifeste rc.8 reste lisible sans migration si le nouveau champ est optionnel ; changer `version` ferait lire tous les manifestes rc.8 comme absents (retour au repli par contenu de #342, `unlink` ne retire plus les hooks par valeur) ; (b) si #505 confirme la forme privée `"$serde_json::private::Number"` dans `Owned.entries[].native`, un manifeste rc.8 peut la contenir : la lecture doit accepter les deux formes, sinon `unlink` ne reconnaît plus un hook à `timeout: 5` (même exigence que `OwnedJson`/`NativeValue`). Compatibilité descendante (un binaire rc.8 lisant un manifeste du lot) : un champ inconnu est ignoré à la lecture, donc `unlink` d'une rc.8 laisserait les permissions ajoutées : à dire dans `link.md`, pas à corriger. Version et forme : question 2.
- **Nouveau code (#525, et #524).** Le brief dit un code pour #525 ; la spec en ajoute deux : #524 (§4.5, « nouveau code ») et #525 (§4.6, « code stable »). #523 et #517 n'en ajoutent pas (`unknown-state`, `unknown-step`, `unknown-free-agent` gardent leur nom et leur sens). Règle commune, imposée : chaque code ajouté entre dans la table de `project-format.md` (code, gravité, sens, correction) et dans `--json` ; il est stable dès sa publication (v1.0.0). Nommage : voir la question 3.
- **Adoption (#534).** Oui, une nouvelle option de `link` ou une nouvelle commande demande l'accord de Dimitri : la règle « demander avant d'adopter une convention absente du cadre » couvre une forme de message, un format de sortie et une surface de CLI ; la spec le dit déjà (§4.1 : « le plan la propose, Dimitri la valide »). Une option change `--help`, `docs/wiki/link.md`, la complétion générée et un golden au moins ; une commande change en plus la liste de `CLAUDE.md` (« Commands ») et contredit le non-objectif « tout nouveau format ou nouvelle commande » (§2), à lever par Dimitri s'il la choisit. Le plan peut avancer sur les tâches 2 à 4 sans cette réponse ; la tâche 1 (interfaces de `merge_settings`, de `Owned` et du rapport) en dépend. Question 1.
- **CHANGELOG.** `## Unreleased` n'existe plus : la première entrée du lot la crée au-dessus de `## v1.0.0-rc.8`. Précédents lus : `### Fixed` (rc.8), `### Added` avec une mention « New diagnostics: … » (rc.7). Une entrée par défaut corrigé (spec §7) ; sections proposées à la question 5.

### Observé, non décidé

- Nommage des codes : kebab-case, préfixe de famille (`command-*`, `skill-*`, `unknown-*`, `duplicate-*`), pas de verbe d'action, `ignored-file` existe déjà (warning, `.yml` de `flows/` ou `automations/`). Habitude de la table, aucun outil ne l'impose.
- Octet exécutable : `copy_executable_bits` (`link.rs`) ne garde que le bit exécutable, Unix seulement, et la spec reprend cette forme pour #521 : c'est la règle de la spec, pas une règle du cadre ; la même garde `#[cfg(unix)]` existe dans `patch.rs`.
- Avertissement de configuration (#506) : le texte, la sortie (stderr) et le préfixe `warning:` sont une habitude de `validate`/`init` ; pas de format décidé pour un avertissement de lecture de `config.yaml`.
- Où vit la liste des agents intégrés (#517) : `check.rs` du modèle (la spec le suppose) ; `armadai-model` connaît déjà les cibles (`unknown-target`), aucune règle ne place cette donnée.

### Questions pour Dimitri

Aucune : les cinq questions du cadre sont tranchées (voir « Conventions déjà décidées »). Les renvois « question 1 à 5 » plus haut désignent ces cinq décisions.
