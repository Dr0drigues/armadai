# OH7 Lot 4 — Extraire `armadai-providers` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extraire `src/providers/` (+ `json_runner`, `factory`, `rate_limiter`, `cli`, `api/*`, `proxy`) **et** `src/model_registry/` vers un crate lib `crates/armadai-providers/`, dépendant de `armadai-core`, avec la feature `api` gatant `reqwest`.

**Architecture:** `armadai-providers → armadai-core` uniquement (0 couplage bin résiduel : `providers` et `model_registry` n'ont aucun `use crate::` vers un module bin — vérifié). Le crate implémente le trait `armadai_core::provider::Provider`. La feature `api` (= `dep:reqwest`) gate les providers HTTP (anthropic/google) + le fetch online de `model_registry` ; le provider `cli` (tokio::process), le rate-limiter et les helpers cache-only de `model_registry` sont toujours-actifs. Extraction atomique (un déplacement partiel ne compile pas) → une PR.

**Tech Stack:** Rust edition 2024, Cargo workspace, reqwest (feature `api`), tokio (process/async), async-trait, tokio-stream.

## Global Constraints

- **Branche** : master-only. UNE PR (atomique). Squash-merge, revue indépendante + CI verte (6 checks). CI verte ≠ suffisante.
- **Gate CI** (bin = référence, features propagées) : `cargo fmt --all -- --check` ; clippy 3 modes `-D warnings` (`tui` / `tui,providers-api` / `tui,web,storage`) ; `cargo test` 2 modes (`tui` / `tui,storage`) ; **+ `cargo build -p armadai-core`** (cœur nu inchangé) **+ `cargo build -p armadai-providers`** et **`cargo build -p armadai-providers --features api`**.
- **Refactor pur** : aucun test réécrit sauf déplacement d'`use`/de fichier / rename de feature-gate. Réconciliation des tests obligatoire : les tests de providers+model_registry migrent sous `cargo test -p armadai-providers` ; le total est conservé (baseline bin actuel : 572 `tui` / 596 `tui,storage`, + core 470 + storage 22).
- **`rust-analyzer` non fiable** (stale/ABI) → **vérifier au compilateur** uniquement.
- **Code/commentaires/commits anglais.** Conventional Commits scope `oh7`. Finir le commit par `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **NE PAS `git add -A`** (untracked pré-existant). Stager explicitement.
- **`fake-claude`** : reste dans le bin, inchangé.

## Feature plumbing (le point délicat)

`providers-api` (bin) gate aujourd'hui un **mix** : (a) du code qui part dans le crate (`providers/api`, `factory`, `model_registry/fetch`) et (b) du code qui **reste** dans le bin (`starters_registry` utilise `reqwest` directement ; `linker/model_resolution`, `tui/mod`, `web/api`, `cli/link`, `cli/new` référencent l'API gated). Donc :

- **`armadai-providers`** : feature interne `api = ["dep:reqwest"]`. Dans les fichiers **déplacés**, `#[cfg(feature = "providers-api")]` → `#[cfg(feature = "api")]`.
- **bin** : `armadai-providers = { path = … }` **non-optionnel** (CliProvider/factory toujours dispo). `providers-api = ["armadai-providers/api", "dep:reqwest"]` — **garder `dep:reqwest`** pour `starters_registry`. Les `#[cfg(feature = "providers-api")]` du **bin** (linker/tui/web/cli/starters_registry) restent **inchangés** (nom de feature du bin conservé) ; comme `providers-api` forwarde `armadai-providers/api`, les items api du crate sont dispo quand le bin a `providers-api`.

## File Structure

```
Cargo.toml                    # racine : + dep armadai-providers ; providers-api re-plombée ; reqwest reste (starters_registry)
src/
  main.rs                     # `mod providers;` (+ son #[allow(dead_code)]) et `mod model_registry;` RETIRÉS
  ...                         # bin : crate::providers:: -> armadai_providers:: ; crate::model_registry:: -> armadai_providers::model_registry::
crates/
  armadai-providers/
    Cargo.toml                # dep armadai-core (path) ; feature api = dep:reqwest
    src/
      lib.rs                  # = ex-src/providers/mod.rs + `pub mod model_registry;`
      cli.rs, factory.rs, json_runner.rs, proxy.rs, rate_limiter.rs
      api/{anthropic,google,openai,mod}.rs
      model_registry/{fetch,mod}.rs
```

---

## Task 4: Extraire `armadai-providers`

**Files:**
- Create: `crates/armadai-providers/Cargo.toml`
- Move (dir): `src/providers/` → `crates/armadai-providers/src/` (`mod.rs` → `lib.rs`)
- Move (dir): `src/model_registry/` → `crates/armadai-providers/src/model_registry/`
- Modify: `crates/armadai-providers/src/lib.rs` (ajout `pub mod model_registry;`)
- Modify: crate files — `#[cfg(feature = "providers-api")]` → `#[cfg(feature = "api")]` ; `crate::providers::` → `crate::`
- Modify: `Cargo.toml` racine (dep + feature)
- Modify: `src/main.rs` (retrait des 2 `mod` + le `#[allow(dead_code)]`)
- Modify (imports bin): `crate::providers::` → `armadai_providers::` (5 fichiers) ; `crate::model_registry::` → `armadai_providers::model_registry::` (8 fichiers)

**Interfaces:**
- `armadai-providers` — Produces: `armadai_providers::{factory, cli, proxy, json_runner, rate_limiter}`, `armadai_providers::api::{…}` (gated `api`), `armadai_providers::model_registry::{ModelEntry, ModelCost, ModelLimits, fetch::…}`. Implémente `armadai_core::provider::Provider`.

- [ ] **Step 1: Branche**

```bash
cd "$(git rev-parse --show-toplevel)"
git checkout master && git pull --ff-only
git checkout -b feat/oh7-4-extract-providers
```

- [ ] **Step 2: Déplacer providers + model_registry dans le crate**

```bash
mkdir -p crates/armadai-providers
git mv src/providers crates/armadai-providers/src
git mv crates/armadai-providers/src/mod.rs crates/armadai-providers/src/lib.rs
git mv src/model_registry crates/armadai-providers/src/model_registry
```

- [ ] **Step 3: Déclarer `model_registry` dans le lib.rs du crate**

Dans `crates/armadai-providers/src/lib.rs`, ajouter (à côté des autres `pub mod`) :
```rust
pub mod model_registry;
```

- [ ] **Step 4: Renommer le feature-gate DANS le crate uniquement**

```bash
grep -rl 'feature = "providers-api"' crates/armadai-providers/src/ \
  | xargs sed -i '' 's/feature = "providers-api"/feature = "api"/g'
# Vérifier : plus aucun providers-api dans le crate
grep -rn 'providers-api' crates/armadai-providers/src/ && echo "!! RESTE" || echo "OK: crate gate = api"
```

- [ ] **Step 5: Réécrire les auto-références internes du crate (`crate::providers::` → `crate::`)**

```bash
grep -rl "crate::providers::" crates/armadai-providers/src/ \
  | xargs sed -i '' 's/crate::providers::/crate::/g'
# model_registry auto-ref (crate::model_registry::) reste valide (sous-module du crate) — ne pas toucher.
grep -rn "crate::providers" crates/armadai-providers/src/ && echo "!! RESTE crate::providers" || echo "OK"
grep -rn "crate::" crates/armadai-providers/src/ | grep -vE "crate::model_registry|crate::(cli|factory|proxy|json_runner|rate_limiter|api)\b" | grep -v "armadai_core" || echo "(refs crate:: internes = sous-modules du crate, OK)"
```
Attendu : les seuls `crate::` restants pointent vers des sous-modules du crate (`crate::model_registry`, `crate::factory`, `crate::api`, …) ou sont `armadai_core::`. Aucun `crate::providers`.

- [ ] **Step 6: Créer `crates/armadai-providers/Cargo.toml`**

```toml
[package]
name = "armadai-providers"
version = "1.0.0-rc.5"
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "ArmadAI — LLM provider implementations (API + CLI) and model registry."

[features]
api = ["dep:reqwest"]

[dependencies]
armadai-core = { path = "../armadai-core" }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["process", "io-util", "time", "macros", "rt", "rt-multi-thread", "sync"] }
tokio-stream = { version = "0.1", features = ["io-util"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls-native-roots"], optional = true }
```

> **Réconcilier au compilateur** : `cargo build -p armadai-providers` (sans api) puis `cargo build -p armadai-providers --features api`, puis `cargo test -p armadai-providers --features api`. Ajuster deps/features tokio selon les `unresolved import`/`unused`. `reqwest` DOIT rester derrière `api` (0 reqwest quand api off).

- [ ] **Step 7: Câbler le bin (dep + features) + retirer les `mod`**

Dans `Cargo.toml` racine, `[dependencies]` :
```toml
armadai-providers = { path = "crates/armadai-providers" }
```
`[features]` — re-plomber `providers-api` (garder `dep:reqwest` pour starters_registry) :
```toml
providers-api = ["dep:reqwest", "armadai-providers/api"]
```
(`reqwest` reste dans `[dependencies]` du bin, `optional = true`.)

Dans `src/main.rs`, retirer les lignes `mod model_registry;` **et** `#[allow(dead_code)]` + `mod providers;`.

- [ ] **Step 8: Réécrire les imports du bin**

```bash
# model_registry AVANT providers (sinon 'crate::providers' matcherait pas model_registry, ordre sans risque ici mais on garde net)
grep -rl "crate::model_registry" src/ | xargs sed -i '' 's/crate::model_registry/armadai_providers::model_registry/g'
grep -rl "crate::providers" src/ | xargs sed -i '' 's/crate::providers/armadai_providers/g'
# Vérifs
grep -rn "crate::providers\|crate::model_registry\|mod providers;\|mod model_registry;" src/ && echo "!! RESTE" || echo "OK: bin repointé"
```
Attendu : `OK`.

- [ ] **Step 9: Traiter le dead_code libéré par le retrait du `#[allow(dead_code)] mod providers;`**

Le bin masquait le dead_code de `providers` via un `#[allow(dead_code)]` blanket. En crate lib, les items `pub` ne sont plus dead_code. Si `cargo clippy -p armadai-providers -- -D warnings` (ou le build) signale un dead_code résiduel (item `pub(crate)`/privé réellement inutilisé), le traiter **fidèlement** : soit `pub` s'il est utilisé cross-crate par le bin, soit `#[allow(dead_code)]` **scopé** sur l'item précis (comme `ProviderMetadata` au Lot 1d), jamais un blanket. Documenter chaque cas dans le rapport.

- [ ] **Step 10: Builds ciblés (feature off/on) + garde-fou**

```bash
cargo build -p armadai-providers                 # api OFF : 0 reqwest
cargo tree -p armadai-providers | grep reqwest && echo "!! reqwest sans api" || echo "OK: pas de reqwest sans api"
cargo build -p armadai-providers --features api   # api ON
cargo build -p armadai-core                       # cœur nu inchangé
cargo build --no-default-features --features tui   # bin sans providers-api : compile, pas de reqwest tiré par armadai-providers
```

- [ ] **Step 11: Gate CI complète**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | grep "test result:"
cargo test --no-default-features --features tui,storage 2>&1 | grep "test result:"
cargo test -p armadai-providers --features api 2>&1 | grep "test result:"
```

- [ ] **Step 12: Réconcilier les comptes de tests**

Relever les comptes bin (`tui`, `tui,storage`) + `cargo test -p armadai-providers --features api` + rappels core (470) / storage (22). Vérifier que `(bin_tui après + providers_crate)` conserve le total d'avant Lot 4 (bin_tui avant = 572 ; le bin doit chuter du nombre de tests migrés vers le crate providers, qui réapparaissent sous `-p armadai-providers`). AUCUN test perdu. Documenter l'arithmétique exacte.

- [ ] **Step 13: Commit**

```bash
git add crates/armadai-providers Cargo.toml Cargo.lock src/
git status --short | grep -v '^??'
git commit -m "$(cat <<'MSG'
refactor(oh7): Lot 4 — extract armadai-providers leaf crate (#252)

Move src/providers/* + src/model_registry/* into crates/armadai-providers/,
depending on armadai-core. Pure extraction (0 residual bin coupling):
- lib.rs = old providers/mod.rs + pub mod model_registry
- feature providers-api -> api (dep:reqwest) inside the crate; bin forwards
  providers-api = ["armadai-providers/api", "dep:reqwest"] (reqwest kept for
  starters_registry which stays bin-side)
- internal crate::providers:: -> crate:: ; bin crate::{providers,model_registry}::
  -> armadai_providers::...

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

## Invariant de fin de Lot 4

- `crates/armadai-providers/` extrait, dépend de `armadai-core` seul (+ externes). `cargo build -p armadai-providers` vert ; `cargo tree -p armadai-providers` **sans reqwest** quand `api` off, **avec** quand `api` on.
- `grep -rn "crate::providers\|crate::model_registry\|mod providers;\|mod model_registry;" src/` → vide.
- `grep -rn "crate::providers\|providers-api" crates/armadai-providers/src/` → vide (gate = `api`, auto-refs = `crate::`).
- `cargo build -p armadai-core` inchangé (cœur nu, aucune régression) ; `cargo build --no-default-features --features tui` ne tire pas reqwest via armadai-providers.
- Gate verte 3 modes clippy + 2 modes test ; réconciliation exacte (aucun test perdu).

## Hors périmètre

- Bin final / interfaces (cli/tui/web/shell/linker/audit/registres/theme restent dans le bin) — c'est déjà l'état ; le Lot 5 ne fait que re-plomber les features restantes et ajouter le `cargo build -p armadai-core` nu au YAML CI.
- Façade `pub use` de `armadai-providers` — YAGNI (API = `armadai_providers::<module>::<Item>`).

## Self-Review (rempli à l'écriture)

- **Couverture spec** : providers+json_runner+factory+rate_limiter+model_registry → crate ✓ ; dépend de core ✓ ; feature `api`=reqwest ✓.
- **Feature plumbing** : le mix `providers-api` (crate vs bin, starters_registry garde reqwest) explicité en Section dédiée + Steps 4/7 ; rename gate **crate-only** ✓.
- **Placeholders** : manifeste avec reconcile-au-compilateur (Step 6). Pas de « TBD ».
- **Cohérence chemins** : bidirectionnel (interne `crate::providers::`→`crate::` ; bin →`armadai_providers::`) ; `crate::model_registry` interne reste (sous-module).
- **Piège dead_code** : retrait du `#[allow(dead_code)] mod providers;` traité explicitement (Step 9, allow scopé ou pub, jamais blanket).
- **Piège atomicité** : un déplacement partiel ne compile pas → une PR/un commit.
