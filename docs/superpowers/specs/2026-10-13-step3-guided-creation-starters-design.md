# Étape 3 — Création guidée, starters natifs, modèles de flows, export Mermaid

Date : 2026-10-13 · Statut : brouillon · Suite de : étapes 1 (#464) et 2 (#468) · Décisions de fond : brainstorm 2026-10-08/09 (mémoire `project_cap_v1_declaratif_2026-10-08`)

## 1. Objectif

Après l'étape 2, tout ce que `armadai` lit est le modèle canonique, mais **créer** un projet reste hérité : `new` et les 7 starters sont écrits à l'ancien format et passent par le convertisseur de `migrate` à chaque `init --pack` / `new`. L'étape 3 rend la création **native, guidée et vérifiable** :
1. **Starters natifs** : les packs embarqués sont réécrits au format canonique (plus de conversion à l'init).
2. **Création guidée** : `armadai new agent|flow|automation|skill`, interactif ou par options, qui écrit un fichier valide et le valide avant de rendre la main.
3. **Modèles de flows** : un flow se crée depuis un gabarit (`pipeline`, `review-loop`, `plan-build-verify`) appliqué à des agents du projet.
4. **Export Mermaid** : `armadai graph [flow]` rend le graphe d'un flow en Mermaid (sortie standard).

**Hors périmètre** : fidélité de Codex/OpenCode/Copilot (étape 4), table de modèles (#455), `isolation`/`effort`/capacités fines (#469, #229, #226), UI no-code, suivi d'état à l'exécution.

## 2. Choix tranchés pour cette spec (à valider)

| # | Choix | Pourquoi |
|---|---|---|
| T1 | Un pack = un dossier `pack.yaml` (`name`, `description`) + un arbre canonique (`agents/<id>.md`, `skills/<id>/`, `flows/<id>.yaml`, `automations/<id>.yaml`, `project.yaml` optionnel) ; `init --pack` **copie** l'arbre sous `.armadai/` et valide | Plus de conversion ; le pack est un projet |
| T2 | Les packs **distants** à l'ancien format (sans `schema_version` dans leur `project.yaml`, avec `agents:` dans `pack.yaml`) restent installables par le convertisseur de `migrate` jusqu'à ce que leurs auteurs migrent ; c'est le seul chemin hérité conservé à l'init | Ne pas casser les registres tiers |
| T3 | Les agents d'exemple « relais CLI » du pack `fullstack` (`claude-cli-reviewer`, `echo-reviewer`, `gemini-reviewer`) sont **supprimés** ; `cli-generic` et les modèles `tdd-*` ne gardent plus `## Pipeline` | `run` et Gemini n'existent plus |
| T4 | `armadai new` devient `new agent|flow|automation|skill` (sans argument : menu interactif) ; l'ancienne forme `new <nom>` disparaît | v1 non publiée ; une seule forme |
| T5 | Les 12 modèles d'agents embarqués sont réécrits au format canonique (frontmatter strict, `description`, `tier`) ; le dossier de modèles du disque garde la priorité | Cohérence avec les starters |
| T6 | Gabarits de flows = fonctions pures dans `armadai-model` (`flow_from_template(kind, agents) -> Flow`), validées par le validateur ; pas de fichiers de gabarit | Testables, pas de syntaxe à inventer |
| T7 | `armadai graph [flow] [--format mermaid]` : Mermaid `flowchart TD`, états en étiquettes d'arêtes, `failed` en pointillés, sans argument tous les flows ; sortie seule sur stdout | Besoin « voir mon flow » ; format unique pour l'instant |
| T8 | Toute commande de création valide le projet entier après écriture et **annule** (supprime ce qu'elle a créé) si le résultat introduit une erreur | Jamais de projet cassé par un assistant |
| T9 | Les modes interactifs utilisent `dialoguer` (déjà présent) ; chaque question a une option non interactive équivalente (testable, scriptable) | Tests sans terminal |

## 3. Formats

### 3.1 Pack natif
```
starters/<nom>/
  pack.yaml          # name, description
  project.yaml       # optionnel : targets, coordinator, policy (schema_version: 1)
  agents/<id>.md  skills/<id>/SKILL.md  flows/<id>.yaml  automations/<id>.yaml
```
`init --pack <nom>` : refuse si `.armadai/project.yaml` existe (sans `--force`) ; copie sans écraser un fichier existant (avertissement) ; crée `project.yaml` par défaut (`targets: [claude]`) si le pack n'en a pas ; écrit `.armadai/.gitignore` ; valide.

### 3.2 `new`
- `new agent [id] [--template T] [--description D] [--tier fast|pro|max] [--skills a,b] [-i]` : écrit `.armadai/agents/<id>.md` ; `id` normalisé (kebab-case) ; refus si le fichier existe.
- `new flow [id] [--template pipeline|review-loop|plan-build-verify] --agents a,b,c [--description D]` : voir §3.3.
- `new automation [id] --event E --run CMD [--description D]`.
- `new skill [id] --description D` : `.armadai/skills/<id>/SKILL.md` avec squelette.
- Interactif : chaque commande pose ses questions manquantes (agents existants proposés pour un flow ; aperçu Mermaid avant écriture).

### 3.3 Gabarits de flows (agents `a,b,c` dans l'ordre donné)
- `pipeline` : `a → b → c` (`sequence`).
- `review-loop` (≥ 2 agents : auteur, relecteur, [livreur]) : auteur → relecteur ; relecteur `states: [approved, rejected]` ; `rejected` → auteur ; `approved` → livreur (absent : fin).
- `plan-build-verify` (3 agents) : planificateur → constructeur → vérificateur ; vérificateur `on_failure` → constructeur.
- `fan-out` (≥ 2 agents : un chef + N ≥ 1) : chef → chaque autre agent (`done`), dans l'ordre donné ; ajouté après le dogfooding (un chef qui distribue à ses spécialistes, que `pipeline` ne sait pas dire).
- `review-loop` écrit `start: <auteur>` (l'auteur a le lien `rejected` entrant) ; un flow peut déclarer `start` pour lever l'ambiguïté de son entrée (`ambiguous-entry`).
Nombre d'agents inadéquat = erreur claire.

### 3.4 Mermaid
```
flowchart TD
  premier_jet["premier-jet (developpeur)"]
  premier_jet --> revue
  revue -- rejected --> corrections
  revue -. failed .-> corrections
```
Identifiants de nœuds assainis (`-` → `_`), étiquettes entre guillemets échappés, déterminisme (ordre des steps puis des liens).

## 4. Critères d'acceptation

1. Les 7 packs (moins les 3 agents CLI) sont au format natif ; `init --pack <nom>` pour chacun produit un projet que `validate` accepte sans erreur et que `link --target claude` compile ; étalons.
2. Un pack distant ancien format s'installe encore (test avec un faux registre local) ; un pack natif invalide est refusé avec diagnostics.
3. `new agent|flow|automation|skill` : chemins nominaux, collisions, id invalide, projet absent, annulation (T8, test par comparaison d'arbres), options non interactives complètes.
4. Les gabarits de flows : chacun produit un flow qui passe `validate` sans erreur ni avertissement ; erreurs de nombre d'agents ; tests par mutation.
5. `graph` : sortie Mermaid exacte (étalons pour pipeline, review-loop avec boucle et `failed`, flow à `sequence` développé) ; flow inconnu = liste des flows ; aucun projet = erreur claire.
6. Les 12 modèles d'agents sont au format canonique ; `new agent --template <chacun>` valide ; plus aucune mention de `## Pipeline`.
7. `grep` : le convertisseur de `migrate` n'est plus appelé que par `migrate` et par l'installation d'un pack distant ancien format ; `new` ne l'appelle plus.
8. Docs : wiki (`getting-started`, `templates`, `starter-packs`, `project-format` §création, `graph`), CHANGELOG, CLAUDE.md ; barrière complète, `cargo build -p armadai-core` et `-p armadai-model` verts ; crate gelé `audit` compile.

## 5. Risques

| # | Risque | Parade |
|---|---|---|
| R1 | Les packs réécrits perdent du contenu utile des anciens | Réécriture mécanique par `migrate` des packs existants puis relecture humaine agent par agent ; diff relu |
| R2 | Un assistant interactif écrit un projet invalide | T8 : validation + annulation ; options non interactives testées |
| R3 | Mermaid invalide (caractères spéciaux dans les ids/étiquettes) | Assainissement + test sur ids piégeux ; étalons |
| R4 | `new <nom>` supprimé surprend | CHANGELOG « Changed » ; message d'erreur qui indique `new agent <nom>` |
| R5 | Un pack distant exotique casse l'installation | Chemin hérité conservé (T2), erreurs de conversion nommées |

## 6. Révisions de la revue indépendante

Les décisions R-B1…R-m10 du plan (`plans/2026-10-13-step3-guided-creation-starters.md`, section « Révisions ») complètent et corrigent cette spec et priment en cas de contradiction : règle d'entrée d'un flow (étape sans lien entrant inconditionnel), extraction des starters par remplacement, garde d'écriture dédiée à `new`, annulation par comparaison avant/après avec restauration sous `--force`, modèles posés par la structure (pas de substitution YAML), détection du format de pack par `format: 1` dans `pack.yaml` pour toute origine, copie par liste blanche, réécriture manuelle du contenu d'`armadai-authoring`, `cli-generic` supprimé (11 modèles), Mermaid à nœuds préfixés `n_` et étiquettes `-->|"x"|`.
