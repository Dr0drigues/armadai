# Plan B5 — Flotte ArmadAI dev

> **Statut** : À faire
> **Effort** : 🟢 Faible (1 jour)
> **Version cible** : 0.11.0

---

## Contexte

Le projet utilise déjà une équipe de 6 agents spécialisés définis dans `.claude/CLAUDE.md` pour le développement. L'objectif est de formaliser cette équipe comme un starter pack installable dans `~/.config/armadai/`, utilisable sur n'importe quel projet Rust.

## Équipe actuelle

| Agent | Rôle | Scope |
|-------|------|-------|
| dev-lead | Coordinateur | Analyse, délégation, synthèse |
| core-specialist | Core & orchestration | src/core/, src/parser/ |
| provider-specialist | Providers & linker | src/providers/, src/linker/, src/model_registry/ |
| cli-specialist | CLI & UX | src/cli/, templates/ |
| ui-specialist | TUI & Web | src/tui/, src/web/ |
| qa-specialist | Tests & CI | tests, clippy, CI |
| release-manager | Releases | Versioning, tags, branches |

## Structure du starter pack

```
starters/armadai-dev/
├── pack.yaml
├── agents/
│   ├── dev-lead.md
│   ├── core-specialist.md
│   ├── provider-specialist.md
│   ├── cli-specialist.md
│   ├── ui-specialist.md
│   ├── qa-specialist.md
│   └── release-manager.md
└── prompts/
    ├── rust-conventions.md       # Conventions Rust (edition 2024, clippy, fmt)
    └── armadai-architecture.md   # Architecture du projet, modules, feature flags
```

### `pack.yaml`

```yaml
name: armadai-dev
description: "ArmadAI development fleet — Rust agent orchestrator team"
agents:
  - dev-lead
  - core-specialist
  - provider-specialist
  - cli-specialist
  - ui-specialist
  - qa-specialist
  - release-manager
prompts:
  - rust-conventions
  - armadai-architecture
```

### Config orchestration recommandée

Après `armadai init --pack armadai-dev`, le `armadai.yaml` généré inclura :

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: dev-lead
  teams:
    - agents:
        - core-specialist
        - provider-specialist
        - cli-specialist
        - ui-specialist
        - qa-specialist
  max_depth: 3
```

Note : `release-manager` reste hors teams (invoqué manuellement pour les releases).

## Implémentation

### Étape 1 — Créer les agents Markdown

Reprendre les descriptions de `.claude/CLAUDE.md` et les formaliser au format agent ArmadAI :
- H1 : nom de l'agent
- `## Metadata` : model (latest:high), provider (anthropic)
- `## System Prompt` : rôle, scope, responsabilités, conventions

Chaque agent doit être **générique Rust** (pas spécifique à ArmadAI) pour être réutilisable.

### Étape 2 — Créer les prompts

- `rust-conventions.md` : Edition 2024, clippy strict, fmt, feature flags, error handling
- `armadai-architecture.md` : Pattern orchestration, modules, execution flow (ce prompt est spécifique à ArmadAI)

### Étape 3 — Valider

- `armadai init --pack armadai-dev` dans un dossier vide → config générée
- `armadai link` → fichiers de linking générés pour tous les agents
- `armadai run dev-lead "hello"` → l'agent répond (test basique)

## Délégation

| Étape | Agent |
|-------|-------|
| Agents & prompts | @cli-specialist |
| Validation linking | @provider-specialist |
| Tests | @qa-specialist |

## Critères de complétion

- [ ] Starter pack `armadai-dev` créé dans `starters/`
- [ ] 7 agents au format Markdown valide
- [ ] 2 prompts inclus
- [ ] `pack.yaml` valide
- [ ] `armadai init --pack armadai-dev` fonctionne
- [ ] Config orchestration générée automatiquement
- [ ] CI verte
