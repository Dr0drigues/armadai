# Architecture - ArmadAI

## Vision

ArmadAI est un compilateur déclaratif d'équipes d'agents IA. Une équipe (agents, prompts, skills,
coordinateur, topologie de délégation) est décrite une fois, en Markdown ou en YAML, puis compilée
par `armadai link` vers les fichiers natifs de Claude Code, Codex, OpenCode et GitHub Copilot.

ArmadAI n'exécute pas d'agents : c'est l'assistant cible qui les exécute. Il n'y a ni moteur
d'exécution, ni TUI, ni interface web, ni stockage. Le recentrage (ce qui a été retiré, gardé ou
gelé) est décrit dans
[la spec de l'étape 0](docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md). Le
code retiré reste récupérable depuis le commit `9b02047` (tag `pre-v1-declaratif` une fois poussé).

---

## Choix techniques

| Aspect | Choix | Justification |
| --- | --- | --- |
| Langage | Rust (édition 2024) | Binaire unique, portable, sans runtime |
| Format agent | Markdown, ou YAML (`.armadai/agents.yaml`) | Lisible, versionnable |
| Sortie | Fichiers natifs de chaque assistant | L'assistant reste maître de l'exécution |
| Réversibilité | Manifeste de `link` (`.armadai/link-manifest.yaml`) | `unlink` retire exactement ce que `link` a écrit |
| Réseau | Aucun pour `link` | Compilation hors ligne et reproductible |
| Secrets | SOPS + age | Chiffrement champ par champ, diff-friendly |
| Configuration | Une seule (aucune feature cargo optionnelle) | Une seule barrière, identique en local et en CI |

---

## Flux

```
définitions (.md / agents.yaml) + prompts + skills + config projet
        │  parser/, agent_source.rs, project.rs        (armadai-core)
        ▼
modèle en mémoire (Agent, ProjectConfig, OrchestrationConfig)
        │  validation : références, orchestration, link.coordinator
        ▼
un linker par cible (linker/claude.rs, codex.rs, opencode.rs, copilot.rs)   (armadai)
        │  résolution du modèle (latest:* → id concret, hors ligne)
        ▼
fichiers natifs + .armadai/link-manifest.yaml
```

Le seul composant qui intervient pendant une session de l'assistant est le hook
`__claude-policy-gate` (Claude Code, `PreToolUse`) : il refuse une délégation que la topologie
déclarée n'autorise pas (voir `docs/wiki/policy-gate.md`).

---

## Crates du workspace

| Crate | Rôle |
| --- | --- |
| `crates/armadai-core` | Le domaine : modèle d'agent et parser (`agent.rs`, `agent_decl.rs`, `agent_source.rs`, `parser/`), config projet (`project.rs`), prompts, skills, starter packs et skills intégrés (`include_dir`), résolution des modèles (`model_resolution.rs`, `model_aliases.rs`, `model_updater.rs`), types déclaratifs d'orchestration (`orchestration/`, dont `policy.rs` pour le gate), validation de packs et de projets (`pack_validation.rs`), isolation d'environnement pour les tests (`test_support.rs`, feature de dev `test-support`). |
| `crates/armadai` | Le binaire (pas de `lib.rs`) : `cli/` (commandes), `linker/` (un linker par cible, `manifest.rs`, `model_resolution.rs`), `claude_adapter/policy_gate.rs` (le hook), `skills_registry/`, `starters_registry/`, `templates/`, tests d'intégration et étalons de sortie de `link` dans `tests/`. |
| `crates/armadai-providers` | Lecture seule du cache local du catalogue models.dev (`model_registry/`) et table outil → fournisseur (`factory.rs`). Aucun accès réseau. |
| `crates/armadai-secrets` | Gestion des secrets SOPS + age. |

`frozen/armadai-audit/` contient l'ancienne commande `audit`, gelée : hors du workspace, hors de
la CI, non maintenue (voir son README).

### Accès réseau

- `link`, `unlink`, `new`, `list`, `inspect`, `validate`, `models` : aucun.
- `init --pack <nom>` : synchronise les registres distants de starters seulement si le pack est
  introuvable localement.
- `skills sync` / `skills add` : clonent des dépôts GitHub de skills.
- `update` : télécharge la dernière version publiée.

---

## Cibles de `link`

| Cible | Fichiers par agent | Fichier racine (coordinateur) |
| --- | --- | --- |
| `claude` | `.claude/agents/<agent>.md` | `.claude/CLAUDE.md` |
| `codex` | `.codex/agents/<agent>.toml` + `.codex/config.toml` | `.codex/AGENTS.md` |
| `opencode` | `.opencode/agents/<agent>.md` | `.opencode/instructions.md` |
| `copilot` | `.github/agents/<agent>.agent.md` | `.github/copilot-instructions.md` |

Les prompts et skills du projet sont copiés sous `prompts/` et `skills/` dans le répertoire de la
cible. La sortie des quatre cibles sur un projet témoin est figée par un test d'étalon
(`crates/armadai/tests/link_golden.rs`).

Deux mécanismes de délégation coexistent, à ne pas confondre : le protocole de marqueurs
`<!--ARMADAI_DELEGATE-->` injecté par les linkers dans le fichier racine, et le gate `PreToolUse`
compilé depuis la topologie déclarée (`orchestration.policy`).

`OrchestrationPattern { Direct, Blackboard, Ring, Hierarchical, Auto }` et les champs
`orchestration`, `triggers`, `ring_config` d'un agent sont des types de déclaration conservés pour
le parser, `inspect`, la validation et le gate ; ils ne pilotent plus aucune exécution.

---

## Format d'un agent (.md)

```markdown
# Code Reviewer

## Metadata
- provider: claude
- model: latest:pro
- temperature: 0.3
- tags: [dev, review, quality]
- stacks: [rust, typescript]

## System Prompt

Tu es un expert en revue de code...

## Instructions

1. Comprendre le contexte du changement
2. Identifier les bugs potentiels et failles de sécurité

## Output Format

Revue structurée : bugs, sécurité, performance, style.
```

Référence complète : `docs/wiki/agent-format.md` (sections, champs de métadonnées et ce que
`link` en fait) et `docs/wiki/declarative-agents.md` (format YAML).

---

## Configuration

### Répertoire utilisateur

```
~/.config/armadai/
├── config.yaml          # Réglages utilisateur (starters_dirs, defaults…)
├── providers.yaml       # Endpoints et listes de modèles (non sensible)
├── agents/              # Bibliothèque d'agents globale
├── prompts/             # Fragments de prompts
├── skills/              # Skills (dont les skills intégrés)
├── starters/            # Starter packs extraits
├── skills-registry/     # Cache du registre de skills GitHub
├── models-cache.json    # Cache du catalogue models.dev, s'il a été écrit par une version antérieure
└── projects.json        # Projets enregistrés par link (models list / check --all)
```

Résolution : `$ARMADAI_CONFIG_DIR`, puis `$XDG_CONFIG_HOME/armadai`, puis `$HOME/.config/armadai`.

### Projet

Recherche en remontant depuis le répertoire courant : `.armadai/config.yaml` (préféré), puis
`armadai.yaml` / `armadai.yml` (ancien format, avec un message de migration).

Résolution des agents, prompts et skills : `.armadai/{type}/`, puis `{type}/`, puis
`~/.config/armadai/{type}/`.

### Couches

1. Valeurs par défaut (impl `Default`)
2. `~/.config/armadai/config.yaml`
3. Variables d'environnement : `ARMADAI_PROVIDER`, `ARMADAI_MODEL`, `ARMADAI_TEMPERATURE`,
   `ARMADAI_STARTERS_DIRS`

---

## Gestion des secrets

`armadai config secrets init` génère une clé age et un `config/providers.sops.yaml` chiffré ;
`armadai config secrets rotate` les renouvelle ; `armadai config providers` affiche l'état. Depuis
le retrait de l'exécution, aucune autre commande ne lit ces clés.

---

## Suite prévue

Un schéma versionné avec validateur et table des symboles, `armadai migrate`, et des adaptateurs
qui produisent un rapport de conformité par cible sont **prévus, pas encore écrits**. Voir la spec
de l'étape 0, sections 6.2 et 8.
