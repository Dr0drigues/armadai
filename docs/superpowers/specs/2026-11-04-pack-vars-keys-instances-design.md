# Variables de pack : clés du bloc `native` et `vars` des instances de template

Date : 2026-11-04 · Statut : prête pour le cadre (4 décisions de Dimitri enregistrées, aucune question ouverte) · Petit lot qui complète « gestion des packs » (`docs/superpowers/specs/2026-11-03-pack-management-design.md`, #539) · Issues #545 et #546 · Cadre : `armadai-rules`, non dupliqué ici.

## 1. Intention

Les variables de pack (`{{var.name}}`, rendues au `link` seulement) laissent deux trous. #545 : elles ne sont pas rendues dans les **clés** du bloc `native` de `project.yaml` (ex. `native.opencode.permission.external_directory: { "{{var.data-dir}}/**": allow }` reste littéral dans `opencode.json`). #546 : un agent instance de template (`from:` / `vars:`) qui met un `{{var.name}}` dans ses `vars:` le voit rester littéral dans `.claude/agents/<id>.md`, sans aucun diagnostic (le template est rendu au chargement, après la substitution de pack, qui ne touche que les corps). Conséquence mesurée sur un pack réel : 6 chemins de machine y restent en dur (5 clés OpenCode, 1 instance de template), donc le pack ne peut pas être poussé dans le dépôt de packs (Dimitri, 2026-11-04 : « Attendre les correctifs »). Dimitri a choisi « Issues, petit lot ensuite » puis « lance le petit lot maintenant » (2026-11-04).

**Décisions de Dimitri (2026-11-04), mot pour mot :**
- #546, substituer ou signaler : « Substituer (Recommandé) » : les variables de pack sont substituées dans les valeurs texte de `vars:` d'une instance.
- Un `{{var.…}}` resté littéral ailleurs dans une sortie compilée : « Hors lot, issue (Recommandé) » : pas de contrôle général dans ce lot ; une issue séparée est à ouvrir après le lot.
- Collision de clés rendues : « Erreur (Recommandé) ».
- Variable absente dans une clé de `native` : « Erreur pour une clé (Recommandé) » : `link` refuse ; les valeurs (et les `vars:` d'instance) gardent « avertissement + vide ».
- Variable définie mais vide dans une clé : « Refuser une clé élargie (Recommandé) » : toute variable vide dans une clé de `native` (définie à `""` dans une couche, ou absente) fait refuser `link`, avant toute écriture ; sinon `"{{var.data-dir}}/**": allow` rendrait `"/**": allow`, tout le disque.

## 2. Objectifs, non-objectifs, anti-objectifs

**Objectifs**
1. Rendre les variables aussi dans les clés texte de `native` (tous niveaux, toutes cibles), avec les mêmes trois couches que pour les valeurs. Une variable absente des trois couches dans une **clé** fait refuser `link` (une clé est un sélecteur de règle : vide, elle élargirait la règle).
2. Deux clés d'un même objet qui rendent la même chaîne : erreur nommée, jamais un écrasement silencieux.
3. Substituer les variables de pack dans les valeurs texte de `vars:` d'un agent instance (absente : avertissement + vide, comme une valeur) : jamais de `{{var.…}}` littéral silencieux dans ce cas.
4. Mettre à jour `docs/wiki/starter-packs.md` (section Variables : portée, « frontmatters jamais substitués » devient « sauf `vars:` d'une instance »).

**Non-objectifs**
- Substituer le reste des frontmatters, `scripts/`, `references/`, `native/<cible>/` (décision du lot précédent, inchangée).
- Un contrôle général « `{{var.…}}` resté littéral dans une sortie compilée » (décidé hors lot : issue séparée à ouvrir après le lot).
- Valider le contenu de `native` ou les chemins obtenus.
- Toucher `.armadai/`, le cache ou le verrou : la substitution reste au `link`, dans la copie de travail.

**Anti-objectifs**
- Réécrire `.armadai/` (une instance garde sa forme `from:`/`vars:` sur disque).
- Casser le YAML d'un frontmatter par une valeur contenant `:`, `#`, des guillemets : une valeur est insérée comme chaîne, jamais re-parsée.
- Changer le type d'une valeur non textuelle (entier, booléen, null) ou le texte d'une clé sans variable.
- Élargir en silence une règle de permission (voir risque R1).

## 3. Contraintes

- `armadai-model` reste pur (`pack_vars.rs` en est la seule logique de substitution) ; le binaire (`pack/effective.rs`) lit les couches et appelle le modèle, comme aujourd'hui.
- Une variable ne se résout qu'au `link` (valeurs d'environnement possibles) : `validate`, qui lit en `Vars::Literal`, ne voit ni collision ni valeur résolue ; il accepte les clés et `vars:` littéraux.
- Syntaxe et échappement inchangés : `{{var.name}}` / `{{ var.name }}`, `\{{` littéral, valeur jamais re-rendue.
- Le texte exact des nouvelles erreurs (collision de clés, variable absente dans une clé) est proposé par le plan et validé par Dimitri à sa relecture du plan (règle du dépôt) ; pas de nouveau code de diagnostic du validateur (ce sont des erreurs et lignes de rapport de `link`).
- Rien de privé : fixtures et doc génériques (`data-dir`, `/srv/data`).

## 4. Conception

- **`armadai-model/src/pack_vars.rs`** : `render_value` rend aussi les clés des objets (reconstruit l'objet ; erreur de collision renvoyée, ses deux clés d'origine et la clé rendue nommées) ; `value_names`/`files_names` comptent les variables des clés (un fichier dont seule une clé porte une variable est rendu, sinon il garde ses octets). `render_files` propage l'erreur (`Result<_, String>` existant) avec le chemin du fichier. Le test existant `render_value_touches_strings_only`, qui fige « clés intactes », est à inverser.
- **Instances (#546)** : portée de `render_files` étendue aux valeurs texte de `vars:` du frontmatter des `agents/*.md` portant `from:`. Contrainte de forme : seules ces valeurs changent, insérées comme chaînes (pas de YAML re-parsé) ; les autres clés du frontmatter restent telles quelles. Le moyen (édition de texte ciblée ou parse + ré-émission de la copie de travail) est au plan ; les positions des diagnostics doivent rester celles de l'agent d'origine ou être dites approximatives. Le rendu du template par `agent_templates` reste après, et n'y re-rend jamais la valeur.
- **Rapport** : une variable absente d'une valeur ou d'un `vars:` rejoint `missing_vars` ; la ligne existante `var <name>: approximated (…)` la couvre, sans nouveau format. Dans une clé, elle est une erreur (pas de ligne de rapport), renvoyée par `render_files`.
- **Doc** : `docs/wiki/starter-packs.md`, section Variables.

## 5. Critères d'acceptation

1. Un pack dont `native.opencode.permission.external_directory` a la clé `"{{var.data-dir}}/**"`, avec `ARMADAI_VAR_DATA_DIR=/srv/data`, donne après `armadai link --target opencode` un `opencode.json` dont la clé est `/srv/data/**` et qui ne contient plus `{{`. Même résultat avec la variable fournie par `.armadai/vars.yaml` ou `vars.local.yaml` (priorité des couches inchangée).
2. Une clé de `native.claude` (ex. sous `env`) portant une variable est rendue de la même façon dans `.claude/settings.json`.
3. Un `project.yaml` dont une variable n'apparaît que dans une clé (aucune valeur n'en porte) est tout de même rendu.
4. Deux clés d'un même objet de `native` qui rendent la même chaîne (`"{{var.a}}/**"` et `"/srv/data/**"`, `a=/srv/data`) : `armadai link` sort avec un code non nul, le message nomme `project.yaml`, les deux clés d'origine et la clé rendue, et aucune sortie compilée n'est écrite ni modifiée (`opencode.json` identique octet pour octet avant et après). (Décision : erreur.)
5. Une variable **vide** dans une **clé** de `native`, qu'elle soit absente des trois couches ou définie à `""` (ex. `data-dir: ""` dans `vars.yaml`, ou `ARMADAI_VAR_DATA_DIR=` vide, avec la clé `"{{var.data-dir}}/**"`) : `armadai link` sort avec un code non nul, l'erreur nomme la variable et le fichier, aucune sortie compilée n'est écrite ni modifiée. La même variable vide ou absente dans une **valeur** ou un `vars:` d'instance donne toujours la valeur vide ; absente, la ligne `var <name>: approximated (…)` du rapport la nomme ; `link` réussit.
6. Un agent instance (`from: <template>`, `vars: { dir: "{{var.data-dir}}" }`) avec `ARMADAI_VAR_DATA_DIR=/srv/data` donne un `.claude/agents/<id>.md` qui contient `/srv/data` à la place de `{{var.data-dir}}`; `.armadai/agents/<id>.md` est inchangé octet pour octet et garde `from:`/`vars:`.
7. Une valeur de variable contenant `: ` et `#` (ex. `a: b # c`) est insérée telle quelle dans l'agent compilé, sans erreur de YAML.
8. Les autres clés du frontmatter d'une instance (ex. `description: "{{var.x}}"`) ne sont pas substituées, et un entier, un booléen ou une clé sans variable de `native` sortent identiques à aujourd'hui.
9. `armadai validate` accepte un projet portant ces clés et ces `vars:` littéraux, avec le même résultat qu'avant ce lot.
10. Essai réel avec Dimitri : le pack réel dont 6 chemins étaient en dur est variabilisé (5 clés, 1 instance), lié dans un projet isolé (`HOME` et `ARMADAI_CONFIG_DIR` isolés) : aucune sortie ne contient un `{{var.` résiduel, et le pack ne contient plus aucun chemin de machine.

## 6. Risques

| # | Risque | Parade |
|---|---|---|
| R1 | Une clé de règle de permission (`"{{var.x}}/**": allow`) dont la variable est absente ou définie vide deviendrait `"/**"` : la règle s'élargirait à tout le disque. | Décidé : erreur pour toute variable vide dans une clé (critère 5) ; un test par cas (absente, définie à `""`), mutés. |
| R2 | Réécrire le frontmatter d'une instance décale les lignes des diagnostics ou casse le YAML (valeur avec `:`/`#`). | Critère 7 ; choix de la technique au plan, muté. |
| R3 | La clé rendue de `native.<cible>` de premier niveau contient un `.` : refusée par l'adaptateur OpenCode (`native.opencode key ... contains a .`) ; le message nomme la clé rendue, pas la clé d'origine. | À vérifier au plan ; accepté si le message reste compréhensible. |
| R4 | Un test « clés intactes » existant (`render_value_touches_strings_only`) passe encore sans protéger la nouvelle règle. | Mutation : désactiver le rendu des clés, constater le rouge de chaque test ajouté. |
| R5 | Un `{{var.…}}` reste littéral ailleurs (frontmatter d'une instance hors `vars:`, `scripts/`) : hors lot (décidé), issue à ouvrir. | Documenté dans le wiki. |

## Questions ouvertes

Aucune. Reste hors lot : ouvrir l'issue du contrôle général des `{{var.…}}` résiduels.
