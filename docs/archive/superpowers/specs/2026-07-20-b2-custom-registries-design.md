# B2 — Registres personnalisés (Lot A : agents / skills / models)

> **Statut** : design validé (brainstorm 2026-07-20)
> **Cible** : axe 2, feature B2, **Lot A**. Le Lot B (starters distants — nouveau système) fera l'objet d'une spec séparée.

## 1. Objectif

Permettre à l'utilisateur d'ajouter des **registres custom** (sources configurables) pour les **agents** (registry/awesome-copilot), **skills** (skills_registry) et **models** (model_registry), au lieu des seules sources câblées en dur. Rétro-compatible : sans config, comportement inchangé.

## 2. Config unifiée `registries:`

Section déclarée dans la config **user** (`~/.config/armadai/`, globale) **et/ou** `armadai.yaml` **projet**, mergées (user = base, projet = ajouts) :

```yaml
registries:
  agents:
    - url: "https://github.com/moi/agents.git"
  skills:
    - url: "https://github.com/moi/skills.git"
  models:
    - url: "https://catalogue.interne/models.json"
```

- Emplacement user : un fichier dédié `~/.config/armadai/registries.yaml` (nouveau), lu par le système de config. Emplacement projet : section `registries:` de `armadai.yaml` (`ProjectConfig`).
- Chaque entrée = `{ url: String }` (extensible plus tard : `name`, `auth`…). `agents`/`skills` = URLs git ; `models` = URL(s) JSON (format models.dev).
- `starters:` est réservé au Lot B (peut apparaître dans le schéma mais non traité en Lot A — documenté).

## 3. Résolution

- **Défauts embarqués CONSERVÉS** : `registry/sync.rs` `DEFAULT_REGISTRY_URL` (awesome-copilot), `skills_registry/sync.rs` `default_skill_sources()` (anthropics/openai), `model_registry/fetch.rs` `MODELS_DEV_URL` (models.dev). Sans config `registries:`, comportement **identique** à aujourd'hui.
- Les registres custom **s'ajoutent** aux défauts (union, dédup par URL). Pas de désactivation des défauts en Lot A (`disable_defaults` = hors scope / YAGNI).
- Résolution : `RegistriesConfig` = merge de la config user (`registries.yaml`) et de `ProjectConfig.registries`. Une fonction `resolved_sources(kind) -> Vec<String>` renvoie défauts + custom pour un type donné.

## 4. Intégration par registre

- **agents** (`src/registry/sync.rs`) : synchroniser depuis `[DEFAULT_REGISTRY_URL] + registries.agents` au lieu de la seule URL par défaut. Cache par source (le cache dir existant `registry_cache_dir()` doit distinguer les sources — sous-dossier par owner/repo, comme skills_registry le fait déjà).
- **skills** (`src/skills_registry/sync.rs`) : `default_skill_sources()` + `registries.skills` (le système gère déjà une liste de sources et un `repos_dir()` par owner/repo — intégration naturelle).
- **models** (`src/model_registry/fetch.rs`) : fetch de `[MODELS_DEV_URL] + registries.models`, parser chaque catalogue (même format models.dev) et **merger** les maps provider→models. **Stratégie fixée** : union par provider ; l'ordre = défaut (models.dev) puis sources custom dans l'ordre déclaré ; si un provider apparaît dans plusieurs sources, la **dernière** source gagne pour ce provider (⇒ un catalogue interne peut surcharger models.dev pour un provider donné). Cache par source (clé dérivée de l'URL).

## 5. Commandes CLI

Nouvelle commande `armadai registry` (cohérente avec `armadai config starters-dir`) :
- `armadai registry list` — affiche, par type, défauts + custom (avec l'origine user/projet).
- `armadai registry add <agents|skills|models> <url>` — ajoute à la config **user** (`registries.yaml`).
- `armadai registry remove <agents|skills|models> <url>` — retire de la config user.

## 6. Architecture

- Nouveau module `src/core/registries.rs` : `struct RegistriesConfig { agents: Vec<RegistrySource>, skills: Vec<RegistrySource>, models: Vec<RegistrySource>, starters: Vec<RegistrySource> }` (Deserialize, Default) ; `RegistrySource { url: String }` ; `load_user_registries()` (lit `registries.yaml`) ; `resolved_sources(kind, project_cfg) -> Vec<String>` (défauts + user + projet, dédup).
- `ProjectConfig` gagne `pub registries: Option<RegistriesConfig>` (`#[serde(default)]`).
- `cli/registry.rs` : commande + sous-commandes list/add/remove (édite `registries.yaml`).
- Les 3 modules registre appellent `resolved_sources(...)` au lieu de leur constante.

## 7. Tests

- `RegistriesConfig` : désérialisation YAML (user + projet), merge, dédup, `resolved_sources` renvoie défauts+custom.
- CLI : `add` puis `list` reflète l'ajout ; `remove` retire ; idempotence.
- Intégration (par registre) : `resolved_sources` est bien consommé (sans réseau : vérifier que la liste des sources inclut défaut + custom ; le fetch réseau reste testé/moqué comme l'existant).
- Non-régression : sans `registries:`, chaque registre utilise exactement sa source par défaut actuelle.

## 8. Hors scope (Lot A)

- **Starters distants** (Lot B, spec séparée) — nouveau système de fetch/install de starter packs.
- `disable_defaults`, authentification des registres privés (token), `name`/priorité par source — évolutions futures.
