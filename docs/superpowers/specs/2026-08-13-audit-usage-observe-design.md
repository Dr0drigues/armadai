# Audit de l'usage observé — nourrir `--propose` avec ce qui tourne vraiment — Design

**Date** : 2026-08-13
**Statut** : validé (design), à implémenter
**Cible** : `armadai audit` / `--propose`. Prérequis : aucun (les briques existantes suffisent).

## Contexte

`armadai audit` se décrit comme un « adoption funnel », mais il ne mesure que le **déclaré** :
`audit/reverse/claude.rs` importe `.claude/agents/`, les skills et `CLAUDE.md`, puis
`audit/rules/` passe des règles statiques dessus. Rien ne mesure ce qui est **réellement
utilisé**.

En face, `armadai watch` + `claude_adapter/` savent déjà lire un transcript JSONL de Claude
Code et le mapper en `RunEvent` (`mapper.rs` corrèle `tool_use` name `Agent` → `tool_result`,
désambiguïse les sous-agents parallèles de même `subagent_type` par leur `description`,
remonte les tokens).

Les deux moitiés ne se parlent pas, et chacune est amputée là où ça compte :

- l'audit ne voit ni hooks, ni slash commands, ni `settings.json`, ni MCP, ni plugins ;
- `transcript.rs` ne lit que les entrées `assistant` et `user` (`_ => None`) et écrase tout
  `tool_use` non-`Agent` en `Block::Other` — le nom de l'outil est perdu, parce que le parseur
  ne sert qu'une vue live ;
- rien n'agrège : `watch` regarde **une** session, jamais l'historique ;
- l'accès à l'historique n'existe pas du tout. Le seul chemin vers un transcript est l'index du
  plugin (`claude-sessions.jsonl`, alimenté par le hook `SessionStart`), donc aveugle à tout ce
  qui précède l'installation du plugin. Aucun code ne touche `~/.claude/projects/`.

`generate_proposal(root, config)` ne reçoit que l'`ImportedConfig` statique et produit un pack
(agents `.md`, fragments de prompt dédupliqués, skills, `pack.yaml`). **Il ne propose aucune
orchestration** : ni `armadai.yaml`, ni coordinateur, ni pattern, ni routes C8.

## Faisabilité — mesurée, pas supposée

Relevé sur le projet ArmadAI lui-même : 59 sessions, 287 Mo de transcripts sous
`~/.claude/projects/-Users-bl209054-work-misc-armadai/`.

Chaque ligne porte bien plus que ce que `transcript.rs` en lit :

| Champ | Ce qu'il donne |
|---|---|
| `parentUuid` | l'arbre de délégation est **explicite** dans la donnée, pas à reconstruire |
| `attributionSkill` / `attributionPlugin` | attribution directe des skills et plugins, **par tour** |
| `cwd`, `gitBranch` | rattachement projet et branche (`cwd` est autoritatif) |
| `isSidechain` | séparation main-thread / sous-agent |
| `timestamp`, `effort`, `version` | fenêtre, effort de raisonnement, version du CLI |
| types `mode`, `permission-mode` | usage réel du mode plan et des permissions |

Deux métriques distinctes en sortent, et l'écart entre elles est le cœur du sujet :

| Skill | Invocations (`tool_use` Skill) | Tours attribués (`attributionSkill`) |
|---|---|---|
| superpowers:brainstorming | 20 | 423 |
| superpowers:writing-plans | 15 | 348 |
| superpowers:subagent-driven-development | 7 | 334 |
| armadai | 11 | 237 |

Compter les invocations donnait un funnel qui décroche (20 → 15 → 7) ; compter les tours
gouvernés donne un funnel stable (423 → 348 → 334). **C'est la seconde lecture qui est juste**,
et c'est elle qui doit alimenter la proposition.

Côté agents, sur le même relevé : `general-purpose` 319 invocations, `qa-specialist` 83,
`core-specialist` 70, `ui-specialist` 36, `cli-specialist` 16, `dev-lead` 9,
`provider-specialist` 8. Le coordinateur déclaré dans `.claude/CLAUDE.md` (`dev-lead`) est donc
contourné, et le principal exécutant (`general-purpose`) n'existe dans aucun fichier de
`.claude/agents/`.

## Décisions de cadrage (Dimitri, 2026-08-12)

