# Étape 1 — Modèle canonique, validateur et `migrate`

Date : 2026-10-12 · Statut : mergée (#464) · Suite de : `2026-10-09-step0-retrait-surfaces-design.md` (étape 0, terminée) · Décisions de conception : brainstorm des 2026-10-08/09 (mémoire `project_cap_v1_declaratif_2026-10-08`)

## 1. Objectif

Remplacer l'ancien modèle (huit lecteurs permissifs, quatre identités par agent, champs sans lecteur) par **un modèle déclaratif unique, strict et versionné**, avec :
1. un **crate pur** `armadai-model` : types, chargement d'un dossier `.armadai/`, écriture canonique, **validateur** (table des symboles, références résolues, erreurs positionnées), **JSON Schema** généré ;
2. `armadai validate` sait valider un projet au nouveau format ;
3. `armadai migrate` convertit un projet à l'ancien format vers le nouveau, avec rapport, sauvegarde et mode `--dry-run`.

**Hors périmètre (étapes suivantes)** : `link` sur le nouveau modèle, l'adaptateur Claude Code, le rapport de conformité, le hook de gate réécrit, l'émission de hooks / permissions / MCP (étape 2) ; création guidée, modèles de flows, starters réécrits, export Mermaid (étape 3) ; Codex / OpenCode / Copilot (étape 4). L'ancien modèle de `armadai-core` **n'est pas modifié** à l'étape 1 : `link`, `list`, `inspect`, `new`, `init`, `extract`, `models` continuent de l'utiliser jusqu'à l'étape 2 ; `migrate` en est le seul nouveau consommateur.

## 2. Décisions déjà prises (rappel, non rediscutées)

Format fichiers typés dans `.armadai/` + manifeste racine avec `schema_version` ; YAML strict, modèle indépendant de la syntaxe ; flow = graphe (steps nommées + liens structurés), liste linéaire en raccourci ; états `done`/`failed` extensibles par step, typés ; `when: failed` + raccourci `on_failure` ; gates = liens (délégation) + 5 capacités `allow`/`ask`/`deny` ; 6 événements de hooks portables, action « exécuter une commande » ; bloc `native` par cible + `.armadai/native/<cible>/`, collision = erreur ; paliers de modèle abstraits ; mode strict partout sauf sous `native` ; réécriture sémantique (pas de conservation des commentaires) avec sauvegarde et avertissement ; bascule d'un coup, sans conserver l'ancien modèle à terme.

## 3. Choix tranchés pour cette spec (à valider)

| # | Choix | Pourquoi |
|---|---|---|
| C1 | Crate `crates/armadai-model` (serde, `serde_yaml_ng`, `thiserror`, `schemars` ; ni clap, ni tokio, ni réseau) | Pur, testable seul ; le garde `cargo build -p armadai-model` rejoint la CI |
| C2 | Manifeste `.armadai/project.yaml` ; sa présence **définit** « projet au nouveau format » | `config.yaml` et `armadai.yaml` sont pris par l'ancien format |
| C3 | **Une seule identité par agent : `id`** = nom de fichier (kebab-case). Elle sert de référence partout, de nom de fichier natif, de `subagent_type` du gate | Remplace titre H1 / clé / stem / slug |
| C4 | Agent = Markdown avec **frontmatter YAML strict** ; le corps est le prompt (Markdown libre) | Proche des formats natifs ; `migrate` concatène les anciennes sections (prompt système, instructions, format de sortie, contexte) comme le faisait `link` |
| C5 | Découverte **par répertoire** (`agents/`, `skills/`, `flows/`, `automations/`), tri alphabétique, pas de liste dans le manifeste | Une source de vérité, pas de liste à resynchroniser |
| C6 | Concept de **prompt supprimé** : `migrate` compose les fragments dans le corps des agents ; les prompts autonomes deviennent des **skills** | Constat du spike : commandes = skills dans les quatre cibles |
| C7 | `orchestration:` ancien → un **flow** + `policy` du manifeste (§6) | Les liens du graphe sont les délégations autorisées |
| C8 | `armadai validate` à deux modes (nouveau si `project.yaml`, ancien sinon, avec invitation à lancer `migrate`) | Pas de rupture avant l'étape 2 |
| C9 | JSON Schema généré par `schemars`, committé dans `schemas/v1/`, test d'obsolescence | Règle l'issue #456 ; nouvelle dépendance (§9) |
| C10 | Hooks : pas de filtre (`matcher`) en étape 1 | Le bloc `native` couvre le besoin fin ; à rouvrir avec les adaptateurs |
| C11 | `migrate` copie dans le projet les agents de la **bibliothèque utilisateur** qu'il référence | Le nouveau modèle est local au projet |

## 4. Structure du dossier

```
.armadai/
  project.yaml            # manifeste
  agents/<id>.md          # frontmatter + prompt
  skills/<id>/SKILL.md    # standard inter-outils, dossier inchangé
  flows/<id>.yaml
  automations/<id>.yaml
  native/<cible>/<chemin> # fichiers copiés tels quels (étape 2)
```

### 4.1 `project.yaml`
```yaml
schema_version: 1            # entier ; seule valeur acceptée : 1
name: mon-projet             # optionnel, informatif
targets: [claude, opencode]  # cibles de link par défaut : claude|codex|copilot|opencode
outputs: { claude: .claude } # optionnel : dossier de sortie par cible
coordinator: lead            # optionnel : id d'agent, son prompt devient le fichier racine
policy:                      # optionnel
  delegation: strict         # off (défaut) | strict
  free_agents: [explorer]    # ids autorisés sans lien
native: { claude: {} }       # optionnel, opaque
```

### 4.2 `agents/<id>.md`
```markdown
---
description: Écrit le code            # obligatoire, une ligne
title: Développeur                    # optionnel (affichage)
tier: pro                             # fast | pro | max (défaut pro)
temperature: 0.4                      # optionnel
tags: [dev]
skills: [conventions-rust]            # ids de skills du projet
capabilities: { read: allow, write: ask, shell: ask, web: deny, delegate: allow }
native: { claude: { model: claude-sonnet-4-5 } }
---
Tu es le développeur…
```
- `id` : `[a-z0-9][a-z0-9-]{0,63}`, égal au nom de fichier sans `.md` ; **pas** de clé `id` dans le frontmatter.
- `capabilities` absent ou clé absente = « laisser le défaut de la cible » (rien n'est émis).
- Le modèle concret passe par `native.<cible>.model` ; `tier` est le seul réglage portable.

### 4.3 `skills/<id>/SKILL.md`
Frontmatter : `name` (= nom du dossier), `description` (obligatoire, ≤ 1024 caractères), `license`, `compatibility`, `metadata`. Une clé inconnue est un **avertissement** (le format appartient à d'autres outils), pas une erreur.

### 4.4 `flows/<id>.yaml`
```yaml
description: Du code à la livraison
steps:                                  # id d'étape -> définition
  premier-jet: { agent: developpeur }
  revue:       { agent: relecteur, states: [approved, rejected], on_failure: corrections }
  corrections: { agent: developpeur }
  livraison:   { agent: livreur }
links:
  - { from: premier-jet, to: revue }                    # sous-entendu : quand done
  - { from: revue, to: corrections, when: rejected }
  - { from: corrections, to: revue }
  - { from: revue, to: livraison, when: approved }
```
- **Raccourci linéaire** : `sequence: [developpeur, relecteur, livreur]` (exclusif avec `steps`/`links`) ; un élément `{ id: x, agent: y }` permet de répéter un agent ; il se développe en graphe.
- États : chaque step a `done` et `failed` ; `states` en ajoute ; un `when` doit appartenir aux états de la step `from` (défaut `done`).
- `on_failure: <step>` ≡ lien `when: failed` ; au plus un chemin d'échec par step (doublon = erreur).
- Les boucles sont permises ; un lien d'une step vers elle-même est une erreur.

### 4.5 `automations/<id>.yaml`
```yaml
event: before-tool      # session-start | session-end | before-tool | after-tool | on-prompt | on-stop
run: ./scripts/check.sh # commande, non vide
description: …          # optionnel
native: {}              # optionnel, opaque
```

### 4.6 Bloc `native`
Carte `{ claude|codex|copilot|opencode: <YAML libre> }` acceptée sur le manifeste, les agents et les automations. Le validateur contrôle **seulement** les clés de cible. Aucune vérification de contenu (« natif, non vérifié »).

## 5. Validateur

Deux couches.
1. **Structure** : désérialisation stricte (`deny_unknown_fields` sauf sous `native`), clés dupliquées refusées, type et domaine de chaque champ.
2. **Sémantique** (table des symboles) : identifiants uniques par genre ; `agents` référencés par les steps, `coordinator`, `free_agents` existent ; skills référencées par un agent existent ; `from`/`to` des liens sont des steps du flow ; `when` ∈ états de la step source ; `on_failure` sans doublon ; `targets`/clés `native`/`outputs` ∈ cibles supportées (liste unique = celle de `LinkTarget`, vérifiée par un test) ; alertes : step inaccessible, flow sans point d'entrée, agent sans description issue d'une dérivation.

**Diagnostic** : `{ severity (error|warning), code, file, path (chemin YAML), line, column, message, hint }`. Position : ligne/colonne fournies par `serde_yaml_ng` (`Error::location`) ; pour le frontmatter d'un agent, décalée à la ligne réelle du fichier. Pas de bibliothèque de diagnostics : un rendu texte maison `fichier:ligne:colonne: error[code]: message` + indice. Codes stables, documentés dans le wiki.

`armadai validate [chemin]` : code de sortie 1 s'il y a au moins une erreur ; `--json` émet les diagnostics.

## 6. `armadai migrate`

`armadai migrate [--path DIR] [--dry-run] [--force]`.

1. **Lecture** de l'ancien projet par les chargeurs actuels de `armadai-core` (config projet `.armadai/config.yaml` | `armadai.yaml` | `armadai.yml`, agents `.md`, agents déclaratifs `agents.yaml` + fragments, skills, prompts).
2. **Écriture** du nouveau format dans `.armadai/` ; **copie des originaux** dans `.armadai/.migrated/<horodatage>/` avant toute écriture ; refus si `project.yaml` existe déjà, sauf `--force`.
3. **Rapport** sur la sortie standard : fichiers écrits, champs **abandonnés** avec la raison, avertissements (commentaires perdus, valeurs dérivées).

### 6.1 Correspondance (d'après l'inventaire de l'étape 0)

| Ancien | Nouveau |
|---|---|
| Agent `.md` : H1, `## System Prompt`, `## Instructions`, `## Output Format`, `## Context` | `id` = slug du H1 (collision = suffixe numérique signalé) ; corps = sections concaténées avec leurs titres ; `title` = H1 |
| Description (1ʳᵉ ligne non vide du prompt, comme `link`) | `description` (dérivée si absente, avertissement) |
| `temperature` (0,7 = valeur par défaut) | `temperature` seulement si ≠ 0,7 |
| `model: latest:*` / alias | `tier` ; modèle concret → `native.<cible>.model` selon le `provider` (anthropic/claude → claude ; openai/codex → codex ; autre → abandonné + rapport) |
| `tags` | `tags` |
| `provider`, `stacks`, `scope`, `model_fallback`, `command`, `args`, `max_tokens`, `timeout`, `cost_limit`, `rate_limit`, `context_window`, `mode`, `orchestration`, `triggers`, `ring_config`, `## Pipeline` | **abandonnés** (aucun lecteur utile ou moteur retiré), listés par agent dans le rapport |
| Agent déclaratif (`agents.yaml` + fragments + `defaults`) | agent `.md` avec le prompt **composé** ; `description` déclarée conservée |
| `prompts:` du projet | skills (`skills/<nom>/SKILL.md`, nom et description du frontmatter ; `apply_to` abandonné) |
| `skills:` du projet | `skills/` (copie du dossier) ; `tools` du frontmatter abandonné |
| `link.target` | `targets: [<cible>]` ; `link.overrides.<cible>.output` → `outputs` |
| `link.coordinator` (titre H1 ou slug) | `coordinator: <id>` |
| `orchestration.coordinator` + `teams[{lead, agents}]` | un flow `orchestration` : steps = coordinateur, chefs d'équipe, agents ; liens coordinateur → chef → agent (agents sans chef rattachés au coordinateur) |
| `orchestration.policy`, `free_agents` | `policy.delegation`, `policy.free_agents` |
| `orchestration.enabled`, `pattern`, `routes`, limites, `defaults`, `routing`, `registries`, `sources` | **abandonnés**, listés dans le rapport |
| Clés inconnues des anciens fichiers | rapportées, jamais silencieuses |

Les packs (`pack.yaml`) ne sont pas des projets : ils restent tels quels jusqu'à l'étape 3.

## 7. Critères d'acceptation

1. `cargo build -p armadai-model` et le garde `armadai-core` (sans dépendance lourde) verts ; `armadai-model` ne dépend ni de `clap`, `tokio`, ni d'un client réseau.
2. Le validateur détecte chaque règle de §5 : un test **par code de diagnostic**, vérifié par mutation, avec position (ligne/colonne) assertée.
3. Aller-retour sémantique : pour chaque fixture, `charger(écrire(charger(x))) == charger(x)`.
4. `schemas/v1/*.schema.json` générés ; test d'obsolescence ; l'énumération des cibles est égale à celle de `LinkTarget`.
5. `migrate` : tests d'étalon sur au moins six anciens projets réels (starters `rust-dev`, `armadai-dev`, `fullstack` avec coordinateur et orchestration, `code-analysis-rust`, un projet à agents déclaratifs, l'exemple `demo-rust-team`) ; `validate` sur chaque sortie = aucune erreur ; idempotence (second passage refusé) ; sauvegarde présente ; `--dry-run` n'écrit rien (vérifié par comparaison d'arbres).
6. Aucune modification de `armadai-core` hors ajout éventuel de fonctions de lecture publiques nécessaires à `migrate` ; le crate gelé `frozen/armadai-audit` compile toujours ; instantanés de `link` inchangés.
7. `armadai validate` sur un ancien projet reste identique, plus l'invitation à migrer.
8. Documentation : page wiki « Format du projet » décrivant §4, codes de diagnostic, CHANGELOG, `CLAUDE.md`.

## 8. Risques

| # | Risque | Parade |
|---|---|---|
| R1 | `migrate` perd des données | Copie préalable, rapport exhaustif des champs abandonnés, `--dry-run`, étalons sur projets réels |
| R2 | Le modèle diverge des besoins des adaptateurs (étape 2) | `native` et `capabilities` couvrent les écarts ; on ne fige que `schema_version: 1` à la fin de l'étape 2 |
| R3 | Deux chemins de validation pendant un temps | Explicite (C8) ; l'ancien disparaît à l'étape 2 |
| R4 | Positions d'erreur imprécises dans le frontmatter | Test de position par code ; repli sur le début du frontmatter si la ligne n'est pas connue |
| R5 | Le crate gelé `audit` casse silencieusement | Aucun changement de signature de `armadai-core` (critère 6) |

## 9. Dépendance nouvelle

`schemars` (génération du JSON Schema). Absente de l'arbre aujourd'hui ; version exacte vérifiée au plan (publiée depuis au moins deux semaines, compatible `serde` et l'édition 2024). Repli si elle gêne : schéma écrit à la main + test de cohérence avec les types. Pas de bibliothèque de diagnostics ni de YAML à positions.

## 10. Suivi

Étape 2 : `link` sur ce modèle (remplace `LinkAgent`), adaptateur Claude Code, rapport de conformité, hook de gate lu depuis `flows/` et `policy`, table de modèles (#455), retrait de l'ancien modèle de `armadai-core` hors lecture de `migrate`. Les issues #456 et #458 sont concernées.
