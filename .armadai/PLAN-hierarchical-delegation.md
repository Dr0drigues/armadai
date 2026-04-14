# Plan — Hierarchical Delegation & Marker Append

## Contexte

Le linker doit injecter le contexte de coordination dans le fichier racine du provider
(CLAUDE.md, GEMINI.md, etc.) pour que le modèle global connaisse sa flotte d'agents et
délègue systématiquement. En parallèle, on restructure l'orchestration multi-équipes.

## Worktrees terminés (à merger)

- [x] **Core-specialist** (`agent-a768e3ad`) — `lead` → `coordinator` dans TeamConfig + champ `name`
- [x] **Provider-specialist** (`agent-a70eece7`) — `WriteMode` enum + `MarkerAppend` dans les 5 linkers
- [x] **QA-specialist** (`agent-a86c8c90`) — schemas `armadai.schema.json` + `pack.schema.json`

## TODO

### 1. Intégration des worktrees
- [ ] Créer une branche feature `feature/hierarchical-delegation`
- [ ] Cherry-pick/merge core-specialist (lead → coordinator)
- [ ] Cherry-pick/merge provider-specialist (WriteMode)
- [ ] Cherry-pick/merge QA-specialist (schemas)
- [ ] Résoudre les conflits éventuels
- [ ] Retirer les targets non implémentés du schema (cursor, aider, windsurf, cline) — garder uniquement claude, codex, copilot, gemini, opencode
- [ ] Vérifier clippy (deux modes) + tests + fmt

### 2. CLI-specialist — logique marqueurs (link.rs / unlink.rs)
- [ ] `link.rs` : pour les `WriteMode::MarkerAppend`, détecter le fichier racine et injecter entre `<!-- armadai:start -->` / `<!-- armadai:end -->`
- [ ] `link.rs` : si marqueurs existants → remplacer (idempotent)
- [ ] `link.rs` : si pas de fichier racine → fallback `Create` (comportement actuel)
- [ ] `link.rs` : `--force` non requis pour MarkerAppend (on injecte, pas on écrase)
- [ ] `unlink.rs` : supprimer la section entre marqueurs dans le fichier racine
- [ ] `unlink.rs` : si fichier racine vide après suppression → le supprimer
- [ ] `unlink.rs` : tenter aussi la suppression du fichier provider en fallback
- [ ] Tests unitaires pour inject/replace/remove des marqueurs
- [ ] Vérifier clippy (deux modes) + tests + fmt

### 3. Mise à jour des starters
- [ ] Mettre à jour `starters/pirate-crew/pack.yaml` — ajouter section orchestration d'exemple
- [ ] Mettre à jour `starters/orchestration-demo/pack.yaml` — ajouter section orchestration
- [ ] Vérifier que `armadai init --pack` génère la bonne section orchestration dans config.yaml

### 4. Mise à jour de la config du projet armadai
- [ ] `.armadai/config.yaml` — renommer `lead` → `coordinator` dans les teams existantes
- [ ] Vérifier que `armadai link` fonctionne avec la nouvelle config

### 5. Tests end-to-end
- [ ] Tester `armadai link --target claude` sur un projet avec CLAUDE.md racine → vérifier injection entre marqueurs
- [ ] Tester `armadai link` 2 fois de suite → vérifier idempotence (pas de duplication)
- [ ] Tester `armadai unlink` → vérifier suppression propre de la section + dossier provider
- [ ] Tester `armadai link` sans CLAUDE.md racine → vérifier création dans .claude/CLAUDE.md

### 6. Release
- [ ] PR feature → develop (squash merge)
- [ ] Déléguer au release-manager pour tag + master sync

## Décisions prises

- **Marqueurs** : `<!-- armadai:start -->` / `<!-- armadai:end -->`
- **WriteMode** sur OutputFile : `Create` (défaut) ou `MarkerAppend { target }`
- **Détection fichier racine** dans `link.rs`, pas dans les linkers (linkers restent sans I/O)
- **lead → coordinator** dans TeamConfig (breaking change assumé en v0.10.x)
- **name optionnel** sur TeamConfig
- **Schemas** : JSON Schema draft 2020-12, pack.schema.json auto-contenu
- **Unlink** : double tentative (marqueurs dans racine + suppression fichier provider)