1. **Finalité** : levier de migration. L'usage observé alimente `audit --propose`.
2. **Sortie** : filtrage du pack **et** topologie d'orchestration **et** routes C8.
3. **Frontière déterministe / LLM** : hybride. Tout ce qui est comptable reste déterministe et
   testable ; seul le **nommage** des routes et des tags de capacité passe derrière `--deep`.
   Sans `--deep`, la proposition sort complète mais sans bloc `routes`.
4. **Arbitrage déclaré / observé** : les deux côte à côte. Le yaml porte l'observé, l'intention
   divergente est conservée en commentaire avec les deux chiffres. L'arbitrage revient à
   l'humain qui relit.
5. **Source** (tranché en séance, forcé par la finalité) : découverte des transcripts par slug du
   `cwd` sous `~/.claude/projects/`. Un projet qu'on veut migrer n'a par définition pas le plugin
   ArmadAI installé, donc l'index du plugin ne peut pas être la source.

## Architecture

Nouveau module `crates/armadai/src/audit/usage/`, en miroir de `audit/reverse/` (qui lit le
déclaré) :

```
audit/usage/
  discovery.rs   cwd → ~/.claude/projects/<slug>/*.jsonl
  scan.rs        streaming ligne-à-ligne, jamais tout en mémoire
  facts.rs       UsageFacts : l'agrégat déterministe
```

Flux, greffé sur l'existant sans le tordre :

```
import_surfaces(root)  ──►  ImportedConfig   (déclaré, inchangé)
                                   │
usage::scan(root)      ──►  UsageFacts       (observé, nouveau)
                                   │
                    AuditContext { config, usage, settings }
                                   ├──► rules::run_rules   → findings U0x
                                   └──► generate_proposal(root, config, usage)
                                             ├── pack enrichi (lot 2)
                                             └── armadai.yaml (lot 3)
```

`UsageFacts` est strictement déterministe et sérialisable : par agent (invocations, tokens,
modèles observés, durées, échecs), par skill (tours attribués), l'arbre parent → enfants avec sa
profondeur et son parallélisme observé, les outils, la fenêtre temporelle et le nombre de
sessions.

**Définition de la fenêtre** : toutes les sessions trouvées pour le projet, sans filtre. La
fenêtre n'est donc pas un paramètre mais un **constat** — le `timestamp` le plus ancien et le plus
récent rencontrés pendant le scan. Elle est rapportée (dans le rapport et dans `USAGE.md`) pour
que le lecteur sache sur quoi porte la mesure, jamais pour la restreindre. Toute expression du
type « sur la fenêtre » dans ce document signifie « sur l'ensemble des sessions scannées ».

### Résolution du slug — stratégie à deux niveaux

La règle observée est le chemin absolu du projet avec `/` remplacé par `-`
(`/Users/bl209054/work/misc/armadai` → `-Users-bl209054-work-misc-armadai`). Le traitement exact
des autres caractères (`.`, `_`, espaces) n'est pas documenté publiquement, donc :

1. tenter la résolution directe par slug ;
2. si le dossier n'existe pas, balayer `~/.claude/projects/*/` et retenir les dossiers dont les
   entrées portent un `cwd` égal à la racine auditée.

Le champ `cwd` est dans la donnée et fait autorité — le slug n'est qu'un raccourci d'accès. Cette
double voie évite de dépendre d'une convention de nommage non spécifiée.

### Un seul changement dans du code existant

`transcript.rs` : `Block::Other` devient `Block::Tool { name }`. `mapper.rs` ignore déjà cette
variante, donc le Workroom est inchangé — mais le nom d'outil devient disponible.

## Lot 1 — Observation

`usage/{discovery,scan,facts}`, l'extension de `Block`, les findings, et une section « Usage
observé » dans le rapport (sessions scannées, fenêtre, top agents et skills).

Findings, préfixe `U` (les statiques sont `A`, le deep pass `D`), toutes déterministes :

| Code | Sév. | Détecte |
|---|---|---|
| `U01` | WARN | Asset déclaré, jamais invoqué sur la fenêtre — candidat à l'exclusion du pack |
| `U02` | INFO | Sous-agent invoqué mais non déclaré (`general-purpose`, `Explore`, `Plan` sont intégrés à Claude Code, pas des fichiers de `.claude/agents/`) |
| `U03` | WARN | Coordinateur déclaré ≠ racine observée des délégations |
| `U04` | INFO | Couverture de session d'un asset déclaré : part des sessions où il apparaît, rapportée sans jugement |

`U02` est la plus importante pour la migration : ArmadAI n'a pas d'équivalent intégré à
`general-purpose`. Sans le matérialiser en agent explicite, le pack généré perd le cheval de
trait et la flotte migrée ne ressemble pas à ce qui tournait.

