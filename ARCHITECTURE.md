# Architecture - ArmadAI

## Vision

ArmadAI est un compilateur déclaratif d'équipes d'agents IA. Une équipe (agents, skills, flows,
automatisations, coordinateur, politique de délégation) est décrite une fois dans `.armadai/`, puis
compilée par `armadai link` vers les fichiers natifs de Claude Code, Codex, OpenCode et GitHub
Copilot.

ArmadAI n'exécute pas d'agents : c'est l'assistant cible qui les exécute. Il n'y a ni moteur
d'exécution, ni TUI, ni interface web, ni stockage. Le recentrage (ce qui a été retiré, gardé ou
gelé) est décrit dans
[la spec de l'étape 0](docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md). Le
code retiré reste récupérable depuis le commit `9b02047` (tag `pre-v1-declaratif`).

État : les étapes 0 à 3 sont mergées (retrait des surfaces, modèle canonique avec `validate` et
`migrate` #464, `link` sur ce modèle avec l'adaptateur Claude Code et le rapport de conformité
#468, création guidée et starters natifs #472). Leurs specs sont dans `docs/superpowers/specs/`.

---

## Choix techniques

| Aspect | Choix | Justification |
| --- | --- | --- |
| Langage | Rust (édition 2024) | Binaire unique, portable, sans runtime |
| Format agent | `.armadai/agents/<id>.md` : frontmatter YAML strict + prompt en corps | Lisible, versionnable, validé par schéma |
| Schéma | Versionné (`schema_version: 1`), JSON Schemas dans `schemas/v1/` | Un projet invalide est refusé avec un diagnostic localisé |
| YAML | `serde-saphyr` | Clés inconnues et doublons détectés |
| Sortie | Fichiers natifs de chaque assistant + rapport de conformité | L'assistant reste maître de l'exécution ; ce qui n'est pas compilé est dit |
| Réversibilité | Manifeste de `link` (`.armadai/link-manifest.yaml`) | `unlink` retire exactement ce que `link` a écrit |
| Réseau | Aucun pour `link` | Compilation hors ligne et reproductible |
| Configuration | Une seule (aucune feature cargo optionnelle) | Une seule barrière, identique en local et en CI |

---

## Flux

```
.armadai/ (project.yaml, agents/, skills/, flows/, automations/, native/)
        │  armadai_model::load_checked : chargement strict + validation   (armadai-model)
        ▼
modèle canonique en mémoire (Project) — erreurs : refus avec diagnostics
        │  compile/ : un plan par cible (claude.rs, legacy_targets.rs)    (armadai)
        ▼
plan = fichiers entiers + fichiers partagés à patcher + rapport de conformité
        │  linker::manifest::write_files (fichiers entiers), compile/patch.rs
        │  (CLAUDE.md racine, .claude/settings.json)
        ▼
fichiers natifs + .armadai/link-manifest.yaml + rapport (compiled / approximated / not-compiled)
```

Un projet au format hérité (`.armadai/config.yaml`, `armadai.yaml`) est refusé par toutes les
commandes de projet, qui renvoient vers `armadai migrate`. `migrate` (`crates/armadai/src/migrate/`)
lit ce format avec le parser hérité d'`armadai-core`, écrit le format canonique et garde une
sauvegarde dans `.armadai/.migrated/<horodatage>/`.

Le seul composant qui intervient pendant une session de l'assistant est le hook
`armadai __claude-policy-gate` (Claude Code, `PreToolUse`) : il refuse une délégation que les flows
de `.armadai/project.yaml` n'autorisent pas (voir `docs/wiki/policy-gate.md`).

---

## Crates du workspace

| Crate | Rôle |
| --- | --- |
| `crates/armadai-model` | Le modèle canonique, pur (ne dépend d'aucun autre crate du workspace) : types (`types.rs`), chargement (`load.rs`), validation et diagnostics (`check.rs`, `diag.rs`), écriture (`write.rs`), JSON Schemas (`schema.rs`, régénérés par `cargo run -p armadai-model --example gen_schemas`), gabarits de flows (`templates.rs`), export Mermaid (`mermaid.rs`). |
| `crates/armadai-core` | Le format hérité, lu par `migrate` (`agent.rs`, `agent_decl.rs`, `agent_source.rs`, `parser/`, `project.rs`, `orchestration/`), et ce que le binaire partage encore : skills et starter packs embarqués (`include_dir`), registres de starters, table statique des paliers de modèles (`model_resolution.rs`), registre des projets liés (`project_registry.rs`), isolation d'environnement pour les tests (`test_support.rs`, feature de dev `test-support`). |
| `crates/armadai` | Le binaire (pas de `lib.rs`) : `cli/` (commandes), `compile/` (un compilateur par cible, rapport, patchs de fichiers partagés), `linker/` (`manifest.rs` et les linkers Codex, Copilot et OpenCode qu'appelle `compile/legacy_targets.rs`), `migrate/`, `claude_adapter/policy_gate.rs` (le hook), `skills_registry/`, `starters_registry/`, `templates/` (gabarits d'agents), tests d'intégration et étalons dans `tests/`. |

`frozen/armadai-audit/` contient l'ancienne commande `audit`, gelée : hors du workspace, hors de
la CI, non maintenue (voir son README).

### Accès réseau

- `link`, `unlink`, `new`, `list`, `inspect`, `validate`, `graph`, `migrate`, `models` : aucun.
- `init --pack <nom>` : synchronise les registres distants de starters seulement si le pack est
  introuvable localement.
- `skills sync` / `skills add` : clonent des dépôts GitHub de skills.
- `update` : télécharge la dernière version publiée.

---

## Cibles de `link`

| Cible | Fichiers par agent | Coordinateur |
| --- | --- | --- |
| `claude` | `.claude/agents/<agent>.md` | bloc `armadai` du `CLAUDE.md` racine |
| `codex` | `.codex/agents/<agent>.toml` + `.codex/config.toml` | `.codex/AGENTS.md` |
| `opencode` | `.opencode/agents/<agent>.md` | `.opencode/instructions.md` |
| `copilot` | `.github/agents/<agent>.agent.md` | `.github/copilot-instructions.md` |

Les skills du projet sont copiées sous `skills/` dans le répertoire de chaque cible ; seul Claude
Code les précharge par agent. Pour Claude Code,
les automatisations deviennent des hooks fusionnés dans `.claude/settings.json`, et
`policy.delegation: strict` y installe le gate (`PreToolUse`, matcher `Agent|Task`). Codex,
OpenCode et Copilot reçoivent leur sortie d'avant l'étape 2, alimentée par le modèle canonique ;
ce qu'ils ne couvrent pas encore (flows, automatisations…) sort en lignes `not-compiled` du
rapport (étape 4). La sortie est figée par des tests d'étalon (`crates/armadai/tests/link_golden.rs`,
`link_migrated_golden.rs`).

Il n'y a plus de protocole de marqueurs de réponse : `link` n'écrit plus de `ARMADAI_DELEGATE`,
`ARMADAI_END` ni `ARMADAI_META`, et la délégation passe par le mécanisme natif de chaque outil.

---

## Format d'un agent (`.armadai/agents/<id>.md`)

```markdown
---
description: Revue de code : bugs, sécurité, performance, style
tier: pro
temperature: 0.3
tags: [dev, review]
skills: [rust-conventions]
capabilities:
  shell: deny
---
Tu es un expert en revue de code...
```

L'id est le nom du fichier. Référence complète (manifeste, skills, flows, automatisations,
diagnostics, migration) : `docs/wiki/project-format.md`. Le format hérité (`## Metadata`,
`agents.yaml`) est décrit dans les pages « Legacy » du wiki, pour `migrate` seulement.

---

## Configuration

### Répertoire utilisateur

```
~/.config/armadai/
├── config.yaml          # Réglages utilisateur (starters_dirs…)
├── skills/              # Skills (dont les skills intégrées, installées par `init`)
├── registry/            # Cache des registres distants de starter packs
├── starters/            # Packs intégrés extraits et packs de l'utilisateur
├── registries.yaml      # Sources de registres supplémentaires (facultatif)
├── skills-registry/     # Cache du registre de skills GitHub
├── projects.json        # Projets enregistrés par link (models list / check --all)
└── agents/, prompts/    # Bibliothèque héritée, lue seulement par migrate
```

Résolution : `$ARMADAI_CONFIG_DIR`, puis `$XDG_CONFIG_HOME/armadai`, puis `$HOME/.config/armadai`.

### Projet

Le projet est le dossier `.armadai/` qui contient `project.yaml`, cherché en remontant depuis le
répertoire courant (`validate [chemin]` lit le chemin donné, ou le répertoire courant). Agents, skills, flows et automatisations sont découverts dans ses
sous-dossiers ; aucune bibliothèque utilisateur n'est consultée.

### Couches

1. Valeurs par défaut (impl `Default`)
2. `~/.config/armadai/config.yaml`
3. Variable d'environnement `ARMADAI_STARTERS_DIRS` (répertoires de starters supplémentaires)

---

## Suite prévue

Étape 4 : des adaptateurs Codex, OpenCode et Copilot écrits sur le modèle canonique, qui compilent
ce qui sort aujourd'hui en `not-compiled` dans leur rapport de conformité.
