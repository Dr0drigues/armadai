# Étape 4, lot A : adaptateur OpenCode fidèle (OpenCode 2.x)

Date : 2026-10-21 · Statut : à valider · Suite de : étapes 1 (#464), 2 (#468), 3 (#472) · Sources : conversation avec Dimitri du 2026-10-21, reconnaissance documentaire et code d'OpenCode (rapport `recon-oc-docs.md`, branche `dev` d'`anomalyco/opencode` 2.1.2, commit 681156c8, doc `opencode.ai/docs`).

## 1. Intention

Dimitri veut utiliser OpenCode avec une **équipe** déclarée une fois dans `.armadai/` : une équipe qui l'aide à affiner une idée, anticipe (normes en place, outils attendus, sécurité, maintenabilité, conventions de code), propose maquettes et architecture, développe ou l'épaule, valide, puis itère jusqu'à une solution propre. Ce parcours, et les normes qui le cadrent, seront décrits par un pack (lot B) puis par des packs de normes (lot C) ; une visualisation de la documentation fonctionnelle, des tâches et de la backlog viendra plus tard (lot D).

Le lot A pose la fondation : qu'un projet `.armadai/` arrive **réellement** dans OpenCode, avec ses rôles, sa délégation, ses skills et ses flows. Aujourd'hui ce n'est pas le cas :

- le coordinateur et la liste de l'équipe sont écrits dans `.opencode/instructions.md`, qu'**aucune version d'OpenCode ne lit** ;
- `model` est écrit sans fournisseur (`claude-opus-4-6`), ce que la 2.x ignore ;
- les skills par agent, la délégation et les flows ne sont pas transmis (`not-compiled`).

Le lot A doit être conçu pour accueillir le lot B : délégation contraignante, retours en arrière dans les flows, skills de normes attachés aux agents, règle « l'implémenteur demande avant de choisir une convention ».

## 2. Objectifs, non-objectifs, anti-objectifs

**Objectifs**
1. Un adaptateur OpenCode sur le modèle canonique (comme l'adaptateur Claude de l'étape 2), qui remplace la sortie héritée pour la cible `opencode`.
2. Chaque élément du modèle (agent, coordinateur, skill, flow, politique, automation, capacités, modèle) reçoit une ligne honnête dans le rapport de conformité : `compiled`, `approximated` (avec ce qui manque) ou `not-compiled` (avec la raison).
3. Cible : **OpenCode 2.x uniquement** (décision de Dimitri : il utilise 2.1.2 ; la doc en ligne décrit cette ligne).
4. Essais réels dans OpenCode 2.1.2 par Dimitri, comme pour Claude Code à l'étape 2.

**Non-objectifs (ce lot)**
- Compatibilité OpenCode 1.x (stable 1.19 : outil `task`, `subagent_depth` à la racine, autre API de plugin).
- Codex, Copilot ; la table de modèles embarquée (#455).
- Gate par plugin TypeScript (`tool.execute.before`) : la permission native suffit pour la délégation ; les automations restent `not-compiled`.
- Commandes OpenCode (`.opencode/commands/`) générées depuis les flows.
- Les lots B, C, D.

**Anti-objectifs (ce qui serait un échec même si « ça marche »)**
- Écrire un fichier qu'OpenCode ignore et le compter `compiled`.
- Écrire une clé de frontmatter inconnue d'OpenCode : elle est **envoyée au fournisseur comme option de modèle** (vérifié dans le code). Jamais `skills:`, `tier:`, `tags:`, `title:` dans un agent.
- Écraser ce que l'utilisateur a écrit à la main dans `AGENTS.md` ou `opencode.json`.
- Toucher la configuration globale (`~/.config/opencode/`).
- Inscrire une table de modèles figée qui deviendrait fausse (les modèles gratuits d'OpenCode changent souvent).

## 3. Contraintes

- OpenCode 2.x lit : `.opencode/agents/*.md` (ou `agent/`), `opencode.json` (racine du projet), `AGENTS.md` (racine ; `CLAUDE.md` seulement en l'absence d'`AGENTS.md` — **correction du 2026-10-21, revue finale** : la doc « rules » le dit, mais le code 2.x, `packages/core/src/config/plugin/instruction.ts` au commit 681156c8, ne cherche que `AGENTS.md`), les fichiers de la clé `instructions`, les skills `SKILL.md` de `.opencode/skills/`, `.claude/skills/` et `.agents/skills/`. Toute affirmation sur le format doit être revérifiée dans la doc 2.x ou le code `dev` avant d'être codée (règle de l'équipe : « vérifie chaque format natif dans la documentation officielle »).
- Frontmatter d'agent 2.x : `description` (requise), `mode` (`primary` | `subagent` | `all`), `model` (`fournisseur/modèle`), `temperature`, `top_p`, `steps`, `disable`, `hidden`, `color`, `permission`. **Attention** : en 2.x, un agent `hidden` disparaît de la liste des sous-agents proposée au modèle.
- Délégation 2.x : outil `subagent` (ex-`task`) ; restriction par agent avec `permission.task` (motifs, dernière règle gagnante), appliquée à l'appel. Profondeur : `experimental.subagent_depth` (défaut 1). L'utilisateur peut toujours invoquer un agent avec `@agent` (hors gate, comme sous Claude Code).
- Outil shell : `shell` en 2.x (`bash` converti) ; permissions `allow` / `ask` / `deny`, par agent, avec motifs ; un `deny` reste appliqué même en mode `--auto`.
- Fichiers partagés avec l'utilisateur (`AGENTS.md`, `opencode.json`) : patchés comme `CLAUDE.md` et `settings.json` à l'étape 2 (`compile/patch.rs` : écriture atomique, propriétaire conservé au mieux, bloc ou clés possédés seulement, enregistrés dans le manifeste, retirés par `unlink`).
- Barrière, mutation obligatoire pour chaque test, revue indépendante avant fusion (règles du dépôt).

## 4. Ce qui a été décidé avec Dimitri

### 4.1 Modèles
- **Par défaut, aucun modèle** n'est écrit dans un agent OpenCode : l'agent hérite du modèle choisi dans la session (Dimitri utilise surtout les modèles gratuits). Ligne de rapport : `model: approximated (inherited from the OpenCode session)` quand l'agent a un `tier`.
- **Correspondance optionnelle** palier → `fournisseur/modèle`, déclarée par l'utilisateur ; appliquée quand elle existe.
- `native.opencode.model` (identifiant exact) est toujours écrit tel quel et l'emporte.

### 4.2 Cible de version
OpenCode 2.x seulement ; une sortie conforme à la doc en ligne et au code de la branche `dev`.

### 4.3 Portée de la délégation sous `strict` (décision : option b)
La règle s'applique à **toute** délégation du projet, y compris depuis les agents principaux intégrés d'OpenCode : en plus des agents qu'ArmadAI écrit, `opencode.json` du projet reçoit une règle `permission.task` pour les agents intégrés **`build` et `plan`** (clés possédées `agent.build.permission.task` et `agent.plan.permission.task`), égale à celle du fil principal (agents d'entrée des flows, ou toutes les cibles des flows avec un coordinateur, plus `free_agents`). Basculer sur `build` ou `plan` par Tab ne contourne donc pas la topologie. Les agents principaux que l'utilisateur a définis lui-même (hors `.armadai/`) ne sont pas touchés ; le rapport le dit. Sous `off`, aucune de ces clés n'est écrite.

### 4.4 Correspondance palier → modèle (décision : configuration utilisateur)
Elle se déclare dans la configuration utilisateur `~/.config/armadai/config.yaml` (personnelle : les modèles gratuits de chacun diffèrent), pas dans `project.yaml`. Forme proposée : `model_tiers: { opencode: { fast: <fournisseur/modèle>, pro: …, max: … } }`. La sortie de `link` dépend donc de la machine pour cette clé seulement ; le rapport indique la source (`model: compiled (from model_tiers in your config)`). Un palier sans correspondance donne l'héritage de session (§4.1). L'édition de `config.yaml` par `config starters-dir` (#487) ne touche que `starters_dirs` : cette clé est préservée.

### 4.5 Choix de l'exécutant (révocables, signalés à Dimitri)
1. **Fichier partagé.** Le guide de l'équipe et des flows va dans un fichier **généré** (`.opencode/armadai-team.md`, possédé, retiré par `unlink`) référencé par la clé `instructions` d'`opencode.json` (entrée possédée dans le tableau), et **non** dans `AGENTS.md` : créer un `AGENTS.md` absent masquerait un `CLAUDE.md` écrit à la main pour OpenCode (qui ne lit `CLAUDE.md` qu'en l'absence d'`AGENTS.md`). Un `AGENTS.md` existant n'est jamais modifié.
2. **Rôle du coordinateur.** Son rôle et ses consignes de délégation vont dans **son propre fichier d'agent** (`mode: primary`). Le fichier partagé, injecté dans tous les agents (sous-agents, `build`, `plan`), ne décrit l'équipe et les flows que de façon neutre, pour qu'un sous-agent ne se prenne pas pour le coordinateur.
3. **`free_agents`.** Les noms s'appliquent tels quels à OpenCode (ex. `general`, `explore`) ; un nom inconnu de la cible (ex. `Explore`, `general-purpose`, noms Claude) donne une ligne de rapport `approximated` qui le nomme. Pas de traduction automatique entre cibles.
4. **Format des agents.** Clés du format documenté pour la 2.x (`permission`, `temperature`, …), que la 2.1.2 décode ; pas le schéma natif v2 en liste (`permissions`). À confirmer par l'essai réel.
5. **Skills.** Toujours copiées dans `.opencode/skills/`, même si `claude` est lié en même temps (OpenCode lit aussi `.claude/skills/` : doublon signalé par OpenCode, sans effet de bord). Pas de dépendance entre cibles.

## 5. Correspondance du modèle canonique (proposition de conception)

| Élément `.armadai/` | Sortie OpenCode 2.x | Rapport |
|---|---|---|
| Agent | `.opencode/agents/<id>.md` : `description`, `mode: subagent`, `temperature`, `permission`, corps = prompt ; aucune clé inconnue | `compiled` |
| Coordinateur (`coordinator:`) | son agent en `mode: primary`, avec son rôle et ses consignes de délégation dans son corps ; `opencode.json` : `default_agent: <id>` et l'entrée `instructions` du fichier généré (clés possédées) ; `.opencode/armadai-team.md` : équipe et flows en guide, neutre (§4.5) | `compiled` |
| Sans coordinateur | pas de `default_agent` ; le fichier généré décrit quand même l'équipe et les flows (guide neutre) ; le principal est `build` | flows `approximated` sous `off` |
| Skill | copiée dans `.opencode/skills/<id>/SKILL.md` | `compiled` |
| `skills:` d'un agent | `permission.skill` de l'agent : `"*": deny` puis `<id>: allow` pour chacune ; plus la consigne « charge la skill X » dans le prompt (comme pour Claude) | `compiled` |
| Flow | en guide dans `.opencode/armadai-team.md` ; liens, états, `on_failure` décrits ; boucles incluses | `approximated` (guide) ou `compiled` (strict, voir ligne suivante) |
| `policy.delegation: strict` | `permission.task` par agent : `"*": deny`, puis `allow` pour chaque cible permise (même règle que le gate Claude : successeurs par lien ou `on_failure` pour un agent, `free_agents` partout) ; la règle du fil principal sur le coordinateur **et** sur `build` et `plan` (§4.3) ; `experimental.subagent_depth` dans `opencode.json` assez grand pour la plus longue chaîne de délégation des flows | `compiled (enforced by OpenCode permissions)` ; agents principaux de l'utilisateur hors `.armadai/` non couverts, dit dans le rapport |
| `policy.delegation: off` | aucune restriction `permission.task` | `approximated (guidance only)` |
| `capabilities` (read, write, shell, web, delegate) | permissions `deny` de l'agent sur les outils correspondants (`edit`, `shell`, `webfetch`/`websearch`, `task`/`subagent`), à vérifier outil par outil dans le code 2.x | `compiled` ou `approximated` |
| Automations (hooks) | rien (plugin TypeScript hors lot) | `not-compiled (needs an OpenCode plugin)` |
| `native.opencode` d'un agent | fusionné dans son frontmatter, après les clés générées (l'utilisateur en est responsable) | `compiled` |
| `native.opencode` du projet | fusionné dans les clés possédées d'`opencode.json` | `compiled` |

Les fichiers hérités que l'ancienne sortie écrivait (`.opencode/instructions.md`, ignoré par OpenCode) sont retirés par le chemin de migration du manifeste déjà en place à l'étape 2 (fichier « stale »).

## 6. Laissé au constructeur

- L'organisation du code (un `compile/opencode.rs` à côté de `compile/claude.rs`, la mutualisation éventuelle du calcul des cibles permises avec `claude_adapter/policy_gate.rs`).
- La formulation exacte du bloc `AGENTS.md` et des consignes de prompt.
- Le calcul de `experimental.subagent_depth` (plus longue chaîne, borné).
- La correspondance précise capacités → noms d'outils 2.x, à établir dans le code.
- Les étalons de sortie et la liste des tests ; le format des lignes de rapport, en suivant celui de l'adaptateur Claude.

## 7. Critères d'acceptation

1. Sur les 7 packs embarqués, `link --target opencode` ne produit que des fichiers et clés qu'OpenCode 2.x lit, sans aucune clé de frontmatter inconnue (test qui compare chaque clé écrite à la liste 2.x).
2. Rapport de conformité complet et honnête ; étalons par pack.
3. `unlink` retire exactement ce que `link` a écrit, y compris `.opencode/armadai-team.md` et les clés possédées d'`opencode.json` (`default_agent`, l'entrée `instructions`, `agent.build/plan.permission.task`, `experimental.subagent_depth`) ; un `opencode.json` écrit à la main garde tout le reste ; un `AGENTS.md` ou `CLAUDE.md` n'est jamais touché.
4. Essai réel par Dimitri dans OpenCode 2.1.2 sur un projet de test (un pack ou projet dédié avec `strict`, aucun des 7 packs ne l'active) : le coordinateur est l'agent par défaut et se présente avec son équipe ; sous `strict`, une délégation hors flow est refusée par OpenCode, depuis le coordinateur comme depuis `build`, et une délégation permise passe ; un agent avec `skills:` ne charge que ses skills ; aucun modèle imposé par défaut.
5. Barrière complète, mutations rouges, revue indépendante.

## 8. Risques

| # | Risque | Parade |
|---|---|---|
| R1 | La 2.x bouge vite (branche `dev`) : un nom de clé ou d'outil change | Reverifier chaque clé dans le code `dev` au moment du lot ; essai réel par Dimitri avant fusion |
| R2 | `permission.task` en `deny` par défaut bloque un usage légitime (agents intégrés `general`, `explore`) | `free_agents` et la même règle que le gate Claude ; le rapport nomme les cibles permises |
| R3 | Clés inconnues envoyées au fournisseur | Test sur la liste blanche des clés écrites |
| R4 | `AGENTS.md` / `CLAUDE.md` existants écrits à la main | Jamais touchés : fichier généré + clé `instructions` (§4.5) |
| R5 | La règle sur `build`/`plan` (§4.3) surprend : ArmadAI change le comportement d'agents intégrés | Seulement sous `strict`, clés possédées retirées par `unlink`, ligne de rapport explicite |
| R6 | Le patch JSON de `compile/patch.rs` ne sait peut-être pas posséder une clé imbriquée (`experimental.subagent_depth`, `agent.build.permission.task`) ni une entrée d'un tableau (`instructions`) | À vérifier en premier par le constructeur ; étendre `patch.rs` si besoin, avec les mêmes gardes que pour `settings.json` |