Pas de règle sur les modèles : `rules/models.rs` couvre déjà les modèles dépréciés. L'écart entre
modèle déclaré et modèles observés va dans le pack (lot 2), pas dans une finding redondante.

Livrable autonome : ce lot répond déjà à la question « que fait-on vraiment des orchestrateurs
natifs sur ce projet ».

### Métriques par type d'asset

Les deux types d'assets ne se mesurent pas pareil, parce que la donnée disponible diffère :

- **skills** : tours attribués (`attributionSkill`), la métrique fiable ;
- **agents** : invocations (corrélation `tool_use` Agent → `tool_result`) et tokens du résultat.
  Les tours **internes** d'un sous-agent ne sont pas comptables : sur le relevé de référence,
  `isSidechain` est `false` partout, donc les transcripts de sous-agents ne sont pas dans ces
  fichiers. On mesure l'invocation et son résultat, pas le détail interne. C'est suffisant pour
  la topologie, et ça évite tout double comptage.

## Lot 2 — Pack enrichi

```rust
generate_proposal(root, config, usage: Option<&UsageFacts>)
```

L'`Option` est le point structurant : sans transcript (projet neuf, première session), la
proposition sort exactement comme aujourd'hui. L'usage améliore, il ne devient jamais un
prérequis.

Ce que l'usage change :

- `- model:` porte le modèle **réellement observé** (majoritaire) au lieu du mapping
  `native_model_to_tier` appliqué au déclaré ; repli sur le comportement actuel si l'agent n'a
  pas été observé.
- `- tags: [imported, hot|warm|cold]` par **tercile** — sur les invocations pour les agents, sur
  les tours attribués pour les skills — et `unused` pour zéro strict. Des quantiles plutôt que
  des seuils absolus : aucune constante arbitraire à justifier, et ça reste juste sur un projet
  de 5 sessions comme de 500. En dessous de 3 assets observés, les terciles n'ont pas de sens :
  on n'émet alors que `unused` (zéro) ou `hot` (non-zéro).
- Ordre des agents dans `pack.yaml` par volumétrie décroissante.
- Les agents de `U02` générés en stubs explicites.
- `.armadai-proposal/USAGE.md` : fenêtre observée, nombre de sessions, méthode de comptage, et la
  liste des décisions que l'usage a prises (tel modèle retenu, tel agent exclu). Sans ça, la
  proposition contient des choix inexpliqués — et une proposition qu'on ne peut pas auditer, on
  ne la relit pas, on la subit.

## Lot 3 — Topologie et routes

### Déduction du pattern

L'arbre observé se projette directement sur le schéma réel (`OrchestrationConfig`,
`TeamConfig { lead, agents }`) :

| Arbre observé | Déduction |
|---|---|
| Aucune délégation | `pattern: direct` |
| Racine → N agents (profondeur 1) | `pattern: hierarchical`, `coordinator`, agents à plat |
| Racine → lead → agents (profondeur ≥ 2) | `pattern: hierarchical` + `teams[].lead` / `teams[].agents` |

`max_concurrency` est mesurable : plusieurs `tool_use` name `Agent` dans un même message
assistant, c'est un fan-out parallèle. Le maximum observé donne la valeur.

Principe pour tout le reste : **n'écrire une clé que si l'observation apporte une information**.
`max_depth`, `timeout`, `max_iterations` gardent leur défaut et n'apparaissent pas. Un yaml
généré qui recopie les défauts les fige sans raison et donne l'illusion d'un choix.

### Le fichier produit

Exemple **illustratif** d'un projet à profondeur ≥ 2 (les chiffres sont ceux d'un cas fictif, pas
du relevé ArmadAI) :

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: claude        # observé — racine de 512 délégations
  # coordinator: dev-lead    # déclaré (.claude/CLAUDE.md) — 9 délégations
  #   ↑ décommenter pour suivre l'intention déclarée (finding U03)
  max_concurrency: 3         # max de fan-out parallèle observé
  teams:
    - lead: dev-lead
      agents: [qa-specialist, core-specialist]
