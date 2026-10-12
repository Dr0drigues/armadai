# B2 Lot A — Registres personnalisés (agents/skills/models) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Config `registries:` (user + projet) pour ajouter des sources custom aux registres agents/skills/models, défauts conservés, + commande CLI `armadai registry`.

**Architecture:** Nouveau module `core/registries.rs` (config + résolution) ; `ProjectConfig.registries` ; les 3 modules registre consomment `resolved_sources(kind)` au lieu de leur constante ; commande `cli/registry.rs`.

**Tech Stack:** Rust edition 2024, serde/serde_yaml_ng, clap.

## Global Constraints
- Base = `origin/release/1.0.0` (@ `feaa234`). Branche `feat/b2-custom-registries`, PR vers `release/1.0.0`.
- Clippy 2 modes CI (`--no-default-features --features tui` ET `--features tui,providers-api`, `-D warnings`) + `cargo fmt -- --check` + `cargo test`. Le `model_registry` fetch réseau est gated `providers-api` — vérifier aussi ce mode.
- **Rétro-compat impérative** : sans section `registries:`, chaque registre utilise EXACTEMENT sa source par défaut actuelle (awesome-copilot / anthropics+openai skills / models.dev). Défauts conservés, custom ajoutés (union, dédup par URL).
- Suivre les patterns existants (config user dans `core/config.rs`, `ProjectConfig` dans `core/project.rs`, commandes dans `cli/mod.rs`).

---

### Task 1: Module `core/registries.rs` + config
**Files:** Create `src/core/registries.rs` ; modify `src/core/mod.rs` (`pub mod registries;`), `src/core/project.rs` (champ `registries`). Tests dans `registries.rs`.

**Interfaces produces:**
- `struct RegistrySource { pub url: String }` (Deserialize, Clone, Eq).
- `struct RegistriesConfig { pub agents: Vec<RegistrySource>, pub skills: Vec<RegistrySource>, pub models: Vec<RegistrySource>, pub starters: Vec<RegistrySource> }` (Deserialize, Default, `#[serde(default)]`).
- `pub enum RegistryKind { Agents, Skills, Models }` (starters = Lot B, non résolu ici).
- `pub fn load_user_registries() -> RegistriesConfig` (lit `~/.config/armadai/registries.yaml` via un helper `registries_config_path()` dans `core/config.rs` ; fichier absent → `Default`).
- `pub fn resolved_sources(kind: RegistryKind, defaults: &[&str], project: Option<&RegistriesConfig>) -> Vec<String>` : `defaults` + user + project, dédup en préservant l'ordre (défauts d'abord).

- [ ] Écrire tests : désérialisation d'un YAML `registries:` (agents/skills/models), `Default` = tout vide, `resolved_sources` = défauts + user + projet dédupliqués (et = défauts seuls si rien). Faire échouer.
- [ ] Implémenter `core/registries.rs` + `registries_config_path()` dans `core/config.rs` + `pub mod registries;` + `ProjectConfig.registries: Option<RegistriesConfig>` (`#[serde(default)]`).
- [ ] Tests passent ; clippy 2 modes + fmt ; commit `feat(core): custom registries config + resolution`.

---

### Task 2: Brancher les 3 registres sur `resolved_sources`
**Files:** Modify `src/registry/sync.rs`, `src/skills_registry/sync.rs`, `src/model_registry/fetch.rs`.

**Interfaces consumes:** `resolved_sources`, `RegistryKind` (Task 1).

- [ ] **agents** (`registry/sync.rs`) : au lieu de synchroniser la seule `DEFAULT_REGISTRY_URL`, itérer sur `resolved_sources(Agents, &[DEFAULT_REGISTRY_URL], project)`. Le cache (`registry_cache_dir()`) doit distinguer les sources (sous-dossier dérivé de l'URL, ex. owner/repo — s'inspirer de `skills_registry::repos_dir`). Adapter la recherche/convert pour agréger sur toutes les sources.
- [ ] **skills** (`skills_registry/sync.rs`) : remplacer `default_skill_sources()` par `resolved_sources(Skills, &DEFAULT_SKILL_SOURCES, project)` (le système gère déjà `repos_dir()` par owner/repo → intégration directe).
- [ ] **models** (`model_registry/fetch.rs`) : fetch `resolved_sources(Models, &[MODELS_DEV_URL], project)`, parser chaque catalogue (format models.dev), merger les maps provider→models — **union par provider, dernière source gagne** (défaut d'abord puis custom). Cache par source (clé dérivée de l'URL). Gardé sous `providers-api` pour le fetch réseau ; les helpers cache-only restent cohérents.
- [ ] Non-régression : sans config, chaque registre = source par défaut unique (les tests existants passent). clippy 2 modes + fmt + test ; commit `feat(registries): resolve custom sources for agents/skills/models`.

---

### Task 3: Commande CLI `armadai registry`
**Files:** Create `src/cli/registry.rs` ; modify `src/cli/mod.rs` (variant + dispatch).

**Interfaces consumes:** `RegistriesConfig`, `load_user_registries`, `registries_config_path` (Task 1).

- [ ] Sous-commande `Registry` dans l'enum `Command` (clap) avec sous-actions `list` / `add <kind> <url>` / `remove <kind> <url>` (`kind` = ValueEnum `agents|skills|models`).
- [ ] `cli/registry.rs::execute(action)` : `list` affiche par type les défauts + les sources user (+ projet si résolu), avec l'origine ; `add`/`remove` chargent `registries.yaml`, modifient la liste du type, réécrivent le fichier (créent le dossier/fichier si absent). Messages clairs, exit codes cohérents.
- [ ] Test : `add agents <url>` puis relire la config → présent ; `remove` → absent ; `add` d'un doublon = idempotent. (Utiliser un `registries.yaml` en tmp via l'override de config dir des tests existants.)
- [ ] clippy 2 modes + fmt + test ; commit `feat(cli): armadai registry list/add/remove`.

---

## Notes
- `registries.yaml` : format identique à la section `registries:` d'`armadai.yaml` (réutiliser `RegistriesConfig` pour les deux). Réutiliser le mécanisme de dossier de config des tests (`ENV_MUTEX` + override, cf tests `skill.rs`/`starter.rs`).
- Ne PAS implémenter les starters distants (Lot B) ni `disable_defaults`. Le champ `starters` de `RegistriesConfig` existe (schéma) mais n'est consommé nulle part en Lot A.
- Cache multi-sources : si distinguer les sources dans le cache est trop invasif pour agents en Task 2, il est acceptable de traiter les sources séquentiellement dans un même index de recherche tant que la provenance reste correcte — documenter le choix.
