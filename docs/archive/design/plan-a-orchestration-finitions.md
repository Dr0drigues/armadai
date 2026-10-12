# Plan A — Finitions orchestration (A1-A3)

> **Statut** : À faire
> **Effort** : 🟢 Faible (1-2 jours)
> **Version cible** : 0.10.7 ou 0.11.0

---

## A1 — Validation orchestration dans `link.rs`

### Contexte

Le plan d'orchestration (§7.2) prévoit que `armadai link` valide la config orchestration avant de générer les fichiers de linking. Actuellement, seul `run.rs` fait cette validation.

### Fichiers impactés

- `src/cli/link.rs` — Ajouter l'appel à `validate_config()`
- `src/core/orchestration/mod.rs` — `validate_config()` existe déjà (lignes 225-294)

### Implémentation

1. Après le chargement du `ProjectConfig` dans `link.rs`, vérifier si `orchestration.enabled == true`
2. Si oui, appeler `validate_config(&orchestration_config)`
3. En cas d'erreur, bloquer le link avec un message explicite (liste des erreurs de validation)
4. En cas de succès, poursuivre le linking normal

### Comportement attendu

```
$ armadai link
❌ Orchestration config invalid:
  - Coordinator 'unknown-agent' not found
  - Agent 'missing' in team 1 not found
```

### Tests

- Test unitaire : link avec config orchestration invalide → erreur
- Test unitaire : link avec config orchestration valide → succès
- Non-régression : link sans section orchestration → pas de changement

---

## A2 — Exemples `examples/orchestration-patterns/`

### Contexte

Le plan (§12.6) prévoit un dossier avec 4 sous-exemples, un par pattern d'orchestration.

### Structure

```
examples/orchestration-patterns/
├── direct/
│   ├── .armadai/config.yaml        # pattern: direct, 1 agent
│   └── agents/
│       └── analyst.md
├── blackboard/
│   ├── .armadai/config.yaml        # pattern: blackboard, 3 agents parallèles
│   └── agents/
│       ├── frontend-dev.md
│       ├── backend-dev.md
│       └── devops.md
├── ring/
│   ├── .armadai/config.yaml        # pattern: ring, 3 agents séquentiels
│   └── agents/
│       ├── writer.md
│       ├── reviewer.md
│       └── editor.md
└── hierarchical/
    ├── .armadai/config.yaml        # pattern: hierarchical, coordinator + teams
    └── agents/
        ├── coordinator.md
        ├── team-lead.md
        ├── specialist-a.md
        └── specialist-b.md
```

### Configs YAML

**Direct** :
```yaml
orchestration:
  enabled: true
  pattern: direct
```

**Blackboard** :
```yaml
orchestration:
  enabled: true
  pattern: blackboard
  max_rounds: 3
  token_budget: 50000
```

**Ring** :
```yaml
orchestration:
  enabled: true
  pattern: ring
  max_laps: 3
  consensus_threshold: 0.75
```

**Hierarchical** :
```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: coordinator
  teams:
    - lead: team-lead
      agents:
        - specialist-a
        - specialist-b
  max_depth: 3
```

### Agents

Chaque agent doit être minimal mais fonctionnel : H1 (nom), `## Metadata` (model, provider), `## System Prompt` (description du rôle). Privilégier des agents génériques réutilisables comme exemple pédagogique.

---

## A3 — Activer orchestration dans `demo-rust-team`

### Contexte

Le fichier `examples/demo-rust-team/.armadai/config.yaml` contient une config orchestration commentée (lignes 14-25).

### Action

Décommenter la section orchestration :

```yaml
orchestration:
  enabled: true
  pattern: hierarchical
  coordinator: rust-reviewer
  teams:
    - agents:
        - security-auditor
        - rust-test-writer
        - refactor-advisor
  max_depth: 3
```

### Vérification

- La config passe `validate_config()` sans erreur
- `doc-writer` n'est pas dans les teams (par design — agent standalone)
- Le coordinateur `rust-reviewer` n'est pas dans un team

---

## Critères de complétion

- [ ] A1 : `link.rs` valide la config orchestration
- [ ] A1 : Tests unitaires ajoutés
- [ ] A2 : 4 sous-dossiers dans `examples/orchestration-patterns/`
- [ ] A2 : Chaque exemple a une config YAML valide et des agents fonctionnels
- [ ] A3 : Config `demo-rust-team` décommentée et validée
- [ ] CI verte (clippy + tests dans les 2 modes de features)