```

Sur le relevé ArmadAI de référence, la sortie serait **plate** et non en `teams` : `dev-lead`
n'étant appelé que 9 fois contre 319 pour `general-purpose`, l'arbre observé est
`claude → spécialistes` sans intermédiaire. La hiérarchie déclarée apparaîtrait alors en
commentaire, conformément à l'arbitrage côte à côte. C'est exactement le cas où l'observé et le
déclaré divergent, et où le choix revient au relecteur.

Écrit par `writeln!`, comme `proposal.rs` génère déjà `pack.yaml`. Pas de `serde_yaml` : il
effacerait précisément les commentaires qui portent l'arbitrage.

### Les routes, sous `--deep`

Un second prompt embarqué à côté de `deep_auditor.md`, `route_namer.md` : responsabilité unique,
testable seul, sans polluer la persona d'audit. Il reçoit les agents avec leur volumétrie et un
échantillon des `description` de tâches réellement déléguées, il rend des groupes nommés
alimentant `orchestration.routes` (`BTreeMap<String, Vec<String>>`, consommé par
`agent_selection.rs`) et les tags de capacité.

Les excerpts passent **obligatoirement** par `sanitize_excerpt` (redaction A11 puis troncature,
dans cet ordre). Les descriptions de tâches sont du texte libre écrit en cours de session,
c'est-à-dire exactement l'endroit où un chemin ou un secret peut se trouver.

CLI absent ou réponse invalide : pas de bloc `routes`, un avertissement, le reste de la
proposition sort intact.

## Limites assumées

**Ring et Blackboard ne sont pas inférables** et ne seront jamais proposés. Le modèle `Task` de
Claude Code est un appel-retour arborescent : pas de cycle à observer pour un Ring, pas de
tableau partagé en écriture concurrente pour un Blackboard. Les deviner depuis la forme d'un
arbre serait de l'invention déguisée en mesure. La limite est écrite dans `USAGE.md`.

Les tours internes des sous-agents ne sont pas observables sur le relevé de référence (voir
« Métriques par type d'asset »).

## Tests

- **Unitaires** : `discovery` (résolution du slug **et** repli par `cwd`), `scan` (fixtures JSONL
  courtes), `facts` (agrégation, terciles, cas < 3 assets), déduction de pattern (arbres
  synthétiques : vide, plat, profond, parallèle), rendu du yaml (commentaires présents, clés par
  défaut absentes).
- **E2E boîte noire** : un test d'intégration `assert_cmd` lançant le binaire compilé sur un
  dossier de transcripts fixture via `ARMADAI_CLAUDE_PROJECTS_DIR` (même mécanique que
  l'`ARMADAI_SESSION_INDEX` des tests de `watch`), sur le modèle de
  `crates/armadai/tests/hook_stdout.rs`. Assertions sur les findings `U0x` et sur l'`armadai.yaml`
  généré.

  **Pas** un cas gaveldrop : l'adaptateur `Armadai` de `tests/gaveldrop.rs` est spécialisé pour les
  runs orchestrés — son `claims()` ne retient un cas que s'il porte un `pattern`, `build_command`
  produit toujours `run`, et toutes ses assertions portent sur des events JSON, que `armadai audit`
  n'émet pas. L'y faire entrer demanderait d'élargir l'adaptateur aux commandes arbitraires, un
  chantier hors sujet ici.
- **Features** : aucune nouvelle. Le module ne tire que `serde_json`, déjà présent, donc il reste
  non gated comme `audit/`. Clippy doit passer dans les 4 modes CI.

## Risques

- **Volumétrie** : 287 Mo pour 59 sessions sur le projet de référence. Le scan doit être en
  streaming ligne-à-ligne. Pas de `--since` au lot 1 (YAGNI) ; si le scan complet se révèle lent
  à l'usage, c'est le point de sortie naturel.
- **Format de transcript non spécifié** : les champs exploités (`parentUuid`,
  `attributionSkill`, `isSidechain`, `cwd`) sont observés, pas contractuels. Le scan doit tolérer
  leur absence sans échouer — un champ manquant dégrade la métrique correspondante, il ne casse
  pas l'audit. Même posture que `#[serde(default)]` sur le `roster` de `RunStarted`.
- **Nommage des routes** : la seule partie spéculative, isolée derrière `--deep` par
  construction.

## Hors périmètre

- Élargissement du `ReverseLinker` aux hooks, slash commands, `settings.json`, MCP et plugins
  (utile, mais c'est le **déclaré** — sujet distinct).
- Observatoire d'usage / analytics dans le TUI et le Web (option écartée en cadrage).
- Sources non-Claude Code (codex, copilot, gemini, opencode) : la découverte de transcripts est
  spécifique à Claude Code, comme `reverse/claude.rs` l'est déjà.
