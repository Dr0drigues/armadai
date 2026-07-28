# OH7 Lot 2 — Workspace + feuilles (secrets, storage) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduire le Cargo `[workspace]` à la racine et extraire les deux crates feuilles `armadai-secrets` et `armadai-storage`, après avoir cassé les deux arêtes résiduelles `storage → core`.

**Architecture:** Le bin `armadai` reste à la racine (`Cargo.toml` = `[package]` **et** `[workspace]`, `src/` inchangé → aucun chemin `include_dir!`/templates cassé). Les nouveaux crates sont extraits sous `crates/`. Prélude obligatoire : `SqliteLog` (edge `storage→core::es`) et `init_db`/résolution-de-chemin-config (edge `storage→core::config`) remontent côté bin, pour que `armadai-storage` soit un pur wrapper rusqlite (feuille, zéro `use crate::`).

**Tech Stack:** Rust edition 2024, Cargo workspaces (resolver 3), rusqlite (bundled), serde/serde_yaml_ng, SOPS/age (secrets).

## Global Constraints

- **Branche** : master-only. Une PR par sous-lot, squash-merge, revue indépendante + CI verte (6 checks) avant merge. CI verte ≠ suffisante ; revue indé obligatoire.
- **Gate CI** à chaque sous-lot (le bin reste la référence — les features se propagent) :
  - `cargo fmt --all -- --check`
  - clippy 3 modes, `-D warnings` : `--no-default-features --features tui` ; `--no-default-features --features tui,providers-api` ; `--no-default-features --features tui,web,storage`
  - `cargo test` 2 modes : `--no-default-features --features tui` ; `--no-default-features --features tui,storage`
