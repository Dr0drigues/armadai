# OH7 — Reusable orchestration core + Cargo workspace — Design (#252)

## Contexte

ArmadAI est aujourd'hui un **crate binaire unique** (`armadai`, edition 2024,
pas de `lib.rs`, 2 bins : `armadai` + `fake-claude`, features
`tui`/`web`/`storage`/`providers-api`). L'objectif d'OH7 (#252) : extraire le
**cœur d'orchestration** en crate lib réutilisable, orthogonal aux interfaces
(TUI/Web/CLI), livré via un **Cargo workspace multi-crates dans le même repo** —
prérequis naturel d'OH2 (portabilité local/remote) et du plugin Claude Code.

Direction déjà pré-validée (mémoire `project_modularity_workspace`) : **PAS** de
git submodules, **PAS** de crates.io / repos séparés (prématuré) ; le levier =
Cargo workspace multi-crates (frontières imposées par le compilateur, builds
incrémentaux, isolation renforcée des feature flags).

### Le vrai enjeu : casser les cycles

La cartographie des `use crate::<module>` révèle que le module `core` actuel
**n'est pas** un cœur proprement stratifié — il a des cycles bidirectionnels,
tolérés dans un crate unique mais **interdits entre crates** :

| Couplage | Nature | Résolution |
|---|---|---|
| `core ↔ parser` | parsing d'agent (domaine) | parser **rejoint** `armadai-core` (cycle intra-crate OK) |
| `core → linker` | `linker::model_resolution` + `model_aliases` = routing/modèle (mal placé) | **SPLIT** (découvert à l'implémentation 1a) : `model_resolution` mélange deux concerns — les **primitives de tier** (`ModelTier`, `resolve_model_for_tier`, `fallback_model_for_tier`, `classify_model_tier`) **remontent dans core** ; les fonctions opérant sur `LinkAgent` (`remap_*`, `resolve_latest_placeholders`, `TargetKind`, previews) **restent dans linker** (elles importent les primitives depuis core). `model_aliases` (pur) remonte entièrement dans core. Résultat : plus aucune arête core→linker, y compris via `LinkAgent`. |
| `core → providers` | `core` utilise le trait `Provider` ; `providers` utilise les types de `core` | **inversion de dépendance** : trait `Provider` remonte dans core ; providers l'implémente |
| `providers → shell` | `providers` utilise `shell::json_runner` (parsing stream-json) | `json_runner` **descend** dans `armadai-providers` ; shell en dépendra |
| `core → storage` (feature) | `SqliteLog` couple `core` à rusqlite | `SqliteLog` **sort** de core vers le bin, impl de `core::EventLog` (trait qui reste dans core) |

`secrets` est une **feuille** pure (aucun `use crate::`). `storage`, en
revanche, a **deux arêtes descendantes résiduelles vers `core`** (découvert au
cadrage du Lot 2, 2026-07-28) — ce ne sont pas des cycles (core ne pointe pas
vers storage, l'invariant du Lot 1 tient), mais elles empêchent storage d'être
une **feuille** :

1. `storage::es_log` (`SqliteLog`, posé là en 1e) importe
   `core::orchestration::es::{event, log}` ;
2. `storage::mod` (`init_db`/`resolve_storage_path`) importe
   `core::config::{data_dir, load_user_config}` pour résoudre le chemin depuis
   la config utilisateur.

Ces deux couplages sont cassés en **prélude du Lot 2** (`SqliteLog` et la
résolution de chemin config remontent côté bin), pour que `armadai-storage` soit
un pur wrapper rusqlite (schema/queries/open) conforme au design « feuille ».

## Décisions validées (Dimitri, 2026-07-28)

1. **Granularité** : **découpage en couches** (pas split complet). Crates :
   `armadai-core`, `armadai-providers`, `armadai-secrets`, `armadai-storage` +
   le bin `armadai`. Les interfaces/adaptateurs (cli, tui, web, shell, linker,
   registres, audit, theme) **restent dans le bin** pour l'instant.
2. **Frontière du cœur** : **inversion de dépendance**. Le trait `Provider` +
   les types domaine + le trait `EventLog` vivent dans `armadai-core` ;
   `armadai-providers` implémente `Provider` et dépend de core ; core ne dépend
   **jamais** de providers.
3. **`EventLog`/`InMemoryLog`** restent dans core (storage-agnostiques) ;
   **`SqliteLog`** sort vers le bin (dépendance inversée sur `armadai-storage`).
4. **Migration** : casser les cycles d'abord (refactors intra-crate, CI verte),
   extraire ensuite. 1 PR par sous-lot + revue indé + validation Dimitri.

## Architecture cible (graphe de crates)

```
[bin] armadai  (main + fake-claude ; cli, tui, web, shell, linker,
 │              registry, skills_registry, starters_registry, audit,
 │              theme, logging ; câblage SqliteLog)
 │  dépend de ↓
 ├── armadai-providers   (api/{anthropic,google,openai}, cli, factory,
 │    │                   rate_limiter, model_registry, json_runner)
 │    │  dépend de ↓
 │    └── armadai-core    FEUILLE : types domaine (Agent, AgentMetadata,
 │                        OrchestrationConfig, PipelineConfig, events/RunEvent,
 │                        routing, model_resolution, model_aliases) + trait
 │                        Provider + trait EventLog + InMemoryLog + parser +
 │                        moteur ES (orchestration/es + 4 patterns)
 ├── armadai-storage      FEUILLE (rusqlite ; dépendance OPTIONNELLE du bin)
 └── armadai-secrets      FEUILLE (SOPS + age)
```

Règle d'or : chaque crate ne dépend **que vers le bas**. `armadai-core` est une
feuille sans dépendance interne ni dépendance lourde optionnelle (ni reqwest, ni
rusqlite, ni ratatui) → toujours compilée, légère, portable.

## Lots de migration

**Principe** : les gros risques (cassage de cycles) sont des **refactors purs
intra-crate AVANT tout éclatement** ; l'extraction devient mécanique ensuite.

### Lot 1 — Casser les cycles (refactors intra-crate, aucun workspace encore)

Chaque sous-lot = 1 PR, CI verte, aucun changement de structure de crates.

- **1a** — Déplacer `linker::model_resolution` + `linker::model_aliases` vers
  `core::` (routing/modèle). Mettre à jour tous les `use`. *(casse core→linker)*
- **1b** — Déplacer `shell::json_runner` vers `providers::` (parsing
  stream-json : `supports_json`, `json_mode_args`, `StreamEvent`,
  `parse_stream_event`). Shell importera depuis `providers`. *(casse
  providers→shell)*
- **1c** — Déplacer `parser::*` vers `core::parser` (`parse_agent_file`,
  `frontmatter`). *(fusionne le cycle core↔parser en intra-`core`)*
- **1d** — Déplacer le trait `Provider` (+ `CompletionRequest`,
  `CompletionResponse`, `ProviderMetadata`, `TokenStream`) de
  `providers::traits` vers `core::` ; `providers` l'implémente. *(casse
  core→providers)*
- **1e** — Sortir `SqliteLog` de `core::orchestration::es::log` vers un module
  côté bin (adjacent à `storage`), en gardant `EventLog` + `InMemoryLog` dans
  core. Le bin construit `SqliteLog` (impl de `core::EventLog`) quand la
  persistance est active. *(casse core→storage)*

**Invariant de fin de Lot 1** : `core` est cycle-free (feuille au sens des
`use crate::`), `providers` ne dépend que de `core`, `shell` dépend de
`providers` pour le json_runner. Vérifiable : la matrice de dépendances
inter-modules n'a plus de cycle impliquant core/providers.

### Lot 2 — Workspace + feuilles

Casser d'abord les deux arêtes `storage → core` (prélude), puis introduire
`[workspace]` à la racine (racine = `[workspace]` **et** `[package]`, voir
« Disposition ») et extraire les deux feuilles. Sous-lots (1 PR chacun) :

- **2a** — `storage::es_log` (`SqliteLog`) remonte dans un module **bin**
  (`src/es_log.rs`, gated `storage`). Il dépend de `core::EventLog` +
  `storage::Database`. Mise à jour des imports `crate::storage::es_log::SqliteLog`
  → `crate::es_log::SqliteLog` (cli). *(casse storage→core edge #1)*
- **2b** — `init_db`/`resolve_storage_path` + garde-fou test + résolution via
  `core::config` remontent dans un module **bin** `src/db.rs` (gated `storage`).
  `storage::mod` ne garde que `Database`, `open(path)` (Connection+schema) et
  `init_embedded` (inchangé, `#[cfg(test)]` pour l'instant). Callers
  `crate::storage::init_db()` → `crate::db::init_db()`. Après 2b :
  `grep 'use crate::' src/storage/` = vide → **storage est core-free**.
  *(casse storage→core edge #2)*
- **2c** — Introduire `[workspace]` racine (`members = ["crates/*"]`,
  `resolver = "3"`) et extraire **`armadai-secrets`** (feuille pure) sous
  `crates/armadai-secrets/`. Le bin en dépend (`armadai-secrets = { path = … }`),
  `crate::secrets::` → `armadai_secrets::`. Premier crate = validation du
  workspace.
- **2d** — Extraire **`armadai-storage`** sous `crates/armadai-storage/`.
  Convertir `#[cfg(test)] pub fn init_embedded` → `pub fn open_in_memory()`
  (non-gated : valide en crate lib, invisible sinon aux tests du bin), convertir
  les 2 intra-doc-links `[crate::core::…]` (queries.rs, schema.rs) en texte
  brut, mettre à jour `crate::storage::` → `armadai_storage::` (dont `es_log.rs`
  et `db.rs` côté bin).

**`[workspace.dependencies]`** (centralisation des versions) : **YAGNI au Lot
2** — les 2 feuilles épinglent leurs versions directement (alignées sur la
racine). La centralisation arrive quand plusieurs crates partagent des deps
lourdes (Lots 3/4).

### Lot 3 — Extraire `armadai-core`

Déplacer `core/*` (+ `parser` fusionné + `model_resolution`/`model_aliases` +
trait `Provider` + `EventLog`/`InMemoryLog` + `orchestration/es` + 4 patterns)
vers `crates/armadai-core/`. Ajouter un `lib.rs` avec la façade `pub use`
(Section « API publique »). Le bin et `armadai-providers` dépendent de core.
Gros déplacement mais **mécanique** (core est déjà cycle-free).

### Lot 4 — Extraire `armadai-providers`

Déplacer `providers/*` (+ `json_runner` relocalisé + `factory` + `rate_limiter`
+ `model_registry`) vers `crates/armadai-providers/`, dépend de `armadai-core`.
Feature `api` = `dep:reqwest`.

### Lot 5 — Bin final + features re-plombées

Le bin `armadai` = interfaces + adaptateurs (cli, tui, web, shell, linker,
registry, skills_registry, starters_registry, audit, theme, logging) + câblage
`SqliteLog` + `main` + `fake-claude`. Re-plomber les features (Section
suivante). Ajouter le check `cargo build -p armadai-core` (cœur nu) à la CI.

### Disposition des fichiers

**Le bin reste à la racine** (moindre churn, aucun chemin `include_dir!` /
templates cassé) : le `Cargo.toml` racine est **à la fois** `[workspace]` **et**
`[package]` (le bin `armadai`, `src/` inchangé). Seuls les **nouveaux crates**
sont extraits sous `crates/` : `crates/armadai-core/`,
`crates/armadai-providers/`, `crates/armadai-storage/`,
`crates/armadai-secrets/`. Racine : `[workspace] members = ["crates/*"]` +
`[workspace.dependencies]` pour centraliser les versions partagées (tokio,
serde, anyhow, tracing, async-trait…). *(C'est un layout Cargo valide : un
package racine qui est aussi la racine du workspace.)*

## Feature flags à travers le workspace

- **`armadai-core`** : aucune feature optionnelle. Feuille pure, toujours
  compilée (pas de reqwest/rusqlite/ratatui).
- **`armadai-providers`** : feature `api` → `dep:reqwest` (providers HTTP
  anthropic/google/openai + fetch model_registry). Provider `cli`
  (tokio::process) et rate-limiter toujours-actifs.
- **`armadai-storage`** : crate wrapper rusqlite ; **dépendance optionnelle** du
  bin (pas une feature interne au crate).
- **`armadai` (bin)** : `tui` (ratatui/crossterm/pty, bin-local), `web` (axum,
  bin-local), `storage` (→ `dep:armadai-storage` optionnel + câblage
  `SqliteLog`), `providers-api` (→ `armadai-providers/api`).
  `default = ["tui", "web", "storage", "providers-api"]`. Le bin **forwarde**
  ses features aux crates membres.

Les combinaisons CI existantes tournent sur le **bin** (les features se
propagent) → **inchangé**. Ajout : `cargo build -p armadai-core` sans feature
(garde-fou de portabilité du cœur).

## API publique de `armadai-core`

Exposée via `lib.rs` avec des `pub use` curatés (façade stable, pas des chemins
profonds). **YAGNI** : l'extraction préserve les `pub` actuels + une façade
racine ; le durcissement fin de l'API viendra avec OH2/plugin.

- **Types domaine** : `Agent`, `AgentMetadata`, `OrchestrationConfig`,
  `OrchestrationPattern`, `PipelineConfig`.
- **Contrats (traits)** : `Provider` (+ `CompletionRequest`,
  `CompletionResponse`, `ProviderMetadata`, `TokenStream`), `EventLog`
  (+ `InMemoryLog`).
- **Événements** : `RunEvent`, `EventSink`.
- **Moteur ES** : `run_event_sourced`, `resume_event_sourced`, `replay`,
  `Action`, `ExecutionEvent`, `ExecutionState`, les 4 deciders + les entrées
  `run_direct_es` / `run_blackboard_es` / `run_ring_es` / `run_hierarchical_es`.
- **Domaine** : `parse_agent_file`, routing / `model_resolution`.

OH2 implémentera son propre `Provider` (remote) et `EventLog` (persistance
remote) **contre les traits de core** ; le plugin Claude Code pilotera
`run_*_es` et mappera les `RunEvent` vers ses hooks.

## Tests & CI

- Chaque sous-lot du **Lot 1** : refactor pur → la suite existante doit passer à
  l'identique (aucun test réécrit sauf déplacement d'`use`). Gate habituel :
  `cargo fmt --all` + clippy 3 modes (`tui` / `tui,providers-api` /
  `tui,web,storage`) `-D warnings` + `cargo test` 2 modes (`tui`,
  `tui,storage`).
- **Lots 2–5** : après chaque extraction, la même gate sur le **bin** (features
  propagées) + `cargo build -p armadai-core` (cœur nu). Les tests unitaires
  migrent avec leur code dans le crate cible.
- `rust-analyzer` non fiable ici (ABI/stale) → **vérifier au compilateur**.
- Le workflow CI (`.github/workflows/ci.yml`) : adapter les commandes pour le
  workspace (les `--features` du bin restent la référence ; ajouter le build du
  cœur nu). À traiter dans le Lot 5.

## Hors périmètre

- Split complet des interfaces (tui/web/cli/shell en crates séparés) — non
  retenu (choix « couches ») ; possible plus tard, même mécanique.
- Publication sur crates.io / repos séparés — prématuré (mémoire
  `project_modularity_workspace`).
- Durcissement/stabilisation de l'API publique de core au-delà de la façade
  `pub use` — viendra avec OH2/plugin (qui définiront les vrais besoins).
- `linker`/`shell` en crates — restent dans le bin.

## Risques

- **Ampleur du Lot 3** (extraction de core) : gros déplacement de fichiers.
  Atténué en le faisant **après** que les cycles sont cassés (Lot 1) → purement
  mécanique, pas de refactor logique concurrent.
- **`SqliteLog` (1e)** : l'inversion `EventLog` (core) ↔ `SqliteLog` (bin) doit
  préserver le câblage `run --resume/--replay` et la projection `RunEvent`
  (OH1 Lot 6). Couvert par les tests e2e existants ; vérifier qu'ils passent
  après relocalisation.
- **Unification des features Cargo** : le workspace unifie les features entre
  membres — vérifier qu'un mode CI (p.ex. `tui` seul) ne tire pas reqwest via
  une unification involontaire. Le `cargo build -p armadai-core` nu est le
  garde-fou.
- **`fake-claude`** (bin de test e2e) : reste dans le bin (racine) ; inchangé
  puisque `src/` ne bouge pas.
- **Chemins de ressources embarquées** (`include_dir!` du dist Svelte,
  templates, embedded) : le bin restant à la racine (`src/` inchangé), ces
  chemins **ne changent pas** — c'est précisément la raison de garder le bin à
  la racine. Vérifier néanmoins qu'aucune ressource embarquée n'est référencée
  depuis un crate extrait (elles vivent côté bin : web/tui).
