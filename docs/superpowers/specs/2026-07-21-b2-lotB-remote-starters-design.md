# B2 Lot B — Starters distants (remote starter registries)

> **Statut** : design validé (brainstorm 2026-07-21)
> **Cible** : rc.4. Suite de B2 Lot A (#182, registres agents/skills/models). Le champ `RegistriesConfig.starters` a été réservé en Lot A mais `RegistryKind` n'a pas encore de variante `Starters`.
> **Base** : `release/1.0.0`.

## 1. Objectif

Permettre de **fetcher et installer des starter packs depuis des registres distants** (aujourd'hui les starters sont purement locaux : `StarterPack::load(dir)` + dossiers `builtin_starters_dir`/config). Le système est **agnostique du livrable** (git ou archive) derrière une abstraction commune, avec une **norme formalisée** pour ce qu'est un registre de starters.

## 2. Source agnostique du livrable

`RegistrySource` (dans `core/registries.rs`) gagne un **`kind`** :
```yaml
registries:
  starters:
    - url: "https://github.com/moi/armadai-starters.git"      # kind inféré: git
    - url: "https://example.com/packs/rust-qa.tar.gz"          # kind inféré: archive
    - url: "https://interne/packs"
      kind: archive                                            # override explicite
```
- `pub enum SourceKind { Git, Archive }` ; champ `kind: Option<SourceKind>` sur `RegistrySource` (`#[serde(default)]`).
- **Inférence** si `kind` absent : URL finissant par `.git` (ou host git connu) → `Git` ; `.tar.gz`/`.tgz`/`.zip` → `Archive` ; défaut → `Git`.
- Extensible (futurs kinds sans casser le schéma).

## 3. Fetcher pluggable

Trait `StarterFetcher` : `fn fetch(&self, source: &RegistrySource, dest: &Path) -> Result<()>` (dépose le contenu du registre dans `dest`).
- **`GitFetcher`** — clone/pull (shell-out `git`, comme `skills_registry::sync`). **Toujours disponible** (pas de dépendance HTTP).
- **`ArchiveFetcher`** — download (reqwest) + extract (tar.gz/zip). **Gated `providers-api`** (comme `model_registry` fetch réseau) ; sans `providers-api`, une source `archive` est ignorée avec un warning clair.
- Cache : `starters_cache_dir()/<slug(url)>/` (slug dérivé de l'URL, cf. `skills_registry` owner/repo). Réutiliser le helper de cache existant si possible.

## 3.1 Découpage fetch par kind
`fetch_starter_source(source)` choisit le fetcher selon `kind` (résolu). Git = Lot 1 ; Archive = Lot 2.

## 4. Norme d'un registre (hybride)

Quel que soit le transport, une fois fetché dans le cache :
- **Convention (toujours)** : tout dossier contenant un `pack.yaml` (norme `StarterPack` existante : `pack.yaml` + `agents/`/`prompts/`/`skills/`) est un pack.
- **Manifest optionnel** : un `armadai-starters.yaml` à la racine du registre **enrichit** — métadonnées (description, tags), ordre d'affichage, et restriction explicite des packs exposés (si présent, seuls les packs listés sont exposés ; sinon tous les `pack.yaml` trouvés). Schéma :
  ```yaml
  # armadai-starters.yaml (optionnel)
  packs:
    - path: rust-qa        # dossier relatif contenant pack.yaml
      description: "..."    # override/enrichit
      tags: [rust, qa]
  ```
- Absence de manifest = pur scan (rétro-compatible avec n'importe quel dépôt de packs).

## 5. Intégration au système starters existant

- `RegistryKind::Starters` ajouté ; `RegistryKind::sources()` mappe vers `config.starters` ; `resolved_sources(Starters, defaults=&[], user, project)` (pas de défaut embarqué pour les starters distants — union user+projet).
- `all_starters_dirs()` / `load_all_packs()` / `find_pack_dir()` (dans `core/starter.rs`) incluent le **cache distant** (`starters_cache_dir()/*/`) en plus des dossiers locaux existants. Priorité : local > cache distant (un pack local du même nom gagne), documenté.
- `list_available_packs()` inclut les packs distants synchronisés.

## 6. Sync (explicite + auto-on-miss)

- **Explicite** : `armadai registry sync [kind]` — synchronise les sources (tous kinds, ou `starters`/`agents`/`skills`/`models` si précisé) : pour chaque source starters, `fetch_starter_source` dans le cache.
- **Auto-on-miss** : si `armadai init --pack <nom>` ne trouve pas le pack en local NI dans le cache distant, tenter **un sync** des sources starters puis re-chercher, avant d'échouer. Évite un `sync` manuel préalable tout en ne re-fetchant pas à chaque `init`.

## 7. CLI

- Étendre `armadai registry sources add|remove|list` (B2 Lot A) au kind **`starters`** (le `RegistryKind`/ValueEnum CLI gagne `starters`).
- Nouvelle sous-commande **`armadai registry sync [agents|skills|models|starters]`** (sans argument = tous).
- `armadai init --pack <nom>` : inchangé côté surface, mais résout aussi le cache distant (+ auto-on-miss).

## 8. Feature flags / CI

- `GitFetcher` + toute la logique cœur : **toujours compilée** (git shell-out, pas de dép). Clippy CI 2 modes (`tui`, `tui,providers-api`).
- `ArchiveFetcher` : **`#[cfg(feature = "providers-api")]`** (reqwest). Sans `providers-api`, `fetch_starter_source` sur une source `archive` → warning « archive fetch requires providers-api » et skip. Vérifier clippy dans les deux modes.
- Réutiliser une lib d'extraction déjà présente si dispo (tar/flate2/zip) ; sinon ajouter derrière `providers-api` uniquement.

## 9. Tests

- **Lot 1** : `SourceKind` désérialisation + inférence depuis l'URL (git/archive/override) ; `resolved_sources(Starters)` (union user+projet) ; découverte hybride (scan pack.yaml ; manifest restreint/enrichit) sur un dossier fixture ; `find_pack_dir`/`load_all_packs` incluent le cache (fixture) ; priorité local>distant. `GitFetcher` testé via un dépôt git local temporaire (init + commit un pack, fetch, vérifier le pack découvert).
- **Lot 2** : `ArchiveFetcher` (providers-api) sur une archive fixture (tar.gz local → extract → pack découvert) ; CLI `sources add/remove/list starters` (round-trip config) ; `registry sync` (dry/mock) ; auto-on-miss d'`init --pack` (fixture).

## 10. Découpage

- **Lot 1 — cœur + git** : `SourceKind`, `RegistryKind::Starters`, `resolved_sources`, trait `StarterFetcher` + `GitFetcher`, cache, découverte hybride (scan + manifest optionnel), intégration `starter.rs`. Une PR.
- **Lot 2 — archive + CLI + sync** : `ArchiveFetcher` (providers-api), CLI (`sources … starters`, `registry sync`), auto-on-miss `init --pack`. Une PR.

## 11. Hors périmètre

- Kind « index » (manifest distant listant des packs à fetcher chacun via une URL) — le manifest ici est *intra-registre* (enrichissement), pas un index de fetch ; l'index cross-registre est une évolution future.
- Authentification des registres privés (token) — évolution future (comme B2 Lot A).
- Certification/signature des packs — hors v1 (cf. « Registres personnalisés avec certification », projet séparé).