- **Refactors purs** : aucun test réécrit sauf déplacement d'`use`/de fichier. La suite existante doit passer à l'identique. Le nombre de tests total ne doit pas baisser.
- **`rust-analyzer` non fiable ici** (ABI/stale, faux E0432/E0433 en cours d'édition) → **toujours vérifier au compilateur** (`cargo`), jamais aux diagnostics RA.
- **Code/commentaires/commits en anglais.** Commits Conventional Commits ; scope `oh7`. Terminer chaque message de commit par la ligne `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **`fake-claude`** (2ᵉ bin, `src/bin/fake-claude.rs`) reste à la racine, inchangé.
- **`armadai-core` featureless** : ne jamais faire dépendre un crate extrait d'une ressource embarquée (`include_dir!`) — elles restent côté bin.

---

## File Structure

Après le Lot 2 :

```
Cargo.toml                       # racine : [workspace] + [package] armadai (bin), + [[bin]] fake-claude
src/
  main.rs                        # `mod es_log;` (gated storage) + `mod db;` (gated storage) ajoutés ; `mod secrets;` retiré ; `mod storage;` retiré
  es_log.rs                      # NOUVEAU (bin) : SqliteLog (impl armadai_core? non — core reste module au Lot 2 → crate::core) — voir note ci-dessous
  db.rs                          # NOUVEAU (bin) : init_db / resolve_storage_path / garde-fou test (via crate::core::config + armadai_storage::open)
  ...                            # cli, core, linker, providers, tui, web, … inchangés
crates/
  armadai-secrets/
    Cargo.toml
    src/lib.rs                   # ex-src/secrets/mod.rs (structs + load_secrets)
    src/sops.rs                  # ex-src/secrets/sops.rs
  armadai-storage/
    Cargo.toml
    src/lib.rs                   # ex-src/storage/mod.rs (Database, open, open_in_memory) — core-free
    src/schema.rs                # ex-src/storage/schema.rs (doc-links core → texte)
    src/queries.rs               # ex-src/storage/queries.rs (doc-links core → texte)
```

> **Note sur `core` au Lot 2** : `core` n'est PAS encore un crate (c'est le Lot 3). Il reste `mod core;` dans le bin. Donc `src/es_log.rs` et `src/db.rs` continuent d'importer `crate::core::…`. Seuls `secrets` et `storage` deviennent des crates ce Lot-ci. `es_log.rs`/`db.rs` référencent `armadai_storage::{Database, open}` après 2d.

---

## Task 2a: SqliteLog → module bin `src/es_log.rs`

Casse `storage → core` edge #1 en déplaçant `SqliteLog` (qui implémente `core::EventLog`) hors du module `storage` vers un module bin. Purement une relocalisation ; aucune logique ne change.

**Files:**
- Create: `src/es_log.rs` (contenu = ancien `src/storage/es_log.rs` verbatim)
- Delete: `src/storage/es_log.rs`
- Modify: `src/storage/mod.rs:1-2` (retirer `#[cfg(feature = "storage")] pub mod es_log;`)
- Modify: `src/main.rs` (ajouter `#[cfg(feature = "storage")] mod es_log;`)
- Modify (imports): `src/cli/projections.rs`, `src/cli/run.rs`, `src/cli/run_es_record.rs`, `src/cli/run_replay.rs` — `crate::storage::es_log::SqliteLog` → `crate::es_log::SqliteLog`

**Interfaces:**
- Consumes: `crate::core::orchestration::es::{event::ExecutionEvent, log::EventLog}`, `crate::storage::{Database, init_embedded}` (inchangés — `storage` est encore un module au Lot 2)
- Produces: `crate::es_log::SqliteLog` (`pub struct`, `pub fn new(db: crate::storage::Database) -> Self`, `impl EventLog`)

- [ ] **Step 1: Déplacer le fichier**

```bash
git mv src/storage/es_log.rs src/es_log.rs
```

- [ ] **Step 2: Retirer la déclaration du module dans `storage/mod.rs`**

Dans `src/storage/mod.rs`, supprimer les lignes :

```rust
#[cfg(feature = "storage")]
pub mod es_log;
```

- [ ] **Step 3: Déclarer le module bin dans `main.rs`**

Dans `src/main.rs`, ajouter (à côté de `#[cfg(feature = "storage")] mod storage;`) :

```rust
#[cfg(feature = "storage")]
mod es_log;
```

- [ ] **Step 4: Réécrire les imports des consommateurs**

```bash
grep -rl "crate::storage::es_log::SqliteLog" src/ \
  | xargs sed -i '' 's/crate::storage::es_log::SqliteLog/crate::es_log::SqliteLog/g'
```

Vérifier qu'il ne reste aucune référence à `storage::es_log` :

```bash
grep -rn "storage::es_log" src/ && echo "RESTE DES REFS — corriger" || echo "OK: aucune ref"
```

Attendu : `OK: aucune ref`.

- [ ] **Step 5: Invariant — `es_log` n'est plus sous storage**

Le fichier `src/es_log.rs` contient toujours `use crate::core::…` et `crate::storage::Database` (normal : c'est du code bin qui pontera core↔storage). L'edge cassé est `storage → core::es` : `src/storage/` ne contient plus es_log.

```bash
ls src/storage/   # doit montrer : mod.rs queries.rs schema.rs (plus de es_log.rs)
```

- [ ] **Step 6: Gate CI complète**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui
cargo test --no-default-features --features tui,storage
```
Attendu : tout vert, nombre de tests inchangé vs master.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(oh7): Lot 2a — move SqliteLog out of storage into bin es_log module (#252)"
```

---

## Task 2b: `init_db`/résolution-de-chemin → module bin `src/db.rs` ; storage core-free

Casse `storage → core` edge #2. `init_db`, `resolve_storage_path` et le garde-fou de test (tous dépendants de `core::config`) remontent dans un module bin `db`. `storage::mod` n'expose plus qu'un wrapper rusqlite pur : `Database`, `open(path)`, `init_embedded` (test-only, inchangé ce Lot-ci).

**Files:**
- Create: `src/db.rs` (bin) — `init_db`, `resolve_storage_path`, garde-fou `#[cfg(test)]`, tests de résolution
- Modify: `src/storage/mod.rs` — retirer `resolve_storage_path`/`init_db`/le garde-fou + les 2 tests de résolution ; ajouter `pub fn open(path: &Path)` ; garder `Database`, `init_embedded`
- Modify: `src/main.rs` — ajouter `#[cfg(feature = "storage")] mod db;`
- Modify (imports): tous les `crate::storage::init_db` → `crate::db::init_db` (cli/costs.rs, cli/history.rs, cli/projections.rs, cli/run.rs, cli/run_es_record.rs, cli/run_replay.rs, + tui/web éventuels)

**Interfaces:**
- `storage::open` — Produces: `pub fn open(path: &std::path::Path) -> anyhow::Result<Database>` (ouvre la connexion + applique le schéma)
- `db::init_db` — Produces: `pub fn init_db() -> anyhow::Result<crate::storage::Database>` (résout le chemin via `core::config`, applique le garde-fou test, délègue à `crate::storage::open`)
- `storage::init_embedded` — inchangé : `#[cfg(test)] pub fn init_embedded() -> anyhow::Result<Database>`

- [ ] **Step 1: Ajouter `open(path)` dans `storage/mod.rs`**

Dans `src/storage/mod.rs`, ajouter (garder `Database`, `init_embedded`, `mod queries`, `mod schema`) :

```rust
/// Open a persistent SQLite database at `path` and apply the schema.
/// Pure storage primitive: no config/path-resolution logic (that lives
/// bin-side in `crate::db`).
pub fn open(path: &std::path::Path) -> anyhow::Result<Database> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    schema::apply(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}
```

- [ ] **Step 2: Retirer de `storage/mod.rs` tout ce qui dépend de `core`**

Supprimer de `src/storage/mod.rs` :
- `fn resolve_storage_path(...)` (entier, il utilise `crate::core::config::data_dir`)
- `pub fn init_db(...)` (entier, il utilise `crate::core::config::load_user_config` + le garde-fou `#[cfg(test)]`)
- dans `mod tests`, les deux tests `absolute_storage_path_used_verbatim` et `relative_storage_path_anchored_under_data_dir` (ils testent `resolve_storage_path`)

Après suppression, `src/storage/mod.rs` ne doit plus contenir aucun `use crate::` ni `crate::core`. Le `use std::path::{Path, PathBuf}` : garder `Path` (utilisé par `open`), retirer `PathBuf` s'il devient inutilisé (sinon `-D warnings`).

- [ ] **Step 3: Créer `src/db.rs` avec le code remonté**

```rust
//! Bin-side database bootstrap (OH7 Lot 2b).
//!
//! Owns the config-driven path resolution that used to live in
//! `storage::init_db`. Depends on `crate::core::config`; delegates the actual
//! open+schema to the storage wrapper (`crate::storage::open`), which is
//! core-free. Kept bin-side so `armadai-storage` stays a pure rusqlite leaf.

use std::path::{Path, PathBuf};

use crate::storage::Database;

/// Resolve a possibly-relative `storage.path` to an absolute path.
///
/// Absolute paths are used verbatim. A relative path (e.g. the legacy default
/// `data/armadai.sqlite`) is anchored under the user data dir so the DB is
/// CWD-independent and `--resume`/`--replay` work from any directory (#266),
/// warning once so the user can migrate their config to an absolute path.
fn resolve_storage_path(configured: &str) -> PathBuf {
    let p = Path::new(configured);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "storage.path '{configured}' is relative and was resolved under the data dir \
             (it used to be CWD-relative, which fragments the event log per directory and \
             breaks --resume/--replay from another directory). Set an absolute storage.path \
             in config.yaml to silence this."
        );
    });
    crate::core::config::data_dir().join(p)
}

/// Initialize a persistent SQLite database at the configured path.
pub fn init_db() -> anyhow::Result<Database> {
    let config = crate::core::config::load_user_config();
    let path = resolve_storage_path(&config.storage.path);

    // Safety net (#267): no test may open the real user database.
    #[cfg(test)]
    {
        let real = crate::core::config::data_dir();
        assert!(
            !path.starts_with(&real),
            "init_db() would open the real user database at {} during a test — \
             redirect storage (ARMADAI_CONFIG_DIR -> temp config) or use init_embedded()",
            path.display()
        );
    }

    crate::storage::open(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_storage_path_used_verbatim() {
        let abs = if cfg!(windows) {
            r"C:\tmp\db.sqlite"
        } else {
            "/tmp/db.sqlite"
        };
        assert_eq!(resolve_storage_path(abs), PathBuf::from(abs));
    }

    #[test]
    fn relative_storage_path_anchored_under_data_dir() {
        let resolved = resolve_storage_path("data/armadai.sqlite");
        assert!(resolved.is_absolute());
        assert!(resolved.starts_with(crate::core::config::data_dir()));
        assert!(resolved.ends_with("data/armadai.sqlite"));
    }
}
```

- [ ] **Step 4: Déclarer `mod db` dans `main.rs`**

Dans `src/main.rs`, ajouter :

```rust
#[cfg(feature = "storage")]
mod db;
```

- [ ] **Step 5: Réécrire les imports des consommateurs de `init_db`**

```bash
grep -rl "crate::storage::init_db" src/ \
  | xargs sed -i '' 's/crate::storage::init_db/crate::db::init_db/g'
# Formes importées : `use crate::storage::{init_db, queries};`
grep -rln "use crate::storage::{init_db" src/
```

Pour chaque `use crate::storage::{init_db, queries};` restant, remplacer par deux `use` :
```rust
use crate::db::init_db;
use crate::storage::queries;
```

Vérifier :
```bash
grep -rn "storage::init_db" src/ && echo "RESTE — corriger" || echo "OK"
```
Attendu : `OK`.

- [ ] **Step 6: Invariant — storage est core-free**

```bash
grep -rn "use crate::\|crate::core" src/storage/ && echo "RESTE UN COUPLAGE" || echo "OK: storage core-free"
```
Attendu : `OK: storage core-free`. (Rappel : `init_embedded` `#[cfg(test)]` reste dans storage — il n'utilise que rusqlite/schema, pas core.)

- [ ] **Step 7: Gate CI complète** (mêmes 6 commandes qu'en Task 2a Step 6). Attendu : tout vert, nombre de tests inchangé.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(oh7): Lot 2b — move init_db/path-resolution to bin db module, storage now core-free (#252)"
```

---

## Task 2c: `[workspace]` + extraction `armadai-secrets`

Introduit le workspace Cargo et extrait la première feuille (secrets, pure). Le premier crate valide la mécanique workspace.

**Files:**
- Modify: `Cargo.toml` (racine) — ajouter `[workspace]` + dépendance `armadai-secrets`
- Create: `crates/armadai-secrets/Cargo.toml`
- Create: `crates/armadai-secrets/src/lib.rs` (= ancien `src/secrets/mod.rs`)
- Create: `crates/armadai-secrets/src/sops.rs` (= ancien `src/secrets/sops.rs`)
- Delete: `src/secrets/mod.rs`, `src/secrets/sops.rs`
- Modify: `src/main.rs` — retirer `mod secrets;`
- Modify (imports): `src/providers/factory.rs`, `src/cli/config.rs` — `crate::secrets::` → `armadai_secrets::`

**Interfaces:**
- `armadai-secrets` — Produces: `armadai_secrets::{ProviderSecrets, ProviderCredentials, load_secrets}` et `armadai_secrets::sops::{decrypt_file, init_sops}`

- [ ] **Step 1: Ajouter la section `[workspace]` à `Cargo.toml` racine**

Insérer en tête de `Cargo.toml` (avant `[package]`) :

```toml
[workspace]
members = ["crates/*"]
resolver = "3"

[workspace.package]
edition = "2024"
license = "PolyForm-Noncommercial-1.0.0"
repository = "https://github.com/Dr0drigues/swarm-festai"
```

- [ ] **Step 2: Créer le crate secrets — déplacer les fichiers**

```bash
mkdir -p crates/armadai-secrets/src
git mv src/secrets/sops.rs crates/armadai-secrets/src/sops.rs
git mv src/secrets/mod.rs crates/armadai-secrets/src/lib.rs
```

- [ ] **Step 3: Créer `crates/armadai-secrets/Cargo.toml`**

```toml
[package]
name = "armadai-secrets"
version = "1.0.0-rc.5"
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "ArmadAI — SOPS + age encrypted provider secrets loader."

[dependencies]
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_yaml_ng = "0.10"
tracing = "0.1"
```

> `src/lib.rs` (ex-mod.rs) contient déjà `pub mod sops;` en tête → devient la racine du crate sans changement. Aucun `use crate::` interne à réécrire (il n'y en a pas ; `sops.rs` utilise `use super::ProviderSecrets`, valide en crate).

- [ ] **Step 4: Ajouter la dépendance au bin**

Dans `Cargo.toml` racine, section `[dependencies]`, ajouter :

```toml
armadai-secrets = { path = "crates/armadai-secrets" }
```

- [ ] **Step 5: Retirer `mod secrets;` du bin et réécrire les imports**

Dans `src/main.rs`, supprimer la ligne `mod secrets;`.

```bash
grep -rl "crate::secrets" src/ \
  | xargs sed -i '' 's/crate::secrets/armadai_secrets/g'
grep -rn "crate::secrets\|mod secrets" src/ && echo "RESTE — corriger" || echo "OK"
```
Attendu : `OK`.

- [ ] **Step 6: Vérifier la compilation du crate seul + du bin**

```bash
cargo build -p armadai-secrets
cargo build --no-default-features --features tui,providers-api
```
Attendu : les deux compilent.

- [ ] **Step 7: Gate CI complète** (les 6 commandes de Task 2a Step 6) + build du crate feuille :

```bash
cargo build -p armadai-secrets
# puis fmt + clippy 3 modes + test 2 modes
```
Attendu : tout vert, tests inchangés.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(oh7): Lot 2c — introduce Cargo workspace + extract armadai-secrets leaf crate (#252)"
```

---

## Task 2d: Extraction `armadai-storage`

Extrait la seconde feuille (storage, désormais core-free après 2a/2b). Convertit `init_embedded` (test-only) en API publique `open_in_memory` (visible aux tests du bin cross-crate) et corrige les intra-doc-links vers core.

**Files:**
- Create: `crates/armadai-storage/Cargo.toml`
- Move: `src/storage/mod.rs` → `crates/armadai-storage/src/lib.rs`
- Move: `src/storage/schema.rs` → `crates/armadai-storage/src/schema.rs`
- Move: `src/storage/queries.rs` → `crates/armadai-storage/src/queries.rs`
- Modify: `crates/armadai-storage/src/lib.rs` — `#[cfg(test)] pub fn init_embedded` → `pub fn open_in_memory` (non-gated)
- Modify: `crates/armadai-storage/src/{queries.rs,schema.rs}` — intra-doc-links `[crate::core::…]` → texte brut
- Modify: `Cargo.toml` racine — dépendance optionnelle `armadai-storage` derrière la feature `storage`
- Modify: `src/main.rs` — retirer `#[cfg(feature="storage")] mod storage;`
- Modify (imports): `src/es_log.rs`, `src/db.rs`, tout `crate::storage::` → `armadai_storage::` ; `crate::storage::init_embedded` → `armadai_storage::open_in_memory`

**Interfaces:**
- `armadai-storage` — Produces: `armadai_storage::{Database, open, open_in_memory, queries, schema}`
- Consumed by bin : `crate::es_log::SqliteLog::new(db: armadai_storage::Database)`, `crate::db::init_db() -> armadai_storage::open(&path)`

- [ ] **Step 1: Déplacer les fichiers**

```bash
mkdir -p crates/armadai-storage/src
git mv src/storage/queries.rs crates/armadai-storage/src/queries.rs
git mv src/storage/schema.rs  crates/armadai-storage/src/schema.rs
git mv src/storage/mod.rs     crates/armadai-storage/src/lib.rs
```

- [ ] **Step 2: Convertir `init_embedded` → `open_in_memory` (API publique)**

Dans `crates/armadai-storage/src/lib.rs`, remplacer :

```rust
/// Initialize an in-memory SQLite database (for tests).
#[cfg(test)]
pub fn init_embedded() -> anyhow::Result<Database> {
    let conn = Connection::open_in_memory()?;
    schema::apply(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}
```

par (retrait du `#[cfg(test)]`, renommage — c'est une capacité publique du crate lib, donc pas de `dead_code` même si non utilisée en interne) :

```rust
/// Open an in-memory SQLite database with the schema applied. Used by tests
/// (bin-side and crate-side) that must not touch the real user database.
pub fn open_in_memory() -> anyhow::Result<Database> {
    let conn = Connection::open_in_memory()?;
    schema::apply(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}
```

> Le test interne de `queries.rs` (`use crate::storage::init_embedded;` ligne ~695) devient `use crate::open_in_memory;` (on est désormais dans le crate storage → `crate::` = racine du crate storage).

- [ ] **Step 3: Corriger les intra-doc-links vers core**

Dans `crates/armadai-storage/src/queries.rs` (~ligne 660) et `crates/armadai-storage/src/schema.rs` (~ligne 219), retirer les crochets des liens `[\`crate::core::…\`]` (qui ne résolvent plus, storage ne dépendant pas de core) → texte brut backtické :

```bash
grep -rn "crate::core" crates/armadai-storage/src/
```
Remplacer chaque `[\`crate::core::orchestration::es::…\`]` par `\`core::orchestration::es::…\`` (backticks conservés, crochets retirés). Vérifier ensuite :
```bash
grep -rn "\[\`crate::core" crates/armadai-storage/src/ && echo "RESTE UN LIEN" || echo "OK"
```

- [ ] **Step 4: Créer `crates/armadai-storage/Cargo.toml`**

```toml
[package]
name = "armadai-storage"
version = "1.0.0-rc.5"
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "ArmadAI — SQLite-backed persistence (runs + event log)."

[dependencies]
anyhow = "1"
rusqlite = { version = "0.40", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

> Reconcilier les deps avec le compilateur : lancer `cargo build -p armadai-storage` ; ajouter/retirer `serde`/`serde_json`/`chrono` selon les erreurs `unresolved import`/warnings `unused crate`. `rusqlite` + `anyhow` sont certains ; les autres selon ce que `schema.rs`/`queries.rs` référencent réellement.

- [ ] **Step 5: Câbler la dépendance optionnelle dans le bin**

Dans `Cargo.toml` racine :

```toml
# [dependencies]
armadai-storage = { path = "crates/armadai-storage", optional = true }
```
et faire tirer la dépendance par la feature `storage` :
```toml
# [features]
storage = ["dep:armadai-storage"]
```
(Retirer `rusqlite` de la liste des deps optionnelles du bin **uniquement** si plus aucun code bin ne l'utilise directement — vérifier `grep -rn "rusqlite" src/`. `src/es_log.rs` utilise `rusqlite::params!` → **il faut garder `rusqlite` comme dep optionnelle du bin OU** ré-exporter depuis `armadai-storage`. Choix : garder `rusqlite = { …, optional = true }` dans le bin et l'ajouter à la feature : `storage = ["dep:armadai-storage", "dep:rusqlite"]`, car `es_log.rs` (bin) fait du SQL direct.)

- [ ] **Step 6: Retirer `mod storage;` du bin et réécrire les imports**

Dans `src/main.rs`, supprimer `#[cfg(feature = "storage")] mod storage;`.

```bash
# init_embedded (tests bin) -> open_in_memory
grep -rl "crate::storage::init_embedded" src/ \
  | xargs sed -i '' 's/crate::storage::init_embedded/armadai_storage::open_in_memory/g'
# reste des chemins storage
grep -rl "crate::storage" src/ \
  | xargs sed -i '' 's/crate::storage/armadai_storage/g'
grep -rn "crate::storage\|mod storage" src/ && echo "RESTE — corriger" || echo "OK"
```
Attendu : `OK`. Vérifier en particulier `src/es_log.rs` (`armadai_storage::Database`) et `src/db.rs` (`armadai_storage::{Database, open}`).

- [ ] **Step 7: Vérifier compilation crate seul + bin + garde-fou cœur (anticipé)**

```bash
cargo build -p armadai-storage
cargo build --no-default-features --features tui,web,storage
cargo build --no-default-features --features tui        # storage OFF : armadai-storage non tiré
```
Attendu : les trois compilent. Le 3ᵉ prouve que `armadai-storage` (et rusqlite) ne fuit pas quand `storage` est désactivée.

- [ ] **Step 8: Gate CI complète** (6 commandes) + `cargo build -p armadai-storage` + `cargo build -p armadai-secrets`. Attendu : tout vert, nombre de tests total inchangé (les tests de `queries.rs`/`schema.rs`/`init_embedded` migrent avec le code dans le crate storage ; les tests e2e du bin qui utilisaient `init_embedded` compilent via `open_in_memory`).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(oh7): Lot 2d — extract armadai-storage leaf crate (#252)"
```

---

## Invariant de fin de Lot 2

- `Cargo.toml` racine = `[workspace]` (`members = ["crates/*"]`) **et** `[package]` (bin `armadai` + `fake-claude`), `src/` inchangé.
- `crates/armadai-secrets/` et `crates/armadai-storage/` extraits, chacun `cargo build -p …` vert **sans** dépendance interne (feuilles).
- `grep -rn "use crate::" crates/armadai-secrets/src crates/armadai-storage/src` → vide (sauf `crate::` = racine du propre crate, ex. `use crate::open_in_memory` dans le test de queries.rs — c'est intra-crate, OK).
- Le bin dépend de `armadai-secrets` (toujours) et `armadai-storage` (feature `storage`).
- Gate CI verte dans les 3 modes clippy + 2 modes test ; `cargo build --no-default-features --features tui` ne tire ni rusqlite ni armadai-storage.
- `include_dir!`/templates/embedded : inchangés (bin à la racine, `src/` non déplacé).

## Self-Review (rempli à l'écriture)

- **Couverture spec** : 2a+2b = « casser storage→core (prélude) » ✓ ; 2c = `[workspace]` + `armadai-secrets` ✓ ; 2d = `armadai-storage` (feuille pure, `open_in_memory`, doc-links) ✓. `[workspace.dependencies]` explicitement YAGNI au Lot 2 (spec) ✓.
- **Placeholders** : les manifestes de crate ont un step de réconciliation-au-compilateur explicite (2d Step 4) — c'est une vérification concrète, pas un TODO. Aucun « TBD ».
- **Cohérence des types/chemins** : `Database`/`open`/`open_in_memory`/`init_db` nommés identiquement entre tâches ; `crate::es_log::SqliteLog`, `crate::db::init_db`, `armadai_secrets::…`, `armadai_storage::…` cohérents de 2a à 2d.
- **Piège visibilité** `#[cfg(test)] init_embedded` cross-crate : résolu en 2d Step 2 (→ `pub fn open_in_memory`). **Piège dead_code** : le renommage non-gated est fait **à l'extraction** (lib crate, pas bin) pour éviter le `-D warnings` dead_code d'un `pub fn` inutilisé en bin.
- **Piège `rusqlite` bin** : `src/es_log.rs` fait du SQL direct → `rusqlite` reste dep optionnelle du bin (2d Step 5), pas seulement transitive via armadai-storage.
