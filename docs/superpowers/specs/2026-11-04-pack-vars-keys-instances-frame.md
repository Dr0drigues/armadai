# Cadre du lot « variables de pack : clés de `native` et `vars` des instances »

Date : 2026-11-04 · Étape `frame` du flow `lot` · Spec : `docs/superpowers/specs/2026-11-04-pack-vars-keys-instances-design.md` (#545, #546 ; 4 décisions de Dimitri du 2026-11-04 citées dans la spec, non rediscutées). Branche `fix/pack-vars-keys-instances`, sur master `54db202`. Cadre du lot précédent réutilisé, pas recopié : `docs/superpowers/specs/2026-11-03-pack-management-frame.md` (sections 1 et 3 bis ; les décisions Q1, Q3, Q4, Q7 y sont celles qui comptent ici).

Lu : la spec, le cadre précédent, `armadai-rules`, `crates/armadai-model/src/pack_vars.rs` (lignes 1-200). Pas de Bash : aucune commande lancée ; `pack/effective.rs`, `agent_templates.rs` et les tests existants non relus, marqués « à vérifier » quand ils comptent.

## 1. Règles qui s'appliquent (avec source)

- Pureté : toute la logique de substitution reste dans `armadai-model/src/pack_vars.rs` (serde, serde-saphyr, serde_json, thiserror, schemars seulement) ; le binaire (`pack/effective.rs`) lit les couches (environnement, `vars.local.yaml`, `vars.yaml`) et appelle le modèle. Source : `CLAUDE.md`, Where things live ; Dimitri, 2026-11-03 (Q4, « Pur dans le modèle »).
- Substitution au `link` seulement, dans la copie de travail ; `.armadai/` garde `{{var.nom}}` ; `validate` accepte le littéral. Source : Dimitri, 2026-11-03 (Q1, « Au link seulement ») ; spec, section 3.
- Syntaxe `{{var.name}}`, environnement `ARMADAI_VAR_<NOM>`, `\{{` littéral, valeur jamais re-rendue. Source : Dimitri, 2026-11-03 (Q3) ; `pack_vars.rs`.
- Texte exact des nouvelles erreurs (collision de clés, variable absente dans une clé) : proposé par le plan, validé par Dimitri à sa relecture du plan ; golden relus ligne à ligne ensuite. Pas de nouveau code de diagnostic du validateur (erreurs et lignes de rapport de `link`). Source : `armadai-rules`, Diagnostics, sorties et manifeste (Dimitri, 2026-10-27) ; spec, section 3.
- Mutations : une par test, nommée ; le test existant `render_value_touches_strings_only` fige « clés intactes » et doit être inversé (R4 de la spec) ; mutation type : désactiver le rendu des clés, chaque test ajouté doit passer au rouge ; fixtures à valeurs distinctes (une collision se teste avec deux clés d'origine distinctes) ; `contains` suspect : comparer des lignes entières ou la clé exacte. Source : `armadai-rules`, Tests et mutations ; `.claude/CLAUDE.md`.
- Variables d'environnement dans les tests : sous `armadai_core::test_support::env_lock()` (non réentrant), valeur de référence lue sous le verrou, 5 à 8 passes ; `--bin armadai` ou `--test <nom>`, `--no-fail-fast`. Source : `armadai-rules`, Tests et mutations.
- Aucun réseau dans les tests ; rien de privé dans le dépôt public (fixtures et doc génériques : `data-dir`, `/srv/data`) ; le pack réel du critère 10 reste hors dépôt. Source : Dimitri, 2026-11-03 (« Les deux »).
- Essai réel (critère 10) : `HOME` et `ARMADAI_CONFIG_DIR` isolés. Source : `armadai-rules`, Environnement.
- Golden existants inchangés (aucune sortie existante ne doit bouger : clés sans variable et valeurs non textuelles identiques, critère 8). Source : spec, critères 8 et 9 ; `armadai-rules` (golden relus contre la spec).
- Livrable doc : `docs/wiki/starter-packs.md`, section Variables (portée, « frontmatters jamais substitués » devient « sauf `vars:` d'une instance »). Pratique, pas règle écrite.
- Barrière (fmt, clippy `-D warnings`, test, builds `armadai-core` et `armadai-model`, code retour lu sans tube), Git et PR (Conventional Commits en anglais, PR en français, ouvrir et s'arrêter, revue indépendante avant fusion). Source : `armadai-rules`, Barrière et Git et PR. `frozen/armadai-audit/` : seulement si un crate dont il dépend change ; `armadai-model` n'en fait pas partie (cadre précédent, à vérifier).
- Schémas JSON `schemas/v1/` : aucun champ de `project.yaml` ne change, donc rien à régénérer (à confirmer par le plan ; `gen_schemas` si un type bouge).

## 2. Contradictions et corrections de spec

- **Critère 10 (correction à faire par l'analyste, non faite ici).** « Aucune des sorties compilées ne contient un chemin de machine » est faux par construction : les sorties compilées portent les valeurs résolues, donc le chemin de machine de Dimitri y apparaît (c'est le but). Intention, selon `lead` : aucun `{{var.` résiduel dans les sorties compilées, et le pack lui-même ne contient plus aucun chemin de machine. À reformuler ainsi, avec la liste des 6 sites (5 clés, 1 instance) vérifiée dans le pack.
- Spec, section 1 : « 6 chemins en dur (5 clés OpenCode, 1 instance) » est un fait tiré d'un essai antérieur : à revérifier au pack à l'essai réel, pas au plan.
- Rien dans `armadai-rules` n'est contredit par `CLAUDE.md` ou `.claude/CLAUDE.md`.

## 3. Habitudes observées, non enregistrées

- `scope()` de `pack_vars.rs` traite `templates/*.md` en `Body` comme `agents/*.md` ; un `vars:` n'est donc substitué aujourd'hui nulle part. Le lot l'étend aux seuls `agents/*.md` portant `from:` (spec) : ne pas l'étendre aux frontmatters des templates sans décision.
- La technique de #546 (édition de texte ciblée ou parse + ré-émission) n'est pas une décision de Dimitri : c'est un choix d'architecte, borné par les critères 6, 7 et le risque R2 (valeur insérée comme chaîne, positions de diagnostics, `.armadai/` inchangé). Pas de question, à moins que le plan ne trouve que l'option sûre change un golden.

## 3 bis. Décisions de Dimitri sur ce cadre (2026-11-04, transmises par `lead`)

- Q1, variable vide dans une clé : « Refuser une clé élargie (Recommandé) » (Dimitri, 2026-11-04). Dans une clé de `native`, une variable qui rend une chaîne vide (définie vide, comme absente) fait refuser `link`, avant toute écriture. Règle retenue par `lead` pour le plan : « toute variable vide dans une clé = erreur », sans détection de motif (`/**`). Note : c'est la portée de l'option C de la question d'origine (toute valeur vide dans une clé), pas celle de l'option A ; le plan suit la règle de `lead`. Critère à ajouter à la spec : une variable vide dans une clé fait refuser `link`, aucune sortie écrite ni modifiée (la même variable vide dans une valeur reste acceptée).
- Q2, critère 10 : réglée avant la réponse ; l'analyste l'a reformulé selon l'option A (pas de `{{var.` résiduel dans les sorties, le pack sans chemin de machine).

Les questions de la section « Questions pour Dimitri » ci-dessous sont closes.

## 4. Décisions à prendre

Au plan (architect, validées par Dimitri à sa relecture) : textes des deux erreurs ; message de R3 (clé de premier niveau de `native.<cible>` contenant un `.` après rendu : accepter si le message nomme la clé d'origine ou reste compréhensible) ; technique de réécriture des `vars:` ; atomicité (critères 4 et 5 : l'erreur doit tomber avant toute écriture ; le plan nomme où `render_files` est appelé par rapport aux écritures).

## Questions pour Dimitri

### Q1. Une variable définie mais vide, dans une clé : accepter ou refuser ?
Bloque : le plan, tâche `render_value` (comportement d'une clé qui rend vide ou élargie).
Constat : la décision « erreur pour une clé » couvre la variable **absente**. Une variable **présente** avec la valeur `""` (ex. `ARMADAI_VAR_DATA_DIR=`) rend `"{{var.data-dir}}/**"` en `"/**"` : la règle de permission s'élargit à tout le disque, exactement le risque R1, sans erreur. Une clé entièrement vide (`"{{var.x}}"` avec `x=""`) est pire : une clé `""`.
- Option A (recommandée) : refuser seulement une clé qui rend **vide** (`""`) ; une valeur vide qui laisse du texte (`"/**"`) passe (on ne peut pas deviner l'intention). Message à proposer au plan.
- Option B : accepter tout ; la spec ne couvre que l'absent, le cas vide est documenté dans le wiki comme responsabilité de l'auteur du pack.
- Option C : refuser toute variable à valeur vide utilisée dans une clé (même si du texte reste). Plus sûr pour R1, mais refuse un usage légitime (suffixe optionnel).

Critère à ajouter à la spec si A ou C : une clé rendue vide fait refuser `link`, aucune sortie écrite.

### Q2. Le critère 10 corrigé : à confirmer
Bloque : l'écriture du critère d'acceptation par l'analyste, puis la préparation de l'essai réel par le validator.
Constat : voir section 2. Je ne tranche pas pour Dimitri, mais la reformulation est celle de `lead`.
- Option A (recommandée) : « aucune sortie compilée ne contient `{{var.` et le pack ne contient plus aucun chemin de machine » ; les chemins de machine dans les sorties compilées sont attendus.
- Option B : en plus, comparer les sorties compilées à une liste de valeurs attendues fournie par Dimitri pendant l'essai (plus fort, dépend de Dimitri pendant l'essai).
