# Étape 2 — `link` sur le nouveau modèle, adaptateur Claude Code, rapport de conformité

Date : 2026-10-12 · Statut : brouillon · Suite de : `2026-10-12-step1-model-validator-migrate-design.md` (étape 1, mergée #464) · Décisions de fond : brainstorm 2026-10-08/09 (mémoire `project_cap_v1_declaratif_2026-10-08`)

## 1. Objectif

Après l'étape 1, un projet migré n'est lu par aucune commande de génération. L'étape 2 **fait de `armadai-model` l'unique source de vérité de `link`** et livre le premier adaptateur complet (Claude Code) :
1. `link` lit `.armadai/project.yaml` + dossiers typés, pour les quatre cibles ;
2. adaptateur **Claude Code complet** : agents, skills, guidage de flow, hooks (automations), permissions (capabilities), gate de délégation, bloc `native`, passage natif ;
3. **rapport de conformité** par cible : pour chaque élément déclaré, `compiled` / `approximated` / `not compiled`, avec la raison ;
4. les autres commandes qui lisaient l'ancien modèle migrent (§6) ; l'ancien modèle sort du chemin de `link`, `list`, `inspect`, `validate`.

**Hors périmètre** : fidélité complète de Codex, OpenCode, Copilot (étape 4 : ils gardent leur sortie actuelle, construite depuis le nouveau modèle, avec un rapport qui dit ce qui n'est pas compilé) ; création guidée, nouveaux starters, export Mermaid (étape 3) ; table de modèles rafraîchie (#455) ; suivi d'état de flow à l'exécution (post-v1).

## 2. Choix tranchés pour cette spec (à valider)

| # | Choix | Pourquoi |
|---|---|---|
| S1 | `link` ne lit que le nouveau modèle ; un projet ancien reçoit « legacy format, run `armadai migrate` » (code de sortie 1) | Bascule d'un coup, pas de double chemin |
| S2 | Les quatre cibles restent liables : Claude = compilation complète ; Codex/OpenCode/Copilot = sorties actuelles alimentées par le nouveau modèle | Pas de régression de surface avant l'étape 4 ; leurs défauts connus (spike) sont étape 4 |
| S3 | Le rapport de conformité est la contrepartie de toute approximation : rien n'est dégradé en silence | Promesse produit : « guidage + garde-fous + rapport » |
| S4 | `capabilities` → `tools`/`disallowedTools` de l'agent Claude ; `ask` n'existe pas par agent → approximation signalée | Réalité de Claude Code |
| S5 | Automations → hooks de `.claude/settings.json`, fusion non destructive ; propriété des entrées tracée par le manifeste d'écriture (valeurs exactes), pas par un champ marqueur (champ inconnu non documenté côté Claude Code) | Ne jamais écraser les réglages de l'utilisateur |
| S6 | Gate de délégation : le hook `armadai __claude-policy-gate` est **installé par `link`** (c'était le trou du spike) et lit `flows/` + `policy` du nouveau modèle | Le gate existe enfin pour de bon |
| S7 | Paliers de modèle résolus par la table embarquée au moment de `link` ; surcharge par `native.<cible>.model` ; aucune requête réseau | Décision Q-modèles du brainstorm |
| S8 | `extract` est **retiré** (la bibliothèque utilisateur disparaît, C11) ; `init --pack` et `new` passent par le convertisseur de `migrate` (packs et modèles restent en ancien format jusqu'à l'étape 3) | Évite de réécrire les starters deux fois |
| S9 | Collision `native` (un fichier produit à la fois par ArmadAI et par `.armadai/native/claude/`) = erreur de `link`, aucun fichier écrit | Décision du brainstorm |
| S10 | Dans `armadai-core`, l'ancien modèle ne subsiste que pour ce que `migrate`, `init --pack` et `new` lisent ; le reste est supprimé (preuve par `grep`) | Pas de code mort |

## 3. Pipeline de `link`

`charger + valider` (`armadai-model::validate`, une erreur = arrêt avant toute écriture) → **plan de compilation par cible** (liste de `Artifact { path, content, kind, source }`) → vérification des collisions (`native`) → rapport → écriture via le manifeste existant (`linker/manifest.rs` : état d'écriture, `unlink` fiable). `--dry-run` affiche le plan et le rapport sans écrire ; `--json` imprime le rapport.

Options existantes conservées : `--target`, `--output`, `--agents`, `--force`, `--dry-run`. `--coordinator` et `--model` sont retirés (le coordinateur et les modèles sont déclarés dans le projet).

## 4. Adaptateur Claude Code

### 4.1 Fichiers produits
| Élément | Sortie |
|---|---|
| agent `<id>` | `.claude/agents/<id>.md` : frontmatter `name: <id>`, `description`, `model` (palier résolu ou `native.claude.model`), `tools` ; corps = prompt |
| skill `<id>` | `.claude/skills/<id>/` (copie du dossier) |
| coordinateur | `CLAUDE.md` racine (un seul des deux emplacements possibles, jamais `.claude/CLAUDE.md` en plus) : prompt du coordinateur + section « Delegation » générée depuis les flows (qui délègue à qui, états `when`), marquée ArmadAI pour une mise à jour idempotente |
| automations | `.claude/settings.json` → `hooks` (6 événements : session-start→`SessionStart`, session-end→`SessionEnd`, before-tool→`PreToolUse`, after-tool→`PostToolUse`, on-prompt→`UserPromptSubmit`, on-stop→`Stop`), commande = `run` |
| gate (`policy.delegation: strict`) | entrée `PreToolUse` (matcher `Agent`, nom de l'outil de délégation selon la doc ; `Task` n'est qu'un alias historique) appelant `armadai __claude-policy-gate` |
| passage natif | `.armadai/native/claude/<chemin>` copié tel quel ; `native.claude` (manifeste, agent, automation) fusionné dans le fichier correspondant |

### 4.2 Capabilities → outils
`read` → Read, Grep, Glob ; `write` → Edit, Write, NotebookEdit ; `shell` → Bash ; `web` → WebFetch, WebSearch ; `delegate` → Agent. `allow` : l'outil figure dans `tools` ; `deny` : il n'y figure pas (si au moins une capacité est `allow`/`ask`, `tools` est explicite ; sinon `tools` omis = défaut de Claude). `ask` : l'outil figure dans `tools` et le rapport signale « approximated: Claude Code asks per project/session permission, not per agent ».

### 4.3 Gate
Le hook lit `.armadai/` (remontée depuis `cwd`, repli sur le checkout principal d'un worktree git — comportement actuel conservé). Mode `strict` : une délégation est **autorisée** si l'agent cible figure parmi les steps d'un flow, le coordinateur ou `policy.free_agents` ; sinon refusée avec un message qui cite les agents permis. Mode `off` : aucun hook. Contrat : toute erreur de lecture = sortie vide (Claude Code poursuit le flux normal de permissions ; ce n'est pas une approbation). Refus = `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":…}}`. Le `subagent_type` d'un agent ArmadAI est son `id` (frontmatter `name: <id>`). Le hook n'écrit sur stdout que la décision JSON.

### 4.4 Fusion de `settings.json`
Aucun marqueur dans les objets de hook (champ supplémentaire non documenté par Claude Code). Le manifeste d'écriture (`linker/manifest.rs`) enregistre les entrées exactes ajoutées ; `link` retire (égalité de valeur) puis réécrit **seulement** celles-là, `unlink` de même ; si le manifeste est perdu, `link` n'ajoute pas un doublon d'une entrée identique déjà présente et le dit dans le rapport ; tout le reste est préservé en valeur, ordre des clés compris (nombres conservés tels quels, `1e2` pouvant s'écrire `1e+2`) ; seule la mise en forme JSON est normalisée, et seulement quand une valeur change (rapport : `settings.json reformatted`). Un fichier dont la valeur ne change pas n'est pas réécrit. Un `settings.json` illisible = erreur, rien d'écrit.

## 5. Rapport de conformité

Structure : pour chaque cible, une liste `{ element, status, reason }` avec `status ∈ compiled | approximated | not-compiled`. Exemples : `agent dev: compiled` ; `agent dev capability shell=ask: approximated (per-agent ask unavailable)` ; `flow livraison: approximated (rendered as guidance in CLAUDE.md, not enforced)` ; `automation check: not-compiled for codex (step 4)`. Les flows sont toujours « guidage » sauf pour la part de délégation appliquée par le gate (rapport : `enforced: delegation topology`). Rendu texte trié + `--json`. Aucune approximation sans ligne dans le rapport (test).

## 6. Autres commandes

| Commande | Changement |
|---|---|
| `list`, `inspect` | lisent le nouveau modèle |
| `validate` | mode projet = nouveau modèle ; ancien format = erreur « run migrate » ; mode pack inchangé |
| `unlink` | inchangé (basé sur le manifeste d'écriture) ; ne dépend plus de l'ancienne config |
| `init --project` | écrit `project.yaml` + dossiers vides ; `init --pack` / `new` : convertissent via le convertisseur de `migrate` et écrivent le nouveau format |
| `models` | inchangé (table et alias) |
| `extract` | retiré (S8) |
| `schemas/armadai.schema.json` (ancien) | supprimé ; clôt #456 |

## 7. Critères d'acceptation

1. `link` sur chacune des six fixtures migrées produit une sortie Claude conforme aux étalons (agents, skills, `CLAUDE.md`, `settings.json`) ; les étalons de `link` actuels sont remplacés, pas masqués.
2. Fusion de `settings.json` : un fichier utilisateur avec hooks et permissions propres est préservé ; relancer `link` est idempotent ; `unlink` retire exactement ce que `link` a ajouté.
3. Gate : tests sur payloads réels (`Task` avec `subagent_type` permis / refusé / libre / hors projet / worktree / entrée illisible = silence).
4. Rapport : chaque approximation connue (`ask`, flow en guidage, hooks absents pour une cible) a une ligne ; mutation qui la supprime = test rouge.
5. Collision `native` = erreur sans écriture.
6. Les trois autres cibles : `link` sur les fixtures migrées produit leur sortie actuelle et un rapport qui liste `not-compiled` pour ce qu'elles ne couvrent pas.
7. `grep` : plus aucun usage de `ProjectConfig`, `parser::parse_agent_file`, `agent_source` hors `migrate`, `init --pack`, `new` ; code supprimé de `armadai-core` sinon (S10) ; `armadai-core` build seul vert ; crate gelé `audit` toujours compilable (sinon le geler à nouveau avec copie).
8. Docs : wiki (link, format, gate, rapport), CHANGELOG, CLAUDE.md.

## 8. Risques

| # | Risque | Parade |
|---|---|---|
| R1 | Format exact des hooks / `tools` / `settings.json` de Claude Code évolue | Vérifié sur la doc officielle le 2026-10-12 (sous-agents, hooks, skills, mémoire, réglages) ; restent non vérifiés : syntaxe des règles `permissions`, tolérance aux champs inconnus d'un handler, validation d'un `subagent_type` inconnu ; un test de schéma de la sortie ; le rapport signale ce qui n'est pas vérifiable |
| R2 | Écraser les réglages utilisateur | Fusion marquée, test sur fichier utilisateur réaliste, `--dry-run` |
| R3 | Le gate bloque à tort et se fait désinstaller | Sortie vide en cas de doute (contrat actuel), message de refus actionnable |
| R4 | Retrait d'`extract` surprend | Annoncé dans CHANGELOG ; restaurable depuis le tag `pre-v1-declaratif` |
| R5 | Régression silencieuse des 3 autres cibles | Étalons par cible, rapport `not-compiled` |

## 9. Révisions de la revue indépendante (PR #465)

Les décisions R-B1…R-I11 du plan (`plans/2026-10-12-step2-link-claude.md`, section « Révisions ») complètent et corrigent cette spec : fichiers patchés hors `write_files` (`settings.json`, `CLAUDE.md` racine), `disallowedTools` au lieu de `tools`, gate à sémantique d'appelant et alias `Task`, entrées périmées, `.armadai/.gitignore`, modèles déterministes, inventaire des commandes. En cas de contradiction avec §3–§6, le plan prime. Corrections de texte : `--json` est une option nouvelle ; R2 = « fusion tracée par le manifeste » (pas de marqueur) ; `unlink` est modifié (§6).
